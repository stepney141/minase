#!/usr/bin/env python3
"""教師の生成、検査、学習、補正、世代のコミットを順に実行する。

使い方: python3 scripts/vast/generation_chain.py run --help
必須条件をすべて指定し、systemd-run --userの同じサービス内で実行する。
--offerは新しく生成機を借りるときだけ指定する。
--books-per-generationで1世代の本数を定め、番号0から指定本数の直前までを生成する。
--window 1はその世代の本だけ、2は直前の世代の本も先に並べて学習に使う。
第1世代の学習と各世代の診断、補正には、その世代の本だけを使う。
--fit yesでは--fit-tau、--fit-seed、--fit-threadsも指定し、補正したネットをコミットする。
--fit noではこれらの補正引数を指定せず、学習したネットをそのままコミットする。
--lr-linearで線形の経路の学習率を指定し、--validation-interval 0で途中検証を省く。
--train-place generatorは生成機で学習、診断、補正し、localは作業機で実行する。
生成機の障害後はlocalを明示して再開する。コミットとbenchはどちらも作業機で行う。
作業機では--run-dirに接続先とgen-<N>/{received,teacher,logs,train,fit}を置く。
--worktreeにネットをコミットし、--ref-prefixで指定した<接頭辞>/gen-<N>で完了を表す。
生成機には同じディレクトリのgeneration_worker.pyを/workspace/job/へ送る。
SIGTERMとSIGINTでは子プロセスだけを止め、同じ引数での再実行に備える。

暫定最良世代に勝てなかった第N世代を37本から74本へ増量して学び直すときは、まずgen-<N>/のtrain、removal.json、fit（補正した場合）、train.log、fit.log、training-logsのうち存在するものをgen-<N>/books37/へ移す。
参照<接頭辞>/gen-<N>が指すコミットを<接頭辞>/gen-<N>-books37として残す。
次にgit update-ref -d <接頭辞>/gen-<N>で世代の参照を消し、ネットをコミットしたブランチの先頭を第N−1世代のコミットへ戻す。
--books-per-generationを74、--first-generationと--last-generationをNにして再実行する。
番号37〜73の本だけが生成され、74本で学び直す。
"""

import argparse
import fcntl
import json
import math
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
from typing import Any

import common

REMOTE = '/workspace/job'
NETWORK = 'crates/minase/nets/nnue.bin'


def low_priority() -> None:
    """学習、補正、ビルド、および検査を、lishogi Botの対局より低い優先度で動かす。"""
    os.nice(19)


class ChainStop(RuntimeError):
    """教師、学習、または補正が継続の条件を満たしていない。"""


class TransportFailure(RuntimeError):
    """SSHまたはrsyncが失敗し、期間を区切った再試行を必要とする。"""


class Interrupted(BaseException):
    """生成機を停止しない、シグナルによる作業機側の中断。"""

    def __init__(self, signum: int) -> None:
        self.signum = signum


class Children:
    """外部コマンドを子プロセスとして起動し、中断時にはその子孫まで回収する。"""

    def __init__(self) -> None:
        self.processes: list[subprocess.Popen] = []

    def start(self, command: list[str], **kwargs: Any) -> subprocess.Popen:
        process = subprocess.Popen(command, start_new_session=True, **kwargs)
        self.processes.append(process)
        return process

    def run(self, command: list[str], **kwargs: Any) -> subprocess.CompletedProcess:
        seconds = kwargs.pop('timeout') if 'timeout' in kwargs else None
        process = self.start(command, **kwargs)
        try:
            output, errors = process.communicate(timeout=seconds)
        except subprocess.TimeoutExpired:
            self.terminate(process)
            raise
        self.processes.remove(process)
        result = subprocess.CompletedProcess(command, process.returncode, output, errors)
        result.check_returncode()
        return result

    def terminate(self, process: subprocess.Popen) -> None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.communicate()
        self.processes.remove(process)

    def finish(self) -> None:
        for process in list(self.processes):
            self.terminate(process)


def book_number(path: Path) -> int:
    match = re.fullmatch(r'book-(0|[1-9][0-9]*)\.mnsd', path.name)
    if match is None:
        raise ChainStop(f'本のファイル名が不正です: {path}')
    return int(match[1])


def read_header(path: Path) -> dict[str, Any]:
    """MNSD版2の公開形式から、検査済みの本のヘッダだけを読む。"""
    with path.open('rb') as stream:
        header = stream.read(152)
    if len(header) != 152:
        raise ChainStop(f'本のヘッダが途中で切れています: {path}')
    magic, version, rules, commit, checksum, kind, reserved, budget, seed, games, entries = struct.unpack(
        '<4sI32s40s32sB7sQQQQ', header)
    if magic != b'MNSD' or version != 2:
        raise ChainStop(f'MNSD版2の本ではありません: {path}')
    return {'generation_commit': commit.decode('ascii'), 'network_checksum': checksum.hex(),
            'rule_set': rules.rstrip(b'\0').decode('utf-8'), 'teacher_search_budget': budget,
            'seed': seed, 'game_count': games, 'entry_count': entries}


def output_fields(output: str) -> dict[str, str]:
    """inspectと生成ログの「項目: 値」を読む。"""
    fields = {}
    for line in output.splitlines():
        key, separator, value = line.partition(':')
        if separator and value.strip():
            fields[key.strip()] = value.strip()
    return fields


def deadline_hours(counts: list[int], args: argparse.Namespace) -> float:
    """残りの本の生成時間に、生成機の停止前に必要な各世代の学習待ちを加える。"""
    generations = len(counts)
    books = sum(max(0, args.books_per_generation - count) for count in counts)
    hours = args.book_minutes * books * args.deadline_margin / 60
    hours += args.training_wait_hours * (generations if args.train_place == 'generator' else generations - 1)
    if hours <= 0 or not math.isfinite(hours):
        raise ChainStop('残りの本数と学習待ちから正の停止期限を計算できません')
    return hours


def onstart(deadline: int) -> str:
    script = common.onstart_prefix(deadline) + (
        "nohup setsid bash -c 'set -euo pipefail; until [[ -f /workspace/job/started ]]; "
        "do sleep 2; done; exec python3 /workspace/job/generation_worker.py run' "
        '>> /workspace/job/logs/worker.log 2>&1 < /dev/null &\n')
    if len(script) > 4048:
        raise ChainStop(f'起動時スクリプトが4048文字を超えています: {len(script)}')
    return script


def select_offer(offers: list[dict[str, Any]], args: argparse.Namespace) -> dict[str, Any]:
    """指定IDの出品を選び、作成の前にすべての貸出条件を確かめる。"""
    matches = [offer for offer in offers if str(offer['id']) == str(args.offer)]
    if len(matches) != 1:
        raise ChainStop('指定した出品が見つからないか、照会結果に重複しています')
    offer = matches[0]
    minimums = [('cpu_cores', args.min_cpu_cores), ('duration', args.min_duration_days * 86400),
                ('reliability2', args.min_reliability), ('direct_port_count', 1),
                ('inet_up', args.min_inet_up), ('disk_space', args.min_disk)]
    if args.train_place == 'generator':
        minimums.extend([('num_gpus', 1), ('cuda_max_good', 13.0)])
    for key, minimum in minimums:
        value = offer[key]
        if type(value) not in (int, float) or not math.isfinite(value) or value < minimum:
            raise ChainStop(f'出品の{key}が条件を満たしません: {value}')
    if offer['gpu_frac'] != 1 or offer['cpu_cores_effective'] != offer['cpu_cores']:
        raise ChainStop('出品がGPUとCPUの全体を割り当てる条件を満たしません')
    if offer['verification'] != 'verified':
        raise ChainStop(f'出品が検証済みではありません: {offer["verification"]}')
    if re.search(args.cpu_pattern, offer['cpu_name']) is None:
        raise ChainStop(f'出品のCPU名が条件を満たしません: {offer["cpu_name"]}')
    price = offer['dph_total']
    if type(price) not in (int, float) or not math.isfinite(price) or not 0 <= price <= args.max_price:
        raise ChainStop(f'出品の時間単価が条件を満たしません: {price}')
    return offer


class Chain:
    """接続先を保持し、成果物とGit参照を根拠に世代反復を進める。"""

    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.root = args.run_dir
        self.children = Children()
        self.instance_id: int | None = None
        self.address: tuple[str, int] | None = None
        self.manage_remote = False
        self.concurrency: int | None = None
        self.current: int | None = None
        self.started = time.monotonic()

    def git(self, *args: str) -> str:
        return self.children.run(['git', '-C', str(self.args.worktree), *args],
                                 text=True, stdout=subprocess.PIPE).stdout.strip()

    def reference(self, generation: int) -> str:
        return f'{self.args.ref_prefix}/gen-{generation}'

    def commit_for(self, generation: int) -> str | None:
        try:
            return self.git('rev-parse', '--verify', '--quiet', self.reference(generation) + '^{commit}')
        except subprocess.CalledProcessError as error:
            if error.returncode == 1:
                return None
            raise

    def previous_commit(self, generation: int) -> str:
        commit = self.commit_for(generation - 1)
        if commit is None:
            raise ChainStop(f'第{generation - 1}世代のコミット参照がありません')
        return commit

    def checksum(self, commit: str) -> str:
        data = self.children.run(['git', '-C', str(self.args.worktree), 'show', f'{commit}:{NETWORK}'],
                                 stdout=subprocess.PIPE).stdout
        if len(data) < 176 or data[:4] != b'MNUE':
            raise ChainStop(f'コミット{commit}にMNUEのヘッダがありません')
        return data[144:176].hex()

    def book_paths(self, generation: int) -> list[Path]:
        return sorted((self.root / f'gen-{generation}/teacher').glob('book-*.mnsd'), key=book_number)

    def seed(self, generation: int, number: int) -> int:
        return self.args.seed_base + self.args.seed_generation_step * generation + self.args.seed_book_step * number

    def expected_header(self, generation: int, number: int, commit: str, checksum: str) -> dict[str, Any]:
        return {'generation_commit': commit, 'network_checksum': checksum, 'rule_set': 'L0,P0,R1,E0',
                'seed': self.seed(generation, number), 'teacher_search_budget': self.args.nodes}

    def check_header(self, header: dict[str, Any], generation: int, number: int,
                     commit: str, checksum: str) -> None:
        for key, value in self.expected_header(generation, number, commit, checksum).items():
            if str(header[key]) != str(value):
                raise ChainStop(f'第{generation}世代の本{number}: {key}が一致しません')

    def recorded_positions(self, generation: int, number: int) -> int:
        """学習に使える局面の数を、検査済みの本の生成ログから読む。"""
        log = (self.root / f'gen-{generation}/logs/book-{number}.log').read_text()
        return int(output_fields(log)['recorded_positions'])

    def books(self, generation: int, commit: str, checksum: str) -> dict[int, int]:
        result = {}
        for path in self.book_paths(generation):
            number = book_number(path)
            self.check_header(read_header(path), generation, number, commit, checksum)
            result[number] = self.recorded_positions(generation, number)
        return result

    def ssh(self, command: str, seconds: float) -> str:
        if self.address is None:
            raise ChainStop('保存済みのSSH接続先がありません')
        host, port = self.address
        try:
            return self.children.run(['ssh', *common.ssh_options(port, '-p'), f'root@{host}', command],
                                     text=True, stdout=subprocess.PIPE, timeout=seconds).stdout
        except subprocess.CalledProcessError as error:
            if error.returncode != 255:
                raise
            raise TransportFailure('SSH接続に失敗しました') from error
        except (OSError, subprocess.TimeoutExpired) as error:
            raise TransportFailure('SSH接続に失敗しました') from error

    def remote_exists(self, path: str) -> bool:
        try:
            self.ssh('test -f ' + shlex.quote(path), 60)
            return True
        except subprocess.CalledProcessError as error:
            if error.returncode == 1:
                return False
            raise

    def upload(self, source: Path, destination: str) -> None:
        if self.address is None:
            raise ChainStop('転送する生成機がありません')
        try:
            common.scp(*self.address, destination, source)
        except (OSError, subprocess.SubprocessError) as error:
            raise TransportFailure('生成機への転送に失敗しました') from error

    def cpu_count(self) -> int:
        info = json.loads(self.ssh(f'python3 {REMOTE}/generation_worker.py cpu-info', 60))
        print('生成機のCPU割り当て: ' + json.dumps(info, ensure_ascii=False), flush=True)
        count, limit = info['nproc'], info['cpu_limit']
        if type(count) is not int or count < self.args.min_cpu_cores:
            raise ChainStop('生成機のcpusetが必要なスレッド数を満たしません')
        if limit is not None and (type(limit) not in (int, float) or not math.isfinite(limit)
                                  or limit < 0.9 * self.args.min_cpu_cores):
            raise ChainStop('生成機のcgroupのCPU上限が必要量の90%未満です')
        return count

    def clear_instance(self) -> None:
        for name in ('instance-id', 'ssh-address', 'deadline'):
            (self.root / name).unlink(missing_ok=True)
        self.instance_id = None
        self.address = None
        self.manage_remote = False

    def prepare_machine(self, pending: list[int], needs_generation: bool) -> None:
        """借り直しが明示されたときだけ口座を照会し、通常の再開はSSHだけを使う。"""
        args = self.args
        offer_given = hasattr(args, 'offer')
        if self.instance_id is None and not needs_generation:
            return
        if self.instance_id is not None and offer_given:
            if common.instance(self.instance_id, 600) is not None:
                raise ChainStop(f'生成機 {self.instance_id} はまだ存在します。--offerを外して再開してください')
            self.clear_instance()
            if args.train_place == 'generator' and not needs_generation:
                return
        needs_training = args.train_place == 'generator' and any(
            not self.training_complete(generation) for generation in pending)
        if self.instance_id is not None and not needs_generation and not needs_training:
            # 残りが最後の世代の学習だけなら、生成機はすでに停止済みであり得るので接続を求めない。
            address = self.root / 'ssh-address'
            if address.is_file():
                host, port = address.read_text().split()
                self.address = host, int(port)
            self.manage_remote = True
            return
        if self.instance_id is not None:
            try:
                host, port = (self.root / 'ssh-address').read_text().split()
                self.address = host, int(port)
                self.ssh('true', 60)
            except Exception as error:
                raise ChainStop(f'生成機 {self.instance_id} に接続できません。稼働状態、貸出期限、'
                                'SSH接続先を確認し、削除済みの場合だけ--offerを指定してください') from error
            self.manage_remote = True
            self.concurrency = self.cpu_count()
            return
        if not offer_given:
            if needs_generation:
                raise ChainStop('生成が残っていますが生成機がありません。--offerを指定してください')
            return
        label = 'minase-generation-chain-' + self.root.name
        existing = common.ids(label)
        if existing:
            raise ChainStop(f'同じラベルの生成機があります: {existing}。状態を確認してください')
        query = (f'cpu_cores>={args.min_cpu_cores} verified=any reliability>={args.min_reliability} '
                 f'inet_up>={args.min_inet_up} disk_space>={args.min_disk} rentable=true gpu_frac=1 '
                 f'duration>={args.min_duration_days} direct_port_count>=1 dph_total<={args.max_price}')
        raw = common.call(600, 'search', 'offers', query, '--type', 'on-demand', '--raw', '--limit', '1000')
        offer = select_offer(json.loads(raw), args)
        hours = deadline_hours([len(self.book_paths(generation)) for generation in pending], args)
        credit = json.loads(common.call(600, 'show', 'user', '--raw'))['credit']
        print(f'出品 {offer["id"]}: CPU={offer["cpu_name"]}、単価={offer["dph_total"]}米ドル/時、'
              f'期限まで={hours:.3f}時間、見込み費用={offer["dph_total"] * hours:.3f}米ドル、'
              f'残高={credit}米ドル', flush=True)
        deadline = math.ceil(time.time() + hours * 3600)
        (self.root / 'offer.json').write_text(json.dumps(offer, ensure_ascii=False) + '\n')
        (self.root / 'onstart.sh').write_text(onstart(deadline))
        (self.root / 'deadline').write_text(f'{deadline}\n')
        self.instance_id = common.create(str(args.offer), label, self.root / 'onstart.sh', args.image, args.disk)
        self.manage_remote = True
        (self.root / 'instance-id').write_text(f'{self.instance_id}\n')
        common.wait_running(self.instance_id, 600)
        self.address = common.ssh_address(self.instance_id, 600)
        (self.root / 'ssh-address').write_text(f'{self.address[0]} {self.address[1]}\n')
        common.wait_ssh(self.instance_id, *self.address)
        with tempfile.TemporaryDirectory() as temporary:
            config = Path(temporary) / 'worker.json'
            config.write_text(json.dumps({'idle_limit_seconds': args.idle_limit_hours * 3600}))
            self.upload(Path(__file__).with_name('generation_worker.py'), REMOTE + '/')
            self.upload(config, REMOTE + '/')
        self.concurrency = self.cpu_count()
        self.ssh(f'touch {REMOTE}/started', 60)

    def send_bundle(self, generation: int) -> str:
        destination = f'{REMOTE}/bundles/gen-{generation - 1}.bundle'
        if not self.remote_exists(destination):
            self.ssh(f'mkdir -p {REMOTE}/bundles', 60)
            with tempfile.TemporaryDirectory() as temporary:
                path = Path(temporary) / 'source.bundle'
                self.git('bundle', 'create', str(path), self.reference(generation - 1))
                self.upload(path, destination + '.tmp')
            self.ssh(f'mv {destination}.tmp {destination}', 60)
        return destination

    def send_job(self, generation: int, commit: str, bundle: str, books: dict[int, int]) -> None:
        if any(number >= self.args.books_per_generation for number in books):
            raise ChainStop(f'第{generation}世代の本の番号がbooks-per-generation以上です')
        if self.concurrency is None:
            raise ChainStop('生成機の同時対局数がありません')
        job = {'generation': generation, 'commit': commit, 'bundle': bundle,
               'reference': self.reference(generation - 1),
               'generation_seed': self.seed(generation, 0), 'book_step': self.args.seed_book_step,
               'target_books': self.args.books_per_generation,
               'skip_books': sorted(books), 'concurrency': self.concurrency}
        for key in ('games', 'nodes', 'random_moves', 'max_ply', 'hash_mb'):
            job[key] = getattr(self.args, key)
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'job.json'
            path.write_text(json.dumps(job) + '\n')
            self.upload(path, REMOTE + '/job.json.tmp')
        self.ssh(f'mv {REMOTE}/job.json.tmp {REMOTE}/job.json', 60)

    def fetch(self, generation: int) -> None:
        if self.address is None:
            raise ChainStop('取得する生成機がありません')
        directory = self.root / f'gen-{generation}'
        for remote, local in (('teacher', 'received'), ('logs', 'logs')):
            destination = directory / local
            destination.mkdir(parents=True, exist_ok=True)
            try:
                common.rsync_to(*self.address, f'{REMOTE}/{remote}/gen-{generation}/', destination)
            except subprocess.CalledProcessError as error:
                if error.returncode != 23:
                    raise TransportFailure('生成機からの取得に失敗しました') from error
            except (OSError, subprocess.TimeoutExpired) as error:
                raise TransportFailure('生成機からの取得に失敗しました') from error

    def accept_books(self, generation: int, commit: str, checksum: str, books: dict[int, int],
                     best_effort: bool) -> None:
        """全対局の復号とログ照合が成功した本だけを、番号順に学習入力へ加える。"""
        directory = self.root / f'gen-{generation}'
        teacher = directory / 'teacher'
        teacher.mkdir(parents=True, exist_ok=True)
        for path in sorted((directory / 'received').glob('book-*.mnsd'), key=book_number):
            number = book_number(path)
            if number >= self.args.books_per_generation:
                raise ChainStop(f'本の番号{number}がbooks-per-generation以上です')
            if len(books) >= self.args.books_per_generation:
                continue
            log_path = directory / 'logs' / f'book-{number}.log'
            if number in books or not log_path.is_file():
                continue
            try:
                output = self.children.run([str(self.args.minase_bin), 'data', 'selfplay', 'inspect', str(path)],
                                           text=True, stdout=subprocess.PIPE, preexec_fn=low_priority).stdout
                header = output_fields(output)
                self.check_header(header, generation, number, commit, checksum)
                log = output_fields(log_path.read_text())
                for key, value in (('games_completed', self.args.games), ('nodes', self.args.nodes),
                                   ('max_ply', self.args.max_ply), ('hash_mb', self.args.hash_mb),
                                   ('random_moves_max', self.args.random_moves)):
                    if int(log[key]) != value:
                        raise ChainStop(f'本{number}の生成ログの{key}が一致しません')
                games, discarded = int(header['game_count']), int(log['games_discarded_max_ply'])
                entries = int(header['eligible_positions'])
                if games < 0 or discarded < 0 or games + discarded != self.args.games or entries < 0:
                    raise ChainStop(f'本{number}の局数または局面数が不正です')
                if int(log['recorded_positions']) != entries:
                    raise ChainStop(f'本{number}の生成ログの局面数が本の内容と一致しません')
                os.link(path, teacher / path.name)
                books[number] = entries
                print(f'第{generation}世代の本{number}: 経過{time.monotonic() - self.started:.1f}秒、'
                      f'{games}局、{entries}局面、合計{sum(books.values())}局面', flush=True)
            except Exception as error:
                if not best_effort:
                    raise
                print(f'終了時の本{number}の検査に失敗しました: {error}', file=sys.stderr)

    def calibrate(self, generation: int) -> None:
        if generation != 1 or (self.root / 'base.bin').is_file():
            return
        books = self.book_paths(1)
        if len(books) < self.args.calibration_books:
            return
        calibration = self.root / 'calibration.json'
        if not calibration.is_file():
            self.children.run([str(self.args.trainer_bin), 'data', 'calibrate', '--data',
                               *map(str, books[:self.args.calibration_books]), '--output', str(calibration)],
                              preexec_fn=low_priority)
        classes = json.loads(calibration.read_text())['classes']
        if len(classes) != 1:
            raise ChainStop('較正結果のclassesが1項目ではありません')
        scale = classes[0]['teacher_cp_per_logit']
        if type(scale) not in (int, float) or not math.isfinite(scale) or scale <= 0:
            raise ChainStop('較正した教師の勝率尺度が正の有限値ではありません')
        if classes[0]['at_search_bound'] is not False:
            raise ChainStop(f'較正した教師の勝率尺度が探索範囲の端にあります: {scale}')
        self.children.run([str(self.args.trainer_bin), 'nnue', 'export-handcrafted', '--cp-per-logit',
                           str(scale), '--output', str(self.root / 'base.bin')])

    def generate(self, generation: int, commit: str, checksum: str, books: dict[int, int]) -> None:
        if any(number >= self.args.books_per_generation for number in books):
            raise ChainStop(f'第{generation}世代の本の番号がbooks-per-generation以上です')
        if len(books) >= self.args.books_per_generation:
            return
        bundle = None
        sent: tuple[tuple[int, int], ...] | None = None
        last_book = time.monotonic()
        failed_since = None
        while True:
            try:
                if bundle is None:
                    bundle = self.send_bundle(generation)
                if sent != tuple(sorted(books.items())):
                    self.send_job(generation, commit, bundle, books)
                    sent = tuple(sorted(books.items()))
                count = len(books)
                self.fetch(generation)
                self.accept_books(generation, commit, checksum, books, False)
                if len(books) > count:
                    last_book = time.monotonic()
                self.calibrate(generation)
                if self.remote_exists(REMOTE + '/logs/exit-status'):
                    code = self.ssh(f'cat {REMOTE}/logs/exit-status', 60).strip()
                    raise ChainStop(f'生成機の作業者が終了しました: 終了コード{code}')
                failed_since = None
            except TransportFailure as error:
                now = time.monotonic()
                if failed_since is None:
                    failed_since = now
                if now - failed_since >= self.args.ssh_failure_minutes * 60:
                    raise ChainStop(f'SSHまたはrsyncの失敗が上限時間まで続きました: {error}') from error
                print(f'{error}。再試行します', file=sys.stderr)
            else:
                if len(books) >= self.args.books_per_generation:
                    return
            if time.monotonic() - last_book >= self.args.book_stall_minutes * 60:
                raise ChainStop('検査に通った新しい本が得られないまま上限時間に達しました')
            time.sleep(self.args.poll_seconds)

    def training_books(self, generation: int) -> list[Path]:
        """窓に含める直前の世代の本を検査し、世代順と番号順で学習入力を返す。"""
        previous = []
        if self.args.window == 2 and generation >= 2:
            commit = self.previous_commit(generation - 1)
            if not self.books(generation - 1, commit, self.checksum(commit)):
                raise ChainStop(f'第{generation - 1}世代の教師がありません')
            previous = self.book_paths(generation - 1)
        return previous + self.book_paths(generation)

    def network_path(self, generation: int) -> Path:
        stage = 'fit' if self.args.fit == 'yes' else 'train'
        return self.root / f'gen-{generation}' / stage / 'nnue.bin'

    def training_base(self, generation: int) -> Path:
        """次の学習が、前の世代で実際にコミットしたネットを引き継ぐことを確かめる。"""
        base = self.root / 'base.bin' if generation == 1 else self.network_path(generation - 1)
        if not base.is_file():
            raise ChainStop(f'学習の初期値がありません: {base}')
        if generation >= 2:
            commit = self.previous_commit(generation)
            data = self.children.run(['git', '-C', str(self.args.worktree), 'show', f'{commit}:{NETWORK}'],
                                     stdout=subprocess.PIPE).stdout
            if base.read_bytes() != data:
                raise ChainStop(f'学習の初期値が第{generation - 1}世代のコミットのネットと一致しません: {base}')
        return base

    def training_commands(self, trainer: Path, training_books: list[Path], current_books: list[Path], base: Path,
                          directory: Path) -> dict[str, list[str]]:
        """実行場所によらず、同じ条件で学習・診断・補正のコマンドを組み立てる。"""
        args = self.args
        train = directory / 'train'
        command = [str(trainer), 'nnue', 'train', '--data', *map(str, training_books), '--base', str(base),
                   '--lambda', str(args.mix), '--teacher-cp-per-logit', 'calibrate',
                   '--seed', str(args.train_seed), '--threads', str(args.train_threads), '--output-dir', str(train)]
        for key in ('feature_set', 'width', 'head', 'batch', 'epochs', 'patience', 'device',
                    'validation_interval', 'lr_schedule', 'lr_linear'):
            command.extend(['--' + key.replace('_', '-'), str(getattr(args, key))])
        if args.export_epoch == 'last':
            command.extend(['--export-epoch', 'last'])
        checkpoint = {'best': 'best.npz', 'last': 'latest.npz'}[args.export_epoch]
        commands = {
            'train': command,
            'removal': [str(trainer), 'diag', 'nnue-removal', '--float', str(train / checkpoint),
                        '--net', str(train / 'nnue.bin'), '--data', *map(str, current_books),
                        '--sample', str(args.removal_sample), '--seed', str(args.removal_seed),
                        '--output', str(directory / 'removal.json')],
        }
        if args.fit == 'yes':
            commands['fit'] = [str(trainer), 'nnue', 'fit-linear', '--base', str(train / 'nnue.bin'),
                               '--data', *map(str, current_books), '--tau', *map(str, args.fit_tau),
                               '--seed', str(args.fit_seed), '--threads', str(args.fit_threads),
                               '--output-dir', str(directory / 'fit')]
        return commands

    def training_complete(self, generation: int) -> bool:
        directory = self.root / f'gen-{generation}'
        summaries = ['train/summary.json', 'removal.json']
        if self.args.fit == 'yes':
            summaries.append('fit/summary.json')
        return all((directory / name).is_file() for name in summaries)

    def receive_training(self, generation: int) -> None:
        """必要な成果物を取得してから移し、最後の移動で学習の完了を公開する。"""
        if self.address is None:
            raise ChainStop('学習結果を取得する生成機がありません')
        directory = self.root / f'gen-{generation}'
        received = directory / 'training-received'
        if received.exists():
            shutil.rmtree(received)
        received.mkdir(parents=True)
        try:
            common.rsync_to(*self.address, f'{REMOTE}/training/gen-{generation}/', received)
        except (OSError, subprocess.SubprocessError) as error:
            raise TransportFailure('生成機からの学習結果の取得に失敗しました') from error
        artifacts = ['train', 'removal.json']
        if self.args.fit == 'yes':
            artifacts.append('fit')
        for artifact in artifacts:
            name = artifact if artifact == 'removal.json' else artifact + '/summary.json'
            if not (received / name).is_file():
                raise ChainStop(f'第{generation}世代の学習結果がそろっていません: {name}。'
                                '本は作業機にあるので、--train-place localで再開できます')
        # 古い成果物が残っていても、移動の途中で完了と判定しない。
        for name in artifacts:
            destination = directory / name
            if destination.is_dir():
                shutil.rmtree(destination)
            elif destination.exists():
                destination.unlink()
        for name in artifacts:
            os.replace(received / name, directory / name)
        shutil.rmtree(received)

    def train_on_generator(self, generation: int, commit: str, books: dict[int, int]) -> None:
        """生成とは独立した作業者へ学習を渡し、結果を取得するまで生成機を保持する。"""
        if self.training_complete(generation):
            return
        resume = '本は作業機にあるので、--train-place localで再開できます'
        if self.address is None:
            raise ChainStop(f'学習できる生成機がありません。{resume}')
        base = self.training_base(generation)
        training_books = self.training_books(generation)
        directory = Path(REMOTE) / 'training' / f'gen-{generation}'
        teacher = Path(REMOTE) / 'teacher' / f'gen-{generation}'
        logs = self.root / f'gen-{generation}/training-logs'
        logs.mkdir(parents=True, exist_ok=True)
        started = time.monotonic()
        failed_since = None
        prepared = False
        while True:
            if time.monotonic() - started >= self.args.training_wait_hours * 3600:
                raise ChainStop(f'第{generation}世代の学習待ちが上限時間に達しました。ログ: {logs}。{resume}')
            try:
                if not prepared:
                    bundle = self.send_bundle(generation)
                    self.send_job(generation, commit, bundle, books)
                    self.upload(Path(__file__).with_name('generation_worker.py'), REMOTE + '/generation_worker.py.tmp')
                    self.ssh(f'mv {REMOTE}/generation_worker.py.tmp {REMOTE}/generation_worker.py', 60)
                    if not self.remote_exists(str(directory / 'job.json')):
                        self.ssh(f'mkdir -p {directory}', 60)
                        try:
                            for source in dict.fromkeys(path.parent for path in training_books):
                                destination = Path(REMOTE) / 'teacher' / source.parent.name
                                self.ssh(f'mkdir -p {destination}', 60)
                                common.rsync_push(*self.address, source, str(destination))
                        except (OSError, subprocess.SubprocessError) as error:
                            raise TransportFailure('生成機への教師の転送に失敗しました') from error
                        self.upload(base, str(directory / 'base.bin'))
                        commands = self.training_commands(
                            Path(REMOTE) / 'minase/target/release/minase-trainer',
                            [Path(REMOTE) / 'teacher' / path.parent.parent.name / path.name
                             for path in training_books],
                            [teacher / f'book-{number}.mnsd' for number in sorted(books)],
                            directory / 'base.bin', directory)
                        steps = [{'name': 'build',
                                  'command': ['cargo', 'build', '--release', '--locked', '--bin', 'minase-trainer'],
                                  'done': None, 'clean': None, 'resume_file': None, 'resume_args': None}]
                        for name in commands:
                            done = 'removal.json' if name == 'removal' else f'{name}/summary.json'
                            steps.append({'name': name, 'command': commands[name], 'done': str(directory / done),
                                          'clean': str(directory / name) if name in ('train', 'fit') else None,
                                          'resume_file': str(directory / 'train/latest.npz') if name == 'train' else None,
                                          'resume_args': ['--resume', str(directory / 'train')] if name == 'train' else None})
                        with tempfile.TemporaryDirectory() as temporary:
                            path = Path(temporary) / 'job.json'
                            path.write_text(json.dumps({'steps': steps}) + '\n')
                            self.upload(path, str(directory / 'job.json.tmp'))
                        self.ssh(f'mv {directory}/job.json.tmp {directory}/job.json', 60)
                    prepared = True
                try:
                    common.rsync_to(*self.address, str(directory / 'logs') + '/', logs)
                except subprocess.CalledProcessError as error:
                    if error.returncode != 23:
                        raise TransportFailure('生成機からの学習ログの取得に失敗しました') from error
                except (OSError, subprocess.TimeoutExpired) as error:
                    raise TransportFailure('生成機からの学習ログの取得に失敗しました') from error
                if self.remote_exists(str(directory / 'exit-status')):
                    code = self.ssh(f'cat {directory}/exit-status', 60).strip()
                    raise ChainStop(f'生成機の学習作業者が終了しました: 終了コード{code}。ログ: {logs}。{resume}')
                if self.remote_exists(str(directory / 'done')):
                    self.receive_training(generation)
                    return
                self.ssh(f'nohup setsid python3 {REMOTE}/generation_worker.py train {generation} '
                         f'>> {REMOTE}/logs/train-worker.log 2>&1 < /dev/null &', 60)
                failed_since = None
            except TransportFailure as error:
                now = time.monotonic()
                if failed_since is None:
                    failed_since = now
                if now - failed_since >= self.args.ssh_failure_minutes * 60:
                    raise ChainStop(f'SSHまたはrsyncの失敗が上限時間まで続きました: {error}。{resume}') from error
                print(f'{error}。再試行します', file=sys.stderr)
            time.sleep(self.args.poll_seconds)

    def train(self, generation: int) -> dict[str, Any]:
        args = self.args
        directory = self.root / f'gen-{generation}'
        train = directory / 'train'
        train.mkdir(parents=True, exist_ok=True)
        base = self.training_base(generation)
        commands = self.training_commands(args.trainer_bin, self.training_books(generation),
                                          self.book_paths(generation), base, directory)
        summary = train / 'summary.json'
        if not summary.is_file():
            command = commands['train']
            if (train / 'latest.npz').is_file():
                command.extend(['--resume', str(train)])
            with (directory / 'train.log').open('a') as log:
                self.children.run(command, cwd=args.worktree, stdout=log, stderr=subprocess.STDOUT,
                                  preexec_fn=low_priority)
        result = json.loads(summary.read_text())
        if result['status'] != 'candidate':
            raise ChainStop(f'第{generation}世代の学習結果がcandidateではありません: {result["status"]}')
        removal = directory / 'removal.json'
        if not removal.is_file():
            self.children.run(commands['removal'], cwd=args.worktree, preexec_fn=low_priority)
        return result

    def fit(self, generation: int) -> dict[str, Any]:
        args = self.args
        directory = self.root / f'gen-{generation}'
        fit = directory / 'fit'
        summary = fit / 'summary.json'
        if not summary.is_file():
            # fit-linearは空でない出力先を拒否し、完了時にsummary.jsonを書くため、未完了の出力を消す。
            if fit.exists():
                shutil.rmtree(fit)
            base = self.root / 'base.bin' if generation == 1 else self.network_path(generation - 1)
            books = self.book_paths(generation)
            command = self.training_commands(args.trainer_bin, books, books, base, directory)['fit']
            with (directory / 'fit.log').open('a') as log:
                self.children.run(command, cwd=args.worktree, stdout=log, stderr=subprocess.STDOUT,
                                  preexec_fn=low_priority)
        result = json.loads(summary.read_text())
        try:
            d = result['decision']['d']
        except (KeyError, TypeError) as error:
            raise ChainStop(f'第{generation}世代の補正結果にdecision.dがありません') from error
        if type(d) not in (int, float) or not math.isfinite(d) or d >= 0:
            raise ChainStop(f'第{generation}世代の補正結果のdecision.dが負の有限値ではありません: {d}。'
                            '継続には利用者の判断が必要です')
        return result

    def commit_generation(self, generation: int, summary: dict[str, Any], books: dict[int, int], d: float | None) -> None:
        network = self.network_path(generation)
        data = self.children.run(['git', '-C', str(self.args.worktree), 'show', f'HEAD:{NETWORK}'],
                                 stdout=subprocess.PIPE).stdout
        if data != network.read_bytes():
            shutil.copyfile(network, self.args.worktree / NETWORK)
            try:
                self.children.run(['cargo', 'build', '--release', '--locked', '--bin', 'minase'], cwd=self.args.worktree,
                                  preexec_fn=low_priority)
                self.children.run([str(self.args.worktree / 'target/release/minase'), 'dev', 'bench',
                                   '--depth', str(self.args.bench_depth)], cwd=self.args.worktree)
            except BaseException:
                self.git('checkout', 'HEAD', '--', NETWORK)
                raise
            self.git('add', '--', NETWORK)
            body = f'教師{len(books)}本、{sum(books.values())}局面、最良エポック{summary["best_epoch"]}'
            if self.args.fit == 'yes':
                body += f'、D={d:.6f}'
            self.git('commit', '-m', f'feat(eval): embed the NNUE of generation {generation}',
                     '-m', body + '。',
                     '-m', 'Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>', '--', NETWORK)
        self.git('update-ref', self.reference(generation), 'HEAD')

    def destroy(self) -> bool:
        if self.instance_id is None:
            return True
        try:
            common.destroy(self.instance_id)
        except Exception as error:
            print(f'生成機 {self.instance_id} の削除を確認できません: {error}。'
                  '口座の認証と生成機の状態を確認してください', file=sys.stderr)
            return False
        self.clear_instance()
        return True

    def stop_machine(self) -> bool:
        """SSH切断で停止を確認できた場合、削除に成功した場合、管理する生成機がない場合にTrueを返す。"""
        if self.instance_id is None:
            return True
        stopped = False
        try:
            self.ssh(f'python3 {REMOTE}/generation_worker.py stop', 1860)
            end = time.monotonic() + 300
            while (remaining := end - time.monotonic()) > 0:
                try:
                    self.ssh('true', min(20, remaining))
                except TransportFailure:
                    stopped = True
                    break
                time.sleep(min(self.args.poll_seconds, max(0, end - time.monotonic())))
            else:
                print(f'生成機 {self.instance_id}: 5分後もSSHが接続できます。'
                      '管理画面で停止状態を確認してください', file=sys.stderr)
        except Exception as error:
            print(f'生成機 {self.instance_id} の停止を確認できません: {error}。'
                  '管理画面で停止し、課金状態を確認してください', file=sys.stderr)
        return self.destroy() or stopped

    def salvage(self) -> None:
        if self.current is None or self.address is None:
            return
        try:
            commit = self.previous_commit(self.current)
            checksum = self.checksum(commit)
            books = self.books(self.current, commit, checksum)
            # 学習開始後に教師を増やすと、再開時の入力が変わる。
            if len(books) >= self.args.books_per_generation:
                return
            try:
                self.fetch(self.current)
            except Exception as error:
                print(f'終了時の転送に失敗しました: {error}', file=sys.stderr)
            self.accept_books(self.current, commit, checksum, books, True)
        except Exception as error:
            print(f'終了時の取得に失敗しました: {error}', file=sys.stderr)

    def execute(self) -> None:
        self.root.mkdir(parents=True, exist_ok=True)
        path = self.root / 'instance-id'
        if path.is_file():
            self.instance_id = int(path.read_text())
        gen0 = self.git('rev-parse', '--verify', '--end-of-options', self.args.gen0_commit + '^{commit}')
        self.git('update-ref', self.reference(0), gen0)
        for generation in range(self.args.first_generation):
            if self.commit_for(generation) is None:
                raise ChainStop(f'開始世代より前の第{generation}世代のコミット参照がありません')
        pending = [generation for generation in range(self.args.first_generation, self.args.last_generation + 1)
                   if self.commit_for(generation) is None]
        if pending:
            needs_generation = any(len(self.book_paths(generation)) < self.args.books_per_generation
                                   for generation in pending)
            self.prepare_machine(pending, needs_generation)
        for generation in pending:
            self.current = generation
            commit = self.previous_commit(generation)
            checksum = self.checksum(commit)
            books = self.books(generation, commit, checksum)
            self.generate(generation, commit, checksum, books)
            self.calibrate(generation)
            if generation == 1 and not (self.root / 'base.bin').is_file():
                raise ChainStop('第1世代の較正に必要な本数が足りません')
            if self.args.train_place == 'generator':
                self.train_on_generator(generation, commit, books)
            if generation == self.args.last_generation:
                if not self.stop_machine():
                    raise ChainStop(f'生成機 {self.instance_id} の停止も削除も確認できません。'
                                    '管理画面で停止と課金状態を確認してください')
            summary = self.train(generation)
            d = self.fit(generation)['decision']['d'] if self.args.fit == 'yes' else None
            self.commit_generation(generation, summary, books, d)
        self.manage_remote = False
        if not self.destroy():
            raise ChainStop(f'世代の作成は終了しましたが、生成機 {self.instance_id} の削除が未確認です。'
                            '口座の認証を更新し、同じコマンドを再実行すると削除だけを行います')
        print('完了', flush=True)

    def run(self) -> int:
        try:
            self.execute()
            return 0
        except Interrupted as error:
            print('作業機の処理を中断しました。生成機は停止しません。同じコマンドで再開できます', file=sys.stderr)
            return 128 + error.signum
        except Exception as error:
            if self.manage_remote:
                self.salvage()
                self.stop_machine()
            print(f'世代反復を停止しました: {error}', file=sys.stderr)
            return 1
        finally:
            self.children.finish()


def positive_int(value: str) -> int:
    result = int(value)
    if result <= 0:
        raise argparse.ArgumentTypeError('正の整数を指定してください')
    return result


def nonnegative_int(value: str) -> int:
    result = int(value)
    if not 0 <= result < 2**64:
        raise argparse.ArgumentTypeError('0以上2^64未満の整数を指定してください')
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
    run = sub.add_parser('run', help='世代の生成と学習を開始または再開する', argument_default=argparse.SUPPRESS)
    for name in ('run-dir', 'worktree'):
        run.add_argument('--' + name, required=True, type=absolute_path, help='配置先の絶対パス')
    for name in ('minase-bin', 'trainer-bin'):
        run.add_argument('--' + name, required=True, type=lambda value: Path(value).resolve(), help='固定した実行ファイルのパス')
    for name in ('gen0-commit', 'ref-prefix', 'feature-set', 'head', 'device', 'lr-schedule', 'image', 'cpu-pattern'):
        run.add_argument('--' + name, required=True, help='実行条件を明示する')
    for name in ('first-generation', 'last-generation', 'books-per-generation', 'games', 'nodes',
                 'max-ply', 'hash-mb', 'calibration-books', 'width', 'batch', 'epochs', 'patience',
                 'train-threads', 'removal-sample', 'bench-depth',
                 'disk', 'min-cpu-cores'):
        run.add_argument('--' + name, required=True, type=positive_int, help='正の整数')
    for name in ('random-moves', 'seed-base', 'seed-generation-step', 'seed-book-step', 'train-seed',
                 'removal-seed'):
        run.add_argument('--' + name, required=True, type=nonnegative_int, help='0以上の整数')
    for name in ('max-price', 'min-duration-days', 'min-reliability', 'min-inet-up', 'min-disk', 'book-minutes',
                 'deadline-margin', 'training-wait-hours', 'idle-limit-hours', 'poll-seconds', 'book-stall-minutes',
                 'ssh-failure-minutes', 'lr-linear'):
        run.add_argument('--' + name, required=True, type=positive_float, help='正の有限値')
    run.add_argument('--window', required=True, type=int, choices=(1, 2), help='学習に使う教師の世代数')
    run.add_argument('--validation-interval', required=True, type=int, help='0以上の整数。0は途中検証なし')
    run.add_argument('--fit', required=True, choices=('yes', 'no'), help='線形の経路を補正するか')
    run.add_argument('--fit-tau', nargs='+', type=positive_float, help='補正する場合に必要な1個以上の正の有限値')
    run.add_argument('--fit-seed', type=nonnegative_int, help='補正する場合に必要な0以上の整数')
    run.add_argument('--fit-threads', type=positive_int, help='補正する場合に必要な正の整数')
    run.add_argument('--lambda', dest='mix', required=True, type=float, help='探索値の割合。0以上1以下')
    run.add_argument('--export-epoch', required=True, choices=('best', 'last'),
                     help='nnue.binへ書き出すエポック。bestは検証損失が最良、lastは最後に完了したエポック。'
                          '生成機でlastを使うには、前の世代のコミットの学習器が--export-epochに対応する必要がある')
    run.add_argument('--offer', type=positive_int, help='新しく借りる出品ID。通常の再開では省略する')
    run.add_argument('--train-place', required=True, choices=('generator', 'local'),
                     help='学習・診断・補正をgeneratorでは生成機、localでは作業機で行う。障害後の作業機での再開はlocalを指定する')
    args = parser.parse_args(argv)
    prefix = args.ref_prefix
    if (not prefix.startswith('refs/') or prefix.endswith('/')
            or any(not part or part.startswith('.') or part.endswith('.lock') for part in prefix.split('/'))
            or '..' in prefix or '@{' in prefix
            or any(character.isspace() for character in prefix)
            or re.search(r'[\x00-\x20\x7f~^:?*\[\\]', prefix)):
        parser.error('ref-prefixはrefs/で始まり、末尾に/のない有効なGit参照の接頭辞にしてください')
    for name in ('fit_tau', 'fit_seed', 'fit_threads'):
        if args.fit == 'yes' and not hasattr(args, name):
            parser.error('--fit yesには--' + name.replace('_', '-') + 'が必要です')
        if args.fit == 'no' and hasattr(args, name):
            parser.error('--fit noでは--' + name.replace('_', '-') + 'を指定できません')
    if args.first_generation > args.last_generation:
        parser.error('first-generationはlast-generation以下にしてください')
    if args.validation_interval < 0:
        parser.error('validation-intervalは0以上にしてください')
    if not math.isfinite(args.mix) or not 0 <= args.mix <= 1:
        parser.error('lambdaは0以上1以下の有限値にしてください')
    if args.calibration_books > args.books_per_generation:
        parser.error('calibration-booksはbooks-per-generation以下にしてください')
    if args.min_reliability > 1:
        parser.error('min-reliabilityは1以下にしてください')
    if (args.seed_base + args.seed_generation_step * args.last_generation
            + args.seed_book_step * (args.books_per_generation - 1) >= 2**64):
        parser.error('計算したシードが2^64以上です')
    try:
        re.compile(args.cpu_pattern)
    except re.error as error:
        parser.error(f'cpu-patternの正規表現が不正です: {error}')
    return args


def main(argv: list[str]) -> int:
    args = arguments(argv)
    args.run_dir.mkdir(parents=True, exist_ok=True)
    # ディレクトリ自体をロックし、再開状態を表すファイルを増やさない。
    descriptor = os.open(args.run_dir, os.O_RDONLY)
    previous = {}

    def interrupt(signum: int, frame: Any) -> None:
        for sig in (signal.SIGTERM, signal.SIGINT):
            signal.signal(sig, signal.SIG_IGN)
        raise Interrupted(signum)

    try:
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        previous = {sig: signal.signal(sig, interrupt) for sig in (signal.SIGTERM, signal.SIGINT)}
        return Chain(args).run()
    except BlockingIOError:
        print('同じrun-dirの世代反復が実行中です', file=sys.stderr)
        return 1
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        os.close(descriptor)


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
