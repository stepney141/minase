#!/usr/bin/env python3
"""中断可能な生成機を1台ずつ借り、教師の回収、受け入れ、および移行を行う。

使い方: python3 scripts/vast/teacher_generation.py run --help
全引数を明示する。同じrun-dirで再開するときも、保存した全引数と一致させる。
run.jsonに条件、state.jsonに借用履歴、events.jsonlとprogress.jsonlに観測を残す。
受け入れた本とログはgen-<N>/{teacher,logs}/に置き、generation_chain.pyへ渡せる。
生成機が止まったら削除を確認し、config.jsonと組のそろった塊だけを次の機械へ送る。
正常終了、例外、SIGTERM、SIGINTのいずれでも、最後の回収と削除の確認を行う。
"""

import argparse
from datetime import datetime, timezone
from decimal import Decimal, ROUND_CEILING
import fcntl
import ipaddress
import json
import math
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from typing import Any

import common

REMOTE = '/workspace/job'
IMAGE = 'rust:1.98.0-bookworm'
DISK_GB = 30
IDLE_SECONDS = 1800
COLLECT_SECONDS = 60
NETWORK = 'crates/minase/nets/nnue.bin'
# docs/measurements/interruptible-teacher-generation/interruptible-teacher-generation-candidates.md
NPS_PER_THREAD = {'7B13': 186_200, '9654': 214_300}
PROGRESS = re.compile(r'^progress: elapsed_seconds=\S+ finished_games=(\d+) '
                      r'committed_games=(\d+) chunks=(\d+)/(\d+)\s*$', re.M)
STATUS_FIELDS = ('actual_status', 'intended_status', 'cur_state', 'next_state', 'status_msg')


class GenerationStop(RuntimeError):
    """安全に生成を続けられない。"""


class RemoteError(GenerationStop):
    """SSHまたはrsyncの実行に失敗した。"""


class Interrupted(BaseException):
    """シグナルを受け、最後の回収と削除に進む。"""

    def __init__(self, signum: int) -> None:
        self.signum = signum


def atomic_json(path: Path, value: Any) -> None:
    """完成したJSONだけを改名で公開する。"""
    temporary = path.with_name(path.name + '.tmp')
    with temporary.open('w') as stream:
        json.dump(value, stream, ensure_ascii=False, allow_nan=False)
        stream.write('\n')
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def onstart(deadline: int) -> str:
    """自己停止を予約し、待機用bashのPIDをexec後の作業プロセスへ引き継ぐ。"""
    script = common.onstart_prefix(deadline) + (
        "nohup setsid bash -c 'set -euo pipefail; echo $$ > /workspace/job/worker.pid; "
        "until [[ -f /workspace/job/started ]]; do sleep 2; done; "
        "exec python3 /workspace/job/generation_worker.py run' "
        '>> /workspace/job/logs/worker.log 2>&1 < /dev/null &\n')
    if len(script) > 4048:
        raise GenerationStop(f'起動時スクリプトが4048文字を超えています: {len(script)}')
    return script


def bid_price(minimum: int | float) -> float:
    """最低入札額を0.0001米ドル単位で切り上げる。"""
    return float(Decimal(str(minimum)).quantize(Decimal('0.0001'), rounding=ROUND_CEILING))


def select_offer(offers: list[dict], args: argparse.Namespace, remaining: float,
                 excluded: list[int]) -> dict:
    """フェーズ0で決めた条件を満たす出品から、探索速度あたりの単価が最小のものを選ぶ。"""
    candidates = []
    for offer in offers:
        model = re.search(r'\bEPYC\s+(7B13|9654)\b', offer['cpu_name'])
        cores = offer['cpu_cores']
        price = offer['min_bid' if args.rental == 'interruptible' else 'dph_total']
        numbers = (cores, offer['cpu_ram'], offer['direct_port_count'], offer['duration'], price,
                   offer['cpu_cores_effective'], offer['gpu_frac'], offer['dph_total'])
        if any(type(value) not in (int, float) or not math.isfinite(value) for value in numbers) or price < 0:
            continue
        if args.rental == 'interruptible':
            price = bid_price(price)
        if (model is None or offer['cpu_arch'] != 'amd64' or offer['gpu_frac'] != 1
                or offer['cpu_cores_effective'] != cores or cores < 256 or int(cores) != cores
                or offer['cpu_ram'] < cores * 220 * 1.25 or offer['direct_port_count'] < 1
                or offer['rentable'] is not True or offer['duration'] < remaining
                or not 0 <= price <= args.max_price or offer['dph_total'] < 0
                or offer['machine_id'] in excluded):
            continue
        candidates.append((price / (cores * NPS_PER_THREAD[model[1]]), offer))
    if not candidates:
        raise GenerationStop('条件を満たす出品がありません')
    return min(candidates, key=lambda item: item[0])[1]


def chunk_pairs(directory: Path) -> list[tuple[int, Path, Path]]:
    """最終名のMNSDと集計JSONがそろった塊を番号順に返す。"""
    result = []
    for path in directory.glob('chunk-*.mnsd'):
        match = re.fullmatch(r'chunk-(\d{5,})\.mnsd', path.name)
        summary = path.with_suffix('.json')
        if match is not None and summary.is_file():
            result.append((int(match[1]), path, summary))
    return sorted(result)


def chunk_games(pairs: list[tuple[int, Path, Path]], games: int, size: int) -> int:
    """保存局数ではなく、破棄した局も含む塊の担当局数を数える。"""
    return sum(min((number + 1) * size, games) - number * size for number, _, _ in pairs)


class TeacherGeneration:
    """保存した借用履歴と回収済みの成果物を使い、1台ずつ生成する。"""

    def __init__(self, args: argparse.Namespace, clock: Any) -> None:
        self.args = args
        self.clock = clock
        self.root = args.run_dir
        self.deadline = datetime.strptime(args.deadline, '%Y-%m-%dT%H:%M:%SZ').replace(
            tzinfo=timezone.utc).timestamp()
        self.prefix = f'minase-teacher-{self.root.name}-'
        self.state = None
        self.checksum = None
        self.accepted: set[int] = set()
        self.orphans: list[dict] = []

    @property
    def current(self) -> dict | None:
        if self.state is None or self.state['current'] is None:
            return None
        return next(record for record in self.state['rentals'] if record['number'] == self.state['current'])

    def save(self) -> None:
        atomic_json(self.root / 'state.json', self.state)

    def record(self, filename: str, **values: Any) -> None:
        with (self.root / filename).open('a') as stream:
            stamp = datetime.fromtimestamp(self.clock.time(), timezone.utc).isoformat()
            stream.write(json.dumps({'time': stamp, **values}, ensure_ascii=False, allow_nan=False) + '\n')

    def event(self, event: str, **values: Any) -> None:
        self.record('events.jsonl', event=event, **values)

    def remaining(self) -> float:
        return self.deadline - self.clock.time()

    def initialize(self) -> None:
        values = {key: str(value) if isinstance(value, Path) else value for key, value in vars(self.args).items()}
        path = self.root / 'run.json'
        if path.is_file():
            saved = json.loads(path.read_text())
            different = [key for key in saved.keys() | values.keys()
                         if key not in saved or key not in values or saved[key] != values[key]]
            if different:
                raise GenerationStop('保存した引数と一致しません: ' + ', '.join(sorted(different)))
        else:
            atomic_json(path, values)
        state_path = self.root / 'state.json'
        self.state = json.loads(state_path.read_text()) if state_path.is_file() else {
            'rental_count': 0, 'rentals': [], 'excluded_machine_ids': [], 'current': None}
        self.save()
        for name in ('work', 'teacher', 'logs'):
            (self.root / 'remote' / name).mkdir(parents=True, exist_ok=True)
        for name in ('teacher', 'logs'):
            (self.root / f'gen-{self.args.generation}' / name).mkdir(parents=True, exist_ok=True)
        self.accepted = {number for number in self.numbers()
                         if (self.root / f'gen-{self.args.generation}/teacher/book-{number}.mnsd').is_file()}

    def numbers(self) -> range:
        return range(self.args.first_book, self.args.last_book + 1)

    def cost(self) -> float:
        total = 0.0
        for record in self.state['rentals']:
            if record['instance_id'] is None and record['deleted_at'] is not None:
                continue
            if record['dph_total'] is None:
                raise GenerationStop(f'生成機 {record["instance_id"]} の時間単価が未確認です')
            end = self.clock.time() if record['deleted_at'] is None else record['deleted_at']
            total += (end - record['created_at']) / 3600 * record['dph_total']
            total += record['sent_bytes'] / 1e9 * record['inet_down_cost']
            total += record['received_bytes'] / 1e9 * record['inet_up_cost']
        return total

    def limits(self, offer: dict | None) -> None:
        if offer is not None:
            if self.state['rental_count'] >= self.args.max_rentals:
                raise GenerationStop('借りる回数の上限に達しました')
            if self.remaining() < 1500:
                raise GenerationStop('絶対期限まで25分未満なので借りられません')
            if self.cost() + offer['dph_total'] > self.args.max_cost:
                raise GenerationStop('新しい機械の1時間分を加えると累計費用の上限を超えます')
        else:
            if self.remaining() < 600:
                raise GenerationStop('絶対期限まで10分未満なので終了します')
            if self.cost() + self.current['dph_total'] * 0.5 >= self.args.max_cost:
                raise GenerationStop('削除用の30分分を含む累計費用が上限に達しました')

    def instances(self) -> list[dict]:
        value = json.loads(common.call(60, 'show', 'instances', '--raw'))
        if not isinstance(value, list) or any(not isinstance(item, dict) for item in value):
            raise common.InvalidResponse('インスタンスの一覧を読めません')
        return value

    def attach(self, instance: dict) -> None:
        record = self.current
        record['instance_id'] = instance['id']
        record['instance'] = instance
        # IDは単価の解釈より先に保存し、応答が不完全でも削除対象を失わない。
        self.save()
        for key in ('dph_total', 'inet_up_cost', 'inet_down_cost'):
            value = instance[key]
            if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
                raise GenerationStop(f'生成機 {instance["id"]} の{key}が不正です')
            record[key] = value
        self.save()

    def recover(self) -> None:
        """前回の削除未確認機と、作成後に記録できなかった同じ実行の機械を削除する。"""
        if self.current is not None:
            record = self.current
            if record['instance_id'] is None:
                matches = [item for item in self.instances() if item['label'] == record['label']]
                if len(matches) > 1:
                    raise GenerationStop(f'同じラベルの生成機が複数あります: {record["label"]}')
                if matches:
                    self.attach(matches[0])
                else:
                    record['deleted_at'] = self.clock.time()
                    self.state['current'] = None
                    self.save()
            if self.current is not None:
                self.delete('前回の実行で削除が未確認')
        known = {record['instance_id'] for record in self.state['rentals']}
        self.orphans = [item for item in self.instances() if isinstance(item['label'], str)
                        and item['label'].startswith(self.prefix) and item['id'] not in known]
        if self.orphans:
            # 記録のない課金を0とせず、以後の再実行でも費用上限を失わない。
            previous = self.state['unaccounted_instances'] if 'unaccounted_instances' in self.state else []
            self.state['unaccounted_instances'] = sorted(set(previous) | {item['id'] for item in self.orphans})
            self.save()
        if not self.delete_orphans():
            raise GenerationStop('記録にない生成機の削除を確認できません')
        if 'unaccounted_instances' in self.state:
            raise GenerationStop(f'記録にない生成機を削除しましたが、累計費用が不明です: '
                                 f'{self.state["unaccounted_instances"]}')

    def delete_orphans(self) -> bool:
        """記録外の機械は、1台の失敗や中断があっても残りを終了時に削除する。"""
        for item in list(self.orphans):
            try:
                common.destroy(item['id'])
            except Exception as error:
                print(f'記録にない生成機 {item["id"]} の削除を確認できません: {error}', file=sys.stderr)
            else:
                self.orphans.remove(item)
                self.event('deleted_orphan', instance_id=item['id'], instance=item)
        return not self.orphans

    def delete(self, reason: str) -> None:
        record = self.current
        if record is None:
            return
        record['reason'] = reason
        try:
            self.save()
        except OSError as error:
            print(f'削除理由の保存に失敗しました: {error}', file=sys.stderr)
        if record['instance_id'] is None:
            matches = [item for item in self.instances() if item['label'] == record['label']]
            if len(matches) > 1:
                raise GenerationStop(f'同じラベルの生成機が複数あります: {record["label"]}')
            if matches:
                self.attach(matches[0])
        if record['instance_id'] is not None:
            common.destroy(record['instance_id'])
        record['deleted_at'] = self.clock.time()
        self.state['current'] = None
        self.save()
        cpu_fields = {}
        if 'cpu_info' in record:
            cpu_fields = {'cpu_cores': record['offer']['cpu_cores'], 'cpu_info': record['cpu_info']}
        self.event('deleted', rental=record['number'], instance_id=record['instance_id'], reason=reason,
                   **cpu_fields)

    def clean_work(self, record: dict) -> None:
        """片側だけの塊を除き、旧機の作業ログを借用ごとに退避する。"""
        work = self.root / 'remote/work'
        archive = self.root / f'remote/archive/rental-{record["number"]}'
        for number in self.numbers():
            directory = work / f'book-{number}'
            for path in directory.glob('chunk-*'):
                if re.fullmatch(r'chunk-\d{5,}\.(mnsd|json)', path.name):
                    other = path.with_suffix('.json' if path.suffix == '.mnsd' else '.mnsd')
                    if not other.is_file():
                        path.unlink()
            log = work / f'book-{number}.log'
            if log.is_file():
                archive.mkdir(parents=True, exist_ok=True)
                log.replace(archive / log.name)
        # 公開の片側だけが届いた場合も、次の機械の結果とは組にしない。
        for name in ('teacher', 'logs'):
            directory = self.root / 'remote' / name
            for path in directory.iterdir():
                if path.is_file():
                    archive.mkdir(parents=True, exist_ok=True)
                    path.replace(archive / (name + '-' + path.name))

    def rent(self, rejected_offers: set[int]) -> bool:
        rental_type = 'bid' if self.args.rental == 'interruptible' else 'on-demand'
        offers = json.loads(common.call(60, 'search', 'offers',
                                       'cpu_arch=amd64 gpu_frac=1 cpu_cores>=256 direct_port_count>=1 '
                                       'rentable=true verified=any',
                                       '--type', rental_type, '--raw', '--limit', '1000'))
        offer = select_offer([item for item in offers if item['id'] not in rejected_offers], self.args,
                             self.remaining(), self.state['excluded_machine_ids'])
        self.limits(offer)
        number = self.state['rental_count'] + 1
        label = self.prefix + str(number)
        if any(item['label'] == label for item in self.instances()):
            raise GenerationStop(f'同じラベルの生成機があります: {label}')
        start = self.root / 'onstart.sh'
        start.write_text(onstart(int(self.deadline)))
        self.limits(offer)
        bid = bid_price(offer['min_bid']) if self.args.rental == 'interruptible' else None
        record = {'number': number, 'label': label, 'offer': offer, 'rental': self.args.rental,
                  'instance_id': None, 'instance': None, 'created_at': self.clock.time(), 'deleted_at': None,
                  'bid_price': bid, 'dph_total': None, 'inet_up_cost': None, 'inet_down_cost': None,
                  'sent_bytes': 0, 'received_bytes': 0, 'sent_chunks': {}, 'sent_chunk_games': {},
                  'address': None, 'ssh_at': None, 'last_collection': None,
                  'cpu_seconds': None, 'cpu_increased_at': None, 'nonrunning': 0,
                  'build_at': None, 'generation_at': None, 'progress': {}, 'reason': None}
        self.state['rentals'].append(record)
        self.state['current'] = number
        self.state['rental_count'] = number
        self.save()
        try:
            instance_id = common.create(str(offer['id']), label, start, IMAGE, DISK_GB, bid_price=bid)
        except (common.AuthenticationExpired, common.RateLimitExceeded):
            raise
        except Exception as error:
            matches = [item for item in self.instances() if item['label'] == label]
            if len(matches) > 1:
                raise GenerationStop(f'同じラベルの生成機が複数あります: {label}') from error
            if not matches:
                record['reason'] = f'作成に失敗し、同じラベルの生成機が存在しないことを確認: {error}'
                record['deleted_at'] = self.clock.time()
                self.state['current'] = None
                rejected_offers.add(offer['id'])
                self.save()
                self.event('creation_failed', rental=number, offer_id=offer['id'], reason=record['reason'])
                return False
            instance = matches[0]
        else:
            record['instance_id'] = instance_id
            self.save()
            instance = common.instance(instance_id, 60)
            if instance is None:
                raise GenerationStop(f'作成した生成機 {instance_id} の照会結果がありません')
        self.attach(instance)
        self.event('created', rental=number, instance_id=record['instance_id'])
        return True

    def ssh(self, command: str, seconds: float) -> str:
        if self.current['address'] is None:
            raise GenerationStop('生成機のSSH接続先がありません')
        host, port = self.current['address']
        try:
            return common.run_timed(['ssh', *common.ssh_options(port, '-p'), f'root@{host}', command],
                                    seconds, stdout=subprocess.PIPE).stdout
        except (OSError, subprocess.SubprocessError) as error:
            raise RemoteError(f'SSHの実行に失敗しました: {error}') from error

    def connect(self) -> bool:
        record = self.current
        self.limits(None)
        end = record['created_at'] + 600
        instance = record['instance']
        while instance is None or instance['actual_status'] != 'running':
            self.limits(None)
            if self.clock.time() >= end:
                return False
            self.clock.sleep(min(5, end - self.clock.time()))
            if self.clock.time() >= end:
                return False
            instance = common.instance(record['instance_id'], min(60, end - self.clock.time()))
        record['instance'] = instance
        end = self.clock.time() + 300
        while self.clock.time() < end:
            self.limits(None)
            if instance is not None and instance['public_ipaddr'] and instance['ports'] and '22/tcp' in instance['ports']:
                ports = instance['ports']['22/tcp']
                if ports:
                    host = str(ipaddress.ip_address(instance['public_ipaddr']))
                    port = int(ports[0]['HostPort'])
                    if not 1 <= port <= 65535:
                        raise GenerationStop('SSHポートが不正です')
                    record['address'] = [host, port]
                    self.save()
                    try:
                        self.ssh('true', min(20, end - self.clock.time()))
                        record['ssh_at'] = self.clock.time()
                        self.save()
                        return True
                    except RemoteError:
                        pass
            self.clock.sleep(min(5, max(0, end - self.clock.time())))
            if self.clock.time() < end:
                instance = common.instance(record['instance_id'], min(60, end - self.clock.time()))
        return False

    def transfer(self, sources: list[str], destination: str, size: int, receiving: bool,
                 end: float) -> None:
        seconds = min(common.transfer_seconds(size, self.remaining()), end - self.clock.time())
        if seconds <= 0:
            raise GenerationStop('転送の上限時間に達しました')
        host, port = self.current['address']
        ssh = shlex.join(['ssh', *common.ssh_options(port, '-p')])
        command = ['rsync', '-a', '--stats', '-e', ssh]
        if receiving:
            for pattern in ('*.tmp', '.selfplay_generate.lock', 'teacher.mnsd'):
                command.extend(['--exclude', pattern])
        command.extend(['--', *sources, destination])
        failure = None
        try:
            output = common.run_timed(command, seconds, stdout=subprocess.PIPE,
                                      env={**os.environ, 'LC_ALL': 'C'}).stdout
        except subprocess.CalledProcessError as error:
            output, failure = error.stdout, error
        except (OSError, subprocess.SubprocessError) as error:
            raise RemoteError(f'rsyncの実行に失敗しました: {error}') from error
        field = 'received' if receiving else 'sent'
        match = None if output is None else re.search(r'^Total bytes ' + field + r': ([\d,]+)\s*$', output, re.M)
        if match is not None:
            self.current[field + '_bytes'] += int(match[1].replace(',', ''))
            self.save()
        if failure is not None and not (receiving and failure.returncode == 24):
            raise RemoteError(f'rsyncの実行に失敗しました: {failure}') from failure
        if match is None:
            raise GenerationStop(f'rsyncの転送量を読めません: {field}')

    def remote_path(self, path: str) -> str:
        host = self.current['address'][0]
        if ':' in host:
            host = f'[{host}]'
        return f'root@{host}:{path}'

    def upload(self, paths: list[Path], destination: str) -> None:
        self.limits(None)
        size = sum(path.stat().st_size for path in paths)
        end = self.clock.time() + common.transfer_seconds(size, self.remaining())
        self.transfer(list(map(str, paths)), self.remote_path(destination), size, False, end)

    def prepare(self) -> bool:
        generation = self.args.generation
        self.ssh('mkdir -p ' + ' '.join(f'{REMOTE}/{name}/gen-{generation}' for name in ('work', 'teacher', 'logs'))
                 + f' {REMOTE}/bundles', 60)
        with tempfile.TemporaryDirectory(dir=self.root) as temporary:
            directory = Path(temporary)
            config = directory / 'worker.json'
            atomic_json(config, {'idle_limit_seconds': IDLE_SECONDS})
            self.upload([Path(__file__).with_name('generation_worker.py'),
                         Path(__file__).with_name('teacher_generation_probe.py'), config], REMOTE + '/')
            response = self.ssh(f'python3 {REMOTE}/generation_worker.py cpu-info', 60)
            try:
                info = json.loads(response)
                cores, limit = info['nproc'], info['cpu_limit']
            except (ValueError, KeyError, TypeError) as error:
                raise common.InvalidResponse('cpu-infoの応答を読めません') from error
            self.current['cpu_info'] = info
            self.save()
            expected = self.current['offer']['cpu_cores']
            if (type(cores) is not int or cores != expected or
                    (limit is not None and (type(limit) not in (int, float) or not math.isfinite(limit)
                                            or limit < expected * 0.9))):
                machine = self.current['offer']['machine_id']
                self.delete('CPUの割り当てが出品の条件を満たさない')
                self.state['excluded_machine_ids'].append(machine)
                self.save()
                return False
            self.current['concurrency'] = cores
            self.save()
            bundle = directory / 'source.bundle'
            common.run_timed(['git', '-C', str(self.args.repo), 'bundle', 'create', str(bundle), self.args.ref], 600)
            self.upload([bundle], REMOTE + '/bundles/source.bundle')
            for number in self.numbers():
                if number in self.accepted:
                    continue
                source = self.root / f'remote/work/book-{number}'
                pairs = chunk_pairs(source)
                config = source / 'config.json'
                if not config.is_file():
                    if pairs:
                        raise GenerationStop(f'本{number}の塊にconfig.jsonがありません')
                    continue
                destination = f'{REMOTE}/work/gen-{generation}/book-{number}/'
                self.ssh('mkdir -p ' + destination, 60)
                self.upload([config, *[path for _, mnsd, summary in pairs for path in (mnsd, summary)]], destination)
                self.current['sent_chunks'][str(number)] = len(pairs)
                self.current['sent_chunk_games'][str(number)] = chunk_games(pairs, self.args.games, self.args.chunk_games)
                self.save()
            job = {key: getattr(self.args, key) for key in
                   ('generation', 'commit', 'games', 'nodes', 'random_moves', 'max_ply', 'hash_mb', 'chunk_games')}
            job.update(bundle=REMOTE + '/bundles/source.bundle', reference=self.args.ref,
                       generation_seed=self.args.seed_base, book_step=self.args.seed_book_step,
                       target_books=self.args.last_book + 1,
                       skip_books=sorted(set(range(self.args.first_book)) | self.accepted), concurrency=cores)
            path = directory / 'job.json'
            atomic_json(path, job)
            self.upload([path], REMOTE + '/job.json')
        self.ssh(f'touch {REMOTE}/started', 60)
        return True

    def observe(self) -> list[dict]:
        record = self.current
        rows = []
        for number in self.numbers():
            if number in self.accepted:
                continue
            work = self.root / f'remote/work/book-{number}'
            log = work.with_suffix('.log')
            published = self.root / f'remote/logs/book-{number}.log'
            text = log.read_text() if log.is_file() else ''
            if published.is_file():
                text += '\n' + published.read_text()
            if record['build_at'] is None and re.search(r'^\s*Finished\b', text, re.M):
                record['build_at'] = self.clock.time()
                self.event('build_completed', rental=record['number'])
            matches = list(PROGRESS.finditer(text))
            values = {'finished_games': None, 'committed_games': None, 'chunks': None}
            if matches:
                last = matches[-1]
                values = {'finished_games': int(last[1]), 'committed_games': int(last[2]),
                          'chunks': [int(last[3]), int(last[4])]}
                if record['generation_at'] is None:
                    record['generation_at'] = self.clock.time()
                    self.event('generation_resumed', rental=record['number'])
            row = {'rental': record['number'], 'book': number, **values,
                   'collected_games': chunk_games(chunk_pairs(work), self.args.games, self.args.chunk_games)}
            rows.append(row)
            record['progress'][str(number)] = row
        return rows

    def accept(self) -> None:
        for number in self.numbers():
            if number in self.accepted:
                continue
            path = self.root / f'remote/teacher/book-{number}.mnsd'
            log = self.root / f'remote/logs/book-{number}.log'
            if not path.is_file() or not log.is_file():
                continue
            if self.checksum is None:
                raise GenerationStop('受け入れに必要な重みのチェックサムがありません')
            expected = {'generation_commit': self.args.commit, 'network_checksum': self.checksum,
                        'rule_set': 'L0,P0,R1,E0', 'teacher_search_budget': self.args.nodes,
                        'seed': self.args.seed_base + self.args.seed_book_step * number}
            def inspect(command: list[str], **kwargs: Any) -> subprocess.CompletedProcess:
                return common.run_timed(command, 600, **kwargs)
            _, entries = common.inspect_teacher(self.args.minase_bin, path, log, expected, vars(self.args), inspect)
            directory = self.root / f'gen-{self.args.generation}'
            log_target = directory / 'logs' / log.name
            if log_target.exists():
                log_target.unlink()
            os.link(log, log_target)
            os.link(path, directory / 'teacher' / path.name)
            self.accepted.add(number)
            work = self.root / f'remote/work/book-{number}'
            if work.is_dir():
                shutil.rmtree(work)
            work.with_suffix('.log').unlink(missing_ok=True)
            self.event('accepted', book=number, recorded_positions=entries)

    def collect(self) -> tuple[dict | None, bool]:
        data = None
        success = False
        try:
            data = json.loads(self.ssh(f'python3 {REMOTE}/teacher_generation_probe.py {self.args.generation}', 60))
            if (type(data['pid']) is not int or data['pid'] <= 0 or type(data['alive']) is not bool
                    or type(data['bytes']) is not int or data['bytes'] < 0
                    or (data['exit_status'] is not None and not isinstance(data['exit_status'], str))
                    or (data['cpu_seconds'] is not None and
                        (type(data['cpu_seconds']) not in (int, float) or not math.isfinite(data['cpu_seconds'])
                         or data['cpu_seconds'] < 0))):
                raise ValueError('生成機の観測値が不正です')
        except (RemoteError, OSError, subprocess.SubprocessError, ValueError, KeyError, TypeError) as error:
            print(f'生成機の観測に失敗しました: {error}', file=sys.stderr)
            data = None
        if data is not None:
            try:
                end = self.clock.time() + common.transfer_seconds(data['bytes'], self.remaining())
                for name in ('work', 'teacher', 'logs'):
                    self.transfer([self.remote_path(f'{REMOTE}/{name}/gen-{self.args.generation}/')],
                                  str(self.root / 'remote' / name) + '/', data['bytes'], True, end)
                success = True
            except (OSError, subprocess.SubprocessError, common.VastError, GenerationStop) as error:
                print(f'生成機からの回収に失敗しました: {error}', file=sys.stderr)
        rows = self.observe() if success else []
        self.accept()
        for row in rows:
            self.record('progress.jsonl', **row)
        if success:
            self.current['last_collection'] = self.clock.time()
        self.save()
        return data, success

    def decision(self, instance: dict | None, data: dict | None, success: bool) -> str:
        """照会済みの観測へ、停止判定の表の2行目以降を順に適用する。"""
        record = self.current
        record['nonrunning'] = (record['nonrunning'] + 1 if instance is None or
                                instance['actual_status'] != 'running' else 0)
        if record['nonrunning'] >= 2 and data is None:
            return 'stopped'
        last = record['ssh_at'] if record['last_collection'] is None else record['last_collection']
        if not success and self.clock.time() - last >= 1800:
            return 'communication_failure'
        if data is not None:
            if data['exit_status'] is not None:
                return 'worker_exit'
            if not data['alive']:
                return 'worker_missing'
            cpu = data['cpu_seconds']
            if record['cpu_increased_at'] is None or (cpu is not None and
                    (record['cpu_seconds'] is None or cpu > record['cpu_seconds'])):
                record['cpu_increased_at'] = self.clock.time()
            record['cpu_seconds'] = cpu
            if self.clock.time() - record['cpu_increased_at'] >= 600:
                return 'cpu_stalled'
        return 'continue'

    def loss(self) -> None:
        record = self.current
        for key, row in record['progress'].items():
            if int(key) in self.accepted or row['finished_games'] is None:
                continue
            reused = record['sent_chunk_games'][key] if key in record['sent_chunk_games'] else 0
            lost = row['finished_games'] - (row['collected_games'] - reused) + record['concurrency'] / 2
            self.event('loss_estimate', rental=record['number'], book=int(key),
                       estimated_games=lost, observation=row, reused_games=reused)

    def execute(self) -> None:
        self.initialize()
        self.recover()
        common.run_timed(['git', '-C', str(self.args.repo), 'merge-base', '--is-ancestor',
                          self.args.commit, self.args.ref], 60)
        data = common.run_timed(['git', '-C', str(self.args.repo), 'show', f'{self.args.commit}:{NETWORK}'],
                                60, text=False, stdout=subprocess.PIPE).stdout
        if len(data) < 176 or data[:4] != b'MNUE':
            raise GenerationStop('生成コミットにMNUEのヘッダがありません')
        self.checksum = data[144:176].hex()
        self.accept()
        if self.state['rentals']:
            self.clean_work(self.state['rentals'][-1])
        rejected_offers: set[int] = set()
        while len(self.accepted) < len(self.numbers()):
            if self.current is None:
                if not self.rent(rejected_offers):
                    continue
                if not self.connect():
                    rejected_offers.add(self.current['offer']['id'])
                    self.delete('稼働またはSSH接続の確認が時間内に終わらない')
                    continue
                try:
                    if not self.prepare():
                        continue
                except (RemoteError, common.InvalidResponse) as error:
                    offer_id = self.current['offer']['id']
                    self.delete(f'生成機の準備に失敗しました: {error}')
                    rejected_offers.add(offer_id)
                    continue
            self.limits(None)
            try:
                instance = common.instance(self.current['instance_id'], 60)
                if instance is not None and not isinstance(instance['actual_status'], str):
                    raise common.InvalidResponse('actual_statusを読めません')
            except Exception as error:
                raise GenerationStop(f'状態不明のため新しい機械を借りずに終了します: {error}') from error
            self.current['instance'] = instance
            data, success = self.collect()
            decision = self.decision(instance, data, success)
            self.save()
            if decision == 'stopped':
                record = self.current
                observed = None if instance is None else {key: instance[key] for key in STATUS_FIELDS}
                self.event('stopped', rental=record['number'], instance_id=record['instance_id'], observed=observed)
                self.loss()
                self.salvage()
                self.delete('稼働していない状態を2回観測し、SSHも接続できない')
                self.clean_work(record)
                continue
            if decision != 'continue':
                reasons = {'communication_failure': '回収の失敗が30分続きました',
                           'worker_exit': '作業プロセスが終了コードを残しています',
                           'worker_missing': '作業プロセスが見つかりません',
                           'cpu_stalled': 'CPU時間が10分間増えていません'}
                raise GenerationStop(reasons[decision])
            if len(self.accepted) == len(self.numbers()):
                return
            self.clock.sleep(COLLECT_SECONDS)

    def salvage(self) -> None:
        if self.current is None:
            return
        try:
            self.collect()
        except Exception as error:
            print(f'終了時の回収と受け入れに失敗しました: {error}', file=sys.stderr)
            try:
                self.accept()
            except Exception as error:
                print(f'回収済みの教師ファイルの受け入れに失敗しました: {error}', file=sys.stderr)

    def run(self) -> int:
        code, reason = 0, 'すべての教師ファイルを受け入れた'
        try:
            self.execute()
        except Interrupted as error:
            code, reason = 128 + error.signum, f'シグナル{error.signum}で中断'
        except Exception as error:
            code, reason = 1, str(error)
            print(f'教師生成を終了します: {error}', file=sys.stderr)
        finally:
            # 終了処理中の再度のシグナルでも、削除の確認を途中で放棄しない。
            handlers = {sig: signal.signal(sig, signal.SIG_IGN) for sig in (signal.SIGTERM, signal.SIGINT)}
            try:
                self.salvage()
                record = self.current
                try:
                    self.delete(reason)
                except Exception as error:
                    code = 1
                    print(f'生成機 {record["instance_id"]}（{record["label"]}）の削除を確認できません: '
                          f'{error}。課金が続くため費用の上限を保証できません', file=sys.stderr)
                if not self.delete_orphans():
                    code = 1
                if self.state is not None:
                    self.event('finished', reason=reason, exit_code=code)
            finally:
                for sig, handler in handlers.items():
                    signal.signal(sig, handler)
        return code


def integer(value: str) -> int:
    result = int(value)
    if not 0 <= result < 2**64:
        raise argparse.ArgumentTypeError('0以上2^64未満の整数を指定してください')
    return result


def positive_integer(value: str) -> int:
    result = integer(value)
    if result == 0:
        raise argparse.ArgumentTypeError('正の整数を指定してください')
    return result


def positive_float(value: str) -> float:
    result = float(value)
    if not math.isfinite(result) or result <= 0:
        raise argparse.ArgumentTypeError('正の有限値を指定してください')
    return result


def absolute_path(value: str) -> Path:
    path = Path(value)
    if not path.is_absolute():
        raise argparse.ArgumentTypeError('絶対パスを指定してください')
    return path


def arguments(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='command', required=True)
    run = sub.add_parser('run', help='教師の生成を開始または再開する', argument_default=argparse.SUPPRESS)
    for name in ('run-dir', 'repo'):
        run.add_argument('--' + name, required=True, type=absolute_path, help='絶対パス')
    run.add_argument('--minase-bin', required=True, type=lambda value: Path(value).resolve(),
                     help='受け入れの検査に使う実行ファイル')
    for name in ('ref', 'commit', 'deadline'):
        run.add_argument('--' + name, required=True, help='生成条件と終了期限')
    for name in ('generation', 'first-book', 'last-book', 'seed-base', 'seed-book-step', 'random-moves'):
        run.add_argument('--' + name, required=True, type=integer, help='0以上の整数')
    for name in ('games', 'nodes', 'max-ply', 'hash-mb', 'chunk-games', 'max-rentals'):
        run.add_argument('--' + name, required=True, type=positive_integer, help='正の整数')
    for name in ('max-price', 'max-cost'):
        run.add_argument('--' + name, required=True, type=positive_float, help='米ドル単位の正の有限値')
    run.add_argument('--rental', required=True, choices=('interruptible', 'on-demand'), help='借り方')
    args = parser.parse_args(argv)
    if args.first_book > args.last_book:
        parser.error('first-bookはlast-book以下にしてください')
    if args.seed_base + args.seed_book_step * args.last_book >= 2**64:
        parser.error('最後の教師ファイルのシードが2^64以上です')
    if re.fullmatch(r'[0-9a-fA-F]{40}', args.commit) is None:
        parser.error('commitは40桁の16進数にしてください')
    if not args.ref or args.ref.startswith('-'):
        parser.error('refにGitの参照を指定してください')
    try:
        if re.fullmatch(r'\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z', args.deadline) is None:
            raise ValueError
        datetime.strptime(args.deadline, '%Y-%m-%dT%H:%M:%SZ')
    except ValueError:
        parser.error('deadlineは2026-11-15T03:00:00Zの形のUTCで指定してください')
    return args


def main(argv: list[str]) -> int:
    args = arguments(argv)
    args.run_dir.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(args.run_dir, os.O_RDONLY)
    previous = {}

    def interrupt(signum: int, frame: Any) -> None:
        for sig in (signal.SIGTERM, signal.SIGINT):
            signal.signal(sig, signal.SIG_IGN)
        raise Interrupted(signum)

    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        previous = {sig: signal.signal(sig, interrupt) for sig in (signal.SIGTERM, signal.SIGINT)}
        return TeacherGeneration(args, time).run()
    except BlockingIOError:
        print('同じrun-dirの教師生成が実行中です', file=sys.stderr)
        return 1
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        os.close(descriptor)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
