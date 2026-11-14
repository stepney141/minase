#!/usr/bin/env python3
"""フェーズ1の制御スクリプトとprobeを、偽の口座、生成機、時計で検証する。

使い方: python3 -m unittest discover -s scripts/vast -p 'test_teacher_generation.py'
期待値はphase1-brief.mdの15項目と設計書の停止判定・移行手順に基づく。
番号範囲、出品の境界、公開中の回収、移行、費用と再起動、終了経路を検査する。
プロセス起動を禁止し、外部コマンドの契約だけを実ファイルに作用する偽物で代行する。
"""

import ast
from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime, timezone
import fcntl
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common
import generation_chain as chain
import teacher_generation as teacher
import teacher_generation_probe as probe
from test_generation_chain import Clock, write_book, inspect_book


def values(root: Path, clock: Clock) -> dict:
    """全引数を明示し、試験の条件を実装の既定値に依存させない。"""
    return {'run-dir': str(root / 'run'), 'repo': str(root / 'repo'), 'ref': 'refs/heads/master',
            'commit': 'a' * 40, 'minase-bin': str(root / 'inspect'), 'generation': '3',
            'first-book': '2', 'last-book': '3', 'seed-base': '9700000000', 'seed-book-step': '100000',
            'games': '10', 'nodes': '20000', 'random-moves': '0', 'max-ply': '4000', 'hash-mb': '16',
            'chunk-games': '3', 'rental': 'interruptible', 'max-price': '3', 'max-cost': '100',
            'max-rentals': '4',
            'deadline': datetime.fromtimestamp(clock.time() + 7200, timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')}


def argv(options: dict) -> list[str]:
    return ['run', *[part for key, value in options.items() for part in ('--' + key, str(value))]]


def offer(number: int = 42, **updates: object) -> dict:
    """フェーズ0の下限をちょうど満たす出品を作る。"""
    return {'id': number, 'machine_id': number + 100, 'cpu_arch': 'amd64', 'gpu_frac': 1,
            'cpu_cores': 256, 'cpu_cores_effective': 256, 'cpu_name': 'AMD EPYC 7B13 64-Core Processor',
            'cpu_ram': 70400, 'direct_port_count': 1, 'rentable': True, 'duration': 7200,
            'dph_total': 1.0, 'min_bid': 0.4, **updates}


class Commands:
    """口座と各生成機のディスクを保持し、公開・停止・通信失敗を再現する。"""

    def __init__(self, root: Path, clock: Clock) -> None:
        self.root, self.clock = root, clock
        self.calls = []
        self.offers = [offer(), offer(43, min_bid=0.5)]
        self.instances = {}
        self.machines = {}
        self.scenarios = {}
        self.creations = []
        self.jobs = []
        self.current = None
        self.price = 0.8
        self.cpus = {}
        self.authentication = False
        self.deny_after_start = False
        self.fail_query_after_start = False
        self.fail_query_after_create = False
        self.fail_destroy = False
        self.create_failures = []
        self.remote_failures = {}
        self.bundle_failure = None
        self.bad_header = {}
        self.bad_log = {}
        self.ancestor = True
        self.inspect_failure = False
        self.on_start = None
        self.on_create = None
        self.on_destroy = None
        self.sent_bytes = 1000
        self.received_bytes = 2000

    def result(self, command: list, output: object = '', code: int = 0, errors: str = '') -> subprocess.CompletedProcess:
        result = subprocess.CompletedProcess(command, code, output, errors)
        result.check_returncode()
        return result

    def local(self, path: str) -> Path:
        return self.machines[self.current]['root'] / path.removeprefix('/workspace/job/').rstrip('/')

    def active(self) -> bool:
        return self.current in self.instances and self.current in self.machines and self.machines[self.current]['started']

    def publish(self) -> None:
        machine = self.machines[self.current]
        job = json.loads((machine['root'] / 'job.json').read_text())
        for number in range(job['target_books']):
            if number in job['skip_books']:
                continue
            fields = {'rule_set': 'L0,P0,R1,E0', 'generation_commit': job['commit'],
                      'network_checksum': '12' * 32, 'teacher_search_budget': job['nodes'],
                      'seed': job['generation_seed'] + job['book_step'] * number,
                      'game_count': 9, 'entry_count': 120} | self.bad_header
            path = machine['root'] / f'teacher/gen-{job["generation"]}/book-{number}.mnsd'
            write_book(path, fields)
            summary = {'games_completed': 10, 'nodes': 20000, 'max_ply': 4000, 'hash_mb': 16,
                       'random_moves_max': 0, 'games_discarded_max_ply': 1,
                       'recorded_positions': 120, 'chunks_reused': machine['reused'].get(number, 0)} | self.bad_log
            log = machine['root'] / f'logs/gen-{job["generation"]}/book-{number}.log'
            log.write_text('   Finished `release` profile\n'
                           'progress: elapsed_seconds=20.0 finished_games=10 committed_games=10 chunks=4/4\n'
                           + ''.join(f'{key}: {value}\n' for key, value in summary.items()))

    def chunks(self) -> None:
        directory = self.local('/workspace/job/work/gen-3/book-2')
        directory.mkdir(parents=True, exist_ok=True)
        for name in ('config.json', 'chunk-00000.json', 'chunk-00000.mnsd', 'chunk-00003.json',
                     'chunk-00003.mnsd', 'chunk-00001.mnsd', 'chunk-00002.json', 'teacher.mnsd',
                     'chunk-00004.json.tmp', '.selfplay_generate.lock'):
            (directory / name).write_text(name)
        directory.with_suffix('.log').write_text(
            '   Finished `release` profile\n'
            'progress: elapsed_seconds=10.0 finished_games=4 committed_games=0 chunks=0/4\n'
            'progress: elapsed_seconds=20.0 finished_games=8 committed_games=4 chunks=2/4\n')

    def run(self, command: list[str], seconds: float, **kwargs: object) -> subprocess.CompletedProcess:
        self.calls.append((list(command), seconds, dict(kwargs)))
        name = Path(command[0]).name
        if name == 'git':
            if command[3] == 'merge-base':
                return self.result(command, code=0 if self.ancestor else 1)
            if command[3] == 'show':
                return self.result(command, b'MNUE' + bytes(140) + bytes.fromhex('12' * 32))
            if command[3:5] == ['bundle', 'create']:
                if self.bundle_failure is not None:
                    raise self.bundle_failure
                Path(command[5]).write_bytes(b'bundle')
                return self.result(command)
            raise AssertionError(command)
        if name == 'vastai':
            if self.authentication or (self.active() and self.deny_after_start):
                return self.result(command, '', errors='{"status_code": 401, "msg": "Two Factor Authentication"}\n')
            action = command[1:3]
            if action == ['search', 'offers']:
                return self.result(command, json.dumps(self.offers))
            if action == ['create', 'instance']:
                assert not self.instances, '旧機の削除前に借りてはいけません'
                self.creations.append(command)
                if self.on_create is not None:
                    self.on_create(len(self.creations))
                failure = self.create_failures.pop(0) if self.create_failures else None
                if isinstance(failure, Exception):
                    raise failure
                if failure in (401, 429):
                    return self.result(command, errors=json.dumps({'status_code': failure}))
                if failure == 'absent':
                    raise subprocess.TimeoutExpired(command, seconds)
                instance_id = 16 + len(self.creations)
                self.current = instance_id
                directory = self.root / f'machine-{instance_id}'
                directory.mkdir()
                selected = next(item for item in self.offers if str(item['id']) == command[3])
                self.instances[instance_id] = {
                    'id': instance_id, 'label': command[command.index('--label') + 1],
                    'actual_status': 'running', 'intended_status': 'running', 'cur_state': 'running',
                    'next_state': 'running', 'status_msg': '観測した状態', 'public_ipaddr': '192.0.2.1',
                    'ports': {'22/tcp': [{'HostPort': '2222'}]}, 'dph_total': self.price,
                    'inet_up_cost': 3.0, 'inet_down_cost': 5.0}
                self.machines[instance_id] = {'root': directory, 'started': False, 'probes': 0,
                                             'offer': selected, 'scenario': self.scenarios.get(len(self.creations), 'complete'),
                                             'reused': {}}
                if failure == 'timeout':
                    raise subprocess.TimeoutExpired(command, seconds)
                if failure == 'invalid':
                    return self.result(command, '')
                if failure == 'error_after_create':
                    raise RuntimeError('作成後にCLIが失敗しました')
                if self.machines[instance_id]['scenario'] == 'loading':
                    self.instances[instance_id]['actual_status'] = 'loading'
                return self.result(command, json.dumps({'new_contract': instance_id}))
            if action == ['show', 'instances']:
                if self.creations and self.fail_query_after_create:
                    return self.result(command, '{')
                if self.active() and self.fail_query_after_start:
                    return self.result(command, '{')
                if self.active():
                    machine = self.machines[self.current]
                    if machine['scenario'] == 'migrate' and machine['probes'] >= 1:
                        self.instances[self.current]['actual_status'] = 'stopped'
                return self.result(command, json.dumps(list(self.instances.values())))
            if action == ['destroy', 'instance']:
                if self.on_destroy is not None:
                    self.on_destroy()
                if not self.fail_destroy:
                    self.instances.pop(int(command[3]), None)
                return self.result(command)
            raise AssertionError(command)
        if name == 'ssh':
            words = shlex.split(command[-1])
            machine = self.machines[self.current]
            scenario = machine['scenario']
            if words == ['true']:
                return self.result(command, code=255 if scenario == 'no_ssh' else 0)
            operation = 'cpu-info' if words[-1] == 'cpu-info' else words[0]
            key = (machine['offer']['id'], operation)
            if key in self.remote_failures:
                raise self.remote_failures[key]
            if words[0] == 'mkdir':
                for path in words[2:]:
                    self.local(path).mkdir(parents=True, exist_ok=True)
                return self.result(command)
            if words[-1] == 'cpu-info':
                info = self.cpus.get(machine['offer']['id'], {'nproc': 256, 'cpu_limit': 230.4})
                return self.result(command, info if isinstance(info, str) else json.dumps(info))
            if words[0] == 'touch':
                assert self.local('/workspace/job/job.json').is_file()
                self.local(words[1]).touch()
                machine['started'] = True
                job = json.loads(self.local('/workspace/job/job.json').read_text())
                self.jobs.append(job)
                for number in range(job['target_books']):
                    directory = self.local(f'/workspace/job/work/gen-3/book-{number}')
                    machine['reused'][number] = len(list(directory.glob('chunk-*.json')))
                if self.on_start is not None:
                    self.on_start()
                return self.result(command)
            if words[1].endswith('teacher_generation_probe.py'):
                machine['probes'] += 1
                count = machine['probes']
                if scenario == 'migrate':
                    if count == 1:
                        self.chunks()
                    else:
                        return self.result(command, code=255)
                elif scenario == 'probe_failure':
                    return self.result(command, code=255)
                elif scenario == 'vanishing' and count == 1:
                    self.chunks()
                elif scenario in ('complete', 'vanishing'):
                    self.publish()
                data = {'pid': 50, 'alive': scenario != 'missing',
                        'exit_status': '7' if scenario == 'exit' else None,
                        'cpu_seconds': 10 if scenario == 'stalled' else count * 10,
                        'bytes': 10000}
                return self.result(command, json.dumps(data))
            raise AssertionError(command)
        if name == 'rsync':
            machine = self.machines[self.current]
            paths = command[command.index('--') + 1:]
            receiving = paths[0].startswith('root@')
            if receiving:
                remote = paths[0].split(':', 1)[1]
                source, destination = self.local(remote), Path(paths[-1])
                for path in source.rglob('*'):
                    if (not path.is_file() or path.name.endswith('.tmp')
                            or path.name in ('.selfplay_generate.lock', 'teacher.mnsd')):
                        continue
                    target = destination / path.relative_to(source)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(path, target)
                code = 0
                if machine['scenario'] == 'vanishing' and machine['probes'] == 1 and '/work/' in remote:
                    code = 24
                if machine['scenario'] == 'rsync_failure':
                    code = 23
                return self.result(command, f'Total bytes received: {self.received_bytes:,}\n', code)
            destination = paths[-1].split(':', 1)[1]
            key = (machine['offer']['id'], 'upload')
            if key in self.remote_failures:
                raise self.remote_failures[key]
            for name in paths[:-1]:
                source = Path(name)
                target = self.local(destination)
                if destination.endswith('/'):
                    target = target / source.name
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, target)
            return self.result(command, f'Total bytes sent: {self.sent_bytes:,}\n')
        if name == 'inspect':
            if self.inspect_failure:
                return self.result(command, code=7)
            fields = inspect_book(Path(command[-1]))
            return self.result(command, ''.join(f'{key}: {value}\n' for key, value in fields.items()))
        raise AssertionError(command)


class TeacherTests(unittest.TestCase):
    """15項目の外部契約と終了時の結果を確かめる。"""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.clock = Clock()
        self.options = values(self.root, self.clock)
        self.commands = Commands(self.root, self.clock)
        self.output, self.errors = io.StringIO(), io.StringIO()
        self.enterContext(redirect_stdout(self.output))
        self.enterContext(redirect_stderr(self.errors))
        self.enterContext(patch.object(teacher, 'time', self.clock))
        self.enterContext(patch.object(common, 'time', self.clock))
        self.enterContext(patch.object(common, 'run_timed', side_effect=self.commands.run))
        self.enterContext(patch.object(subprocess, 'Popen', side_effect=AssertionError('実コマンドは禁止です')))
        self.enterContext(patch.object(subprocess, 'run', side_effect=AssertionError('実コマンドは禁止です')))

    def execute(self) -> int:
        return teacher.main(argv(self.options))

    def state(self) -> dict:
        return json.loads((self.root / 'run/state.json').read_text())

    def events(self) -> list[dict]:
        return [json.loads(line) for line in (self.root / 'run/events.jsonl').read_text().splitlines()]

    def calls(self) -> list[list[str]]:
        return [command for command, _, _ in self.commands.calls]

    def test_normal_exit_collects_again_deletes_and_chain_reads_accepted_books(self) -> None:
        """事例3・10・11: 範囲外を飛ばし、受理した本を連続生成の入力にする。"""
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        job = self.commands.jobs[0]
        self.assertEqual(job['skip_books'], [0, 1])
        self.assertEqual(job['target_books'], 4)
        self.assertEqual(job['concurrency'], 256)
        self.assertEqual(job['generation_seed'], 9700000000)
        self.assertEqual(job['book_step'], 100000)
        self.assertEqual(job['reference'], 'refs/heads/master')
        self.assertFalse(self.commands.instances)
        self.assertEqual(self.commands.machines[17]['probes'], 2)
        args = teacher.arguments(argv(self.options))
        args.seed_generation_step = 0
        runner = chain.Chain(args)
        self.assertEqual(runner.books(3, 'a' * 40, '12' * 32), {2: 120, 3: 120})
        self.assertIsNone(self.state()['current'])
        self.assertTrue(self.state()['rentals'][0]['deleted_at'])
        events = [item['event'] for item in self.events()]
        self.assertEqual(events.count('build_completed'), 1)
        self.assertEqual(events.count('generation_resumed'), 1)
        self.assertEqual(events[-2:], ['deleted', 'finished'])
        self.assertFalse(list((self.root / 'run/remote/work').glob('book-*')))

    def test_rsync_24_accepts_published_books_on_next_collection(self) -> None:
        """事例1: 公開途中の消失は停止理由にせず、次の60秒の回収で受理する。"""
        self.commands.scenarios[1] = 'vanishing'
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.clock.sleeps, [60])
        self.assertEqual(len(self.commands.creations), 1)
        self.assertEqual(self.commands.machines[17]['probes'], 3)
        self.assertEqual(len(list((self.root / 'run/gen-3/teacher').glob('*.mnsd'))), 2)

    def test_migration_cleans_orphans_and_uploads_pairs_before_started(self) -> None:
        """事例2: 削除後に片側の塊を消し、再開条件と完全な組を送ってから起動する。"""
        self.commands.scenarios[1] = 'migrate'
        def check_cleaned(attempt: int) -> None:
            if attempt == 2:
                work = self.root / 'run/remote/work/book-2'
                self.assertFalse((work / 'chunk-00001.mnsd').exists())
                self.assertFalse((work / 'chunk-00002.json').exists())
        self.commands.on_create = check_cleaned
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(len(self.commands.creations), 2)
        destination = self.commands.machines[18]['root'] / 'work/gen-3/book-2'
        self.assertEqual({path.name for path in destination.iterdir()},
                         {'config.json', 'chunk-00000.json', 'chunk-00000.mnsd',
                          'chunk-00003.json', 'chunk-00003.mnsd'})
        self.assertEqual((destination / 'chunk-00000.mnsd').read_text(), 'chunk-00000.mnsd')
        archive = self.root / 'run/remote/archive/rental-1/book-2.log'
        self.assertIn('finished_games=8', archive.read_text())
        record = self.state()['rentals'][1]
        self.assertEqual(record['sent_chunks'], {'2': 2})
        self.assertEqual(record['sent_chunk_games'], {'2': 4})
        log = self.root / 'run/gen-3/logs/book-2.log'
        self.assertIn('chunks_reused: 2', log.read_text())
        events = self.events()
        loss = next(item for item in events if item['event'] == 'loss_estimate')
        self.assertEqual(loss['estimated_games'], 132)
        self.assertEqual(loss['observation']['collected_games'], 4)
        self.assertEqual(loss['observation']['finished_games'], 8)
        self.assertEqual(loss['observation']['chunks'], [2, 4])
        stopped = next(item for item in events if item['event'] == 'stopped')
        self.assertEqual(stopped['observed']['status_msg'], '観測した状態')
        calls = self.calls()
        second_create = [i for i, c in enumerate(calls) if c[:3] == ['vastai', 'create', 'instance']][1]
        deleted = next(i for i, c in enumerate(calls) if c[:4] == ['vastai', 'destroy', 'instance', '17'])
        self.assertLess(deleted, second_create)
        uploads = [i for i, c in enumerate(calls) if c[0] == 'rsync' and not c[-2].startswith('root@')
                   and i > second_create]
        started = next(i for i, c in enumerate(calls) if i > second_create and c[-1] == 'touch /workspace/job/started')
        self.assertTrue(all(index < started for index in uploads))
        self.assertTrue(calls[uploads[-1]][-1].endswith('/job.json'))

    def test_unconfirmed_deletion_never_creates_replacement(self) -> None:
        """事例4: 削除要求が成功しても一覧に残る旧機を置いたまま借り直さない。"""
        self.commands.scenarios[1] = 'migrate'
        self.commands.fail_destroy = True
        self.assertEqual(self.execute(), 1)
        self.assertEqual(len(self.commands.creations), 1)
        self.assertIn('17', self.errors.getvalue())
        self.assertEqual(self.state()['current'], 1)
        self.assertIsNone(self.state()['rentals'][0]['deleted_at'])

    def test_ssh_timeout_and_running_timeout_delete_then_try_another_offer(self) -> None:
        """事例5: SSHの5分と稼働の10分を別に測り、失敗した試行も借用回数へ数える。"""
        self.commands.scenarios[1] = 'no_ssh'
        self.commands.scenarios[2] = 'loading'
        self.commands.offers.append(offer(44, min_bid=0.6))
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(sum(self.clock.sleeps), 900)
        self.assertEqual(self.state()['rental_count'], 3)
        self.assertEqual([int(c[3]) for c in self.commands.creations], [42, 43, 44])
        self.assertFalse(self.commands.instances)

    def test_cost_reserve_uses_instance_rate_and_final_collection(self) -> None:
        """事例6: 検索価格で借りても、稼働中は作成直後の単価の30分分を残す。"""
        self.options['max-cost'] = '2'
        self.commands.price = 2
        self.commands.on_start = lambda: self.clock.sleep(1800)
        self.assertEqual(self.execute(), 1)
        self.assertIn('30分分', self.errors.getvalue())
        self.assertEqual(self.commands.machines[17]['probes'], 1)
        self.assertEqual(self.state()['rentals'][0]['dph_total'], 2)
        self.assertFalse(self.commands.instances)

    def test_authentication_expiry_during_status_query_reports_remaining_id(self) -> None:
        """事例7: 認証切れでは移行せず、削除できなかったIDを残す。"""
        self.commands.deny_after_start = True
        self.assertEqual(self.execute(), 1)
        self.assertEqual(len(self.commands.creations), 1)
        self.assertIn('状態不明', self.errors.getvalue())
        self.assertIn('生成機 17', self.errors.getvalue())
        self.assertIsNone(self.state()['rentals'][0]['deleted_at'])
        self.assertTrue(list((self.root / 'run/gen-3/teacher').glob('*.mnsd')))

    def test_query_malformed_response_does_not_trigger_migration(self) -> None:
        self.commands.fail_query_after_start = True
        self.assertEqual(self.execute(), 1)
        self.assertEqual(len(self.commands.creations), 1)
        self.assertIn('状態不明', self.errors.getvalue())
        self.assertFalse(self.commands.instances)

    def test_initial_authentication_failure_does_not_rent(self) -> None:
        self.commands.authentication = True
        self.assertEqual(self.execute(), 1)
        self.assertFalse(self.commands.creations)

    def test_worker_and_communication_failures_stop_without_migration(self) -> None:
        """事例8: 終了・消失・CPU停滞と回収の30分失敗は、機械を替えず終了する。"""
        for scenario, elapsed, message in (
                ('exit', 0, '終了コード'), ('missing', 0, '作業プロセスが見つかりません'),
                ('stalled', 600, 'CPU時間が10分間'), ('probe_failure', 1800, '回収の失敗が30分'),
                ('rsync_failure', 1800, '回収の失敗が30分')):
            with self.subTest(scenario=scenario):
                self.options['run-dir'] = str(self.root / scenario)
                self.options['deadline'] = datetime.fromtimestamp(
                    self.clock.time() + 7200, timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
                before = len(self.commands.creations)
                self.commands.scenarios[before + 1] = scenario
                start = self.clock.time()
                self.assertEqual(self.execute(), 1, self.errors.getvalue())
                self.assertEqual(self.clock.time() - start, elapsed)
                self.assertEqual(len(self.commands.creations), before + 1)
                self.assertFalse(self.commands.instances)
                self.assertIn(message, self.errors.getvalue())

    def test_existing_accepted_book_is_skipped_without_reinspection(self) -> None:
        """保存した受け入れ済みの結果は読み直さず、skip_booksへ加える。"""
        path = self.root / 'run/gen-3/teacher/book-2.mnsd'
        path.parent.mkdir(parents=True)
        path.write_bytes('受け入れ済みの本'.encode())
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.commands.jobs[0]['skip_books'], [0, 1, 2])
        self.assertEqual(path.read_bytes(), '受け入れ済みの本'.encode())
        inspected = [c[-1] for c in self.calls() if Path(c[0]).name == 'inspect']
        self.assertEqual(inspected, [str(self.root / 'run/remote/teacher/book-3.mnsd')])

    def test_unrecorded_instances_are_all_deleted_and_unknown_cost_is_not_zeroed(self) -> None:
        """記録外の機械をすべて削除し、課金履歴の欠落を0米ドルで補わない。"""
        self.commands.instances = {90: {'id': 90, 'label': 'minase-teacher-run-90'},
                                   91: {'id': 91, 'label': 'minase-teacher-run-91'}}
        self.assertEqual(self.execute(), 1)
        self.assertFalse(self.commands.instances)
        self.assertFalse(self.commands.creations)
        self.assertEqual(self.state()['unaccounted_instances'], [90, 91])
        self.assertEqual(self.execute(), 1)
        self.assertFalse(self.commands.creations)

    def test_signal_while_deleting_unrecorded_instances_still_deletes_all(self) -> None:
        self.commands.instances = {90: {'id': 90, 'label': 'minase-teacher-run-90'},
                                   91: {'id': 91, 'label': 'minase-teacher-run-91'}}
        def interrupt_once() -> None:
            self.commands.on_destroy = None
            os.kill(os.getpid(), signal.SIGTERM)
        self.commands.on_destroy = interrupt_once
        self.assertEqual(self.execute(), 143)
        self.assertFalse(self.commands.instances)
        self.assertEqual(self.state()['unaccounted_instances'], [90, 91])
        self.assertFalse(self.commands.creations)

    def test_budget_arguments_and_count_survive_restart(self) -> None:
        """事例9: 同じ実行の再起動で、期限、費用見込み、試行回数を初期化しない。"""
        self.options['max-rentals'] = '1'
        self.commands.scenarios[1] = 'exit'
        self.assertEqual(self.execute(), 1)
        first = self.state()
        self.clock.now += 120
        self.assertEqual(self.execute(), 1)
        self.assertEqual(len(self.commands.creations), 1)
        self.assertEqual(self.state()['rentals'], first['rentals'])
        self.assertEqual(self.state()['rental_count'], 1)
        saved = json.loads((self.root / 'run/run.json').read_text())
        for key, changed in (('max-cost', '101'), ('max-rentals', '2'),
                             ('deadline', '2034-01-01T00:00:00Z'), ('nodes', '1')):
            with self.subTest(key=key):
                original = self.options[key]
                self.options[key] = changed
                self.assertEqual(self.execute(), 1)
                self.assertIn(key.replace('-', '_'), self.errors.getvalue())
                self.options[key] = original
        self.assertEqual(json.loads((self.root / 'run/run.json').read_text()), saved)
        runner = teacher.TeacherGeneration(teacher.arguments(argv(self.options)), self.clock)
        runner.initialize()
        rental = first['rentals'][0]
        expected = ((rental['deleted_at'] - rental['created_at']) * 0.8 / 3600
                    + rental['sent_bytes'] * 5 / 1e9 + rental['received_bytes'] * 3 / 1e9)
        self.assertAlmostEqual(runner.cost(), expected)

    def test_saved_current_is_deleted_before_next_rental(self) -> None:
        self.commands.scenarios[1] = 'exit'
        self.commands.fail_destroy = True
        self.assertEqual(self.execute(), 1)
        mark = len(self.commands.calls)
        self.commands.fail_destroy = False
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        later = self.calls()[mark:]
        destroy = next(i for i, c in enumerate(later) if c[:3] == ['vastai', 'destroy', 'instance'])
        create = next(i for i, c in enumerate(later) if c[:3] == ['vastai', 'create', 'instance'])
        self.assertLess(destroy, create)
        self.assertEqual(self.state()['rental_count'], 2)

    def test_exception_and_signals_collect_and_delete(self) -> None:
        """事例11: 本当のシグナルも例外と同じ終了手順を通り、128+番号を返す。"""
        for sig in (signal.SIGTERM, signal.SIGINT):
            with self.subTest(sig=sig):
                self.options['run-dir'] = str(self.root / f'run-{sig}')
                self.commands.on_start = lambda: os.kill(os.getpid(), sig)
                self.assertEqual(self.execute(), 128 + sig, self.errors.getvalue())
                machine = self.commands.machines[self.commands.current]
                self.assertEqual(machine['probes'], 1)
                self.assertFalse(self.commands.instances)
        self.commands.on_start = None
        self.options['run-dir'] = str(self.root / 'run-error')
        self.commands.inspect_failure = True
        self.assertEqual(self.execute(), 1)
        machine = self.commands.machines[self.commands.current]
        self.assertEqual(machine['probes'], 2)
        self.assertFalse(self.commands.instances)

    def test_bad_cpu_assignments_are_deleted_and_machine_ids_excluded(self) -> None:
        """事例12: cpusetの差と90%未満の上限を拒否し、同じmachine_idを選ばない。"""
        self.commands.cpus = {42: {'nproc': 255, 'cpu_limit': None},
                              43: {'nproc': 256, 'cpu_limit': 230.39}}
        self.commands.offers.extend([offer(44, machine_id=142, min_bid=0.4), offer(45, min_bid=0.7)])
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual([int(c[3]) for c in self.commands.creations], [42, 43, 45])
        self.assertEqual(self.state()['excluded_machine_ids'], [142, 143])
        self.assertEqual(len(self.commands.jobs), 1)

    def test_bid_and_fixed_machine_settings_are_sent(self) -> None:
        """事例13: 最低入札額と固定のイメージ、ディスク、自己停止期限を渡す。"""
        self.assertEqual(self.execute(), 0)
        create = self.commands.creations[0]
        for key, expected in (('--bid_price', '0.4'), ('--image', 'rust:1.98.0-bookworm'), ('--disk', '30')):
            self.assertEqual(create[create.index(key) + 1], expected)
        search = next(c for c in self.calls() if c[:3] == ['vastai', 'search', 'offers'])
        self.assertEqual(search[search.index('--type') + 1], 'bid')
        self.assertEqual(search[search.index('--limit') + 1], '1000')
        self.assertIn('verified=any', search[3].split())
        config = json.loads((self.commands.machines[17]['root'] / 'worker.json').read_text())
        self.assertEqual(config, {'idle_limit_seconds': 1800})
        start = (self.root / 'run/onstart.sh').read_text()
        self.assertLessEqual(len(start), 4048)
        self.assertLess(start.index('echo $$ > /workspace/job/worker.pid'), start.index('until [['))
        self.assertIn('exec python3 /workspace/job/generation_worker.py run', start)
        self.assertNotIn('@DEADLINE@', start)

    def test_bid_is_rounded_up_to_four_decimals_and_recorded(self) -> None:
        for minimum, expected in ((0.7999999999999999, 0.8), (0.80001, 0.8001), (0.8001, 0.8001)):
            with self.subTest(minimum=minimum):
                self.options['run-dir'] = str(self.root / f'bid-{minimum}')
                self.commands.offers = [offer(min_bid=minimum)]
                self.assertEqual(self.execute(), 0, self.errors.getvalue())
                create = self.commands.creations[-1]
                self.assertEqual(create[create.index('--bid_price') + 1], str(expected))
                state = json.loads((Path(self.options['run-dir']) / 'state.json').read_text())
                record = state['rentals'][0]
                self.assertEqual(record['bid_price'], expected)
                self.assertEqual(record['offer']['min_bid'], minimum)
                self.assertEqual(record['dph_total'], 0.8)

    def test_rounded_bid_must_not_exceed_maximum_price(self) -> None:
        self.options['max-price'] = '0.80001'
        self.commands.offers = [offer(min_bid=0.80001)]
        self.assertEqual(self.execute(), 1)
        self.assertFalse(self.commands.creations)

    def test_on_demand_uses_total_price_and_does_not_bid(self) -> None:
        self.options['rental'] = 'on-demand'
        self.commands.offers[0]['dph_total'] = 4
        self.commands.cpus[43] = {'nproc': 256, 'cpu_limit': None}
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.commands.creations[0][3], '43')
        self.assertNotIn('--bid_price', self.commands.creations[0])
        search = next(c for c in self.calls() if c[:3] == ['vastai', 'search', 'offers'])
        self.assertEqual(search[search.index('--type') + 1], 'on-demand')
        self.assertIn('verified=any', search[3].split())
        self.assertEqual(self.state()['rentals'][0]['rental'], 'on-demand')
        self.assertIsNone(self.state()['rentals'][0]['bid_price'])

    def test_create_timeout_recovers_by_label_without_duplicate_rental(self) -> None:
        for failure in ('timeout', 'invalid', 'error_after_create'):
            with self.subTest(failure=failure):
                self.options['run-dir'] = str(self.root / failure)
                before = len(self.commands.creations)
                self.commands.create_failures = [failure]
                self.assertEqual(self.execute(), 0, self.errors.getvalue())
                self.assertEqual(len(self.commands.creations), before + 1)
                state = json.loads((Path(self.options['run-dir']) / 'state.json').read_text())
                self.assertEqual(state['rental_count'], 1)

    def test_failed_creations_advance_offer_and_label_until_success(self) -> None:
        self.commands.create_failures = ['absent', common.VastError('すでに貸出中'), ValueError('作成が失敗')]
        self.commands.offers.extend([offer(44, min_bid=0.6), offer(45, min_bid=0.7)])
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.state()['rental_count'], 4)
        self.assertEqual([int(c[3]) for c in self.commands.creations], [42, 43, 44, 45])
        labels = [c[c.index('--label') + 1] for c in self.commands.creations]
        self.assertEqual(labels, [f'minase-teacher-run-{number}' for number in range(1, 5)])
        for record in self.state()['rentals'][:3]:
            self.assertIsNone(record['instance_id'])
            self.assertIsNotNone(record['deleted_at'])

    def test_create_authentication_or_rate_limit_exhaustion_does_not_change_offer(self) -> None:
        for code, attempts in ((401, 1), (429, 5)):
            with self.subTest(code=code):
                self.options['run-dir'] = str(self.root / f'error-{code}')
                self.commands.create_failures = [code] * attempts
                before = len(self.commands.creations)
                self.assertEqual(self.execute(), 1)
                self.assertEqual([c[3] for c in self.commands.creations[before:]], ['42'] * attempts)
                state = json.loads((Path(self.options['run-dir']) / 'state.json').read_text())
                self.assertEqual(state['rental_count'], 1)
                self.assertIn(str(code), self.errors.getvalue())
                self.assertFalse(self.commands.instances)

    def test_unknown_label_after_create_failure_prevents_another_rental(self) -> None:
        self.commands.create_failures = ['absent']
        self.commands.fail_query_after_create = True
        self.assertEqual(self.execute(), 1)
        self.assertEqual(len(self.commands.creations), 1)
        self.assertEqual(self.state()['rental_count'], 1)

    def test_next_offer_cost_and_deadline_are_checked_after_failed_creation(self) -> None:
        for limit in ('cost', 'deadline'):
            with self.subTest(limit=limit):
                self.options['run-dir'] = str(self.root / limit)
                self.commands.create_failures = ['absent']
                self.commands.offers[1]['dph_total'] = 101 if limit == 'cost' else 1
                if limit == 'deadline':
                    self.commands.on_create = lambda _: self.clock.sleep(5701)
                before = len(self.commands.creations)
                self.assertEqual(self.execute(), 1)
                self.assertEqual(len(self.commands.creations), before + 1)
                self.assertFalse(self.commands.instances)

    def test_retry_does_not_exceed_rental_limit(self) -> None:
        self.commands.create_failures = ['absent']
        self.options['max-rentals'] = '1'
        self.assertEqual(self.execute(), 1)
        self.assertEqual(len(self.commands.creations), 1)
        self.assertEqual(self.state()['rental_count'], 1)
        self.assertFalse(self.commands.instances)

    def test_preparation_failures_delete_before_renting_another_offer(self) -> None:
        failures = [('mkdir', OSError('SSHに接続できません')),
                    ('cpu-info', subprocess.CalledProcessError(255, ['ssh'])),
                    ('upload', subprocess.TimeoutExpired(['rsync'], 120)),
                    ('upload', subprocess.CalledProcessError(23, ['rsync'], output='Total bytes sent: 1000\n'))]
        for index, (operation, failure) in enumerate(failures):
            with self.subTest(operation=operation, failure=type(failure).__name__):
                self.options['run-dir'] = str(self.root / f'prepare-{index}')
                self.commands.remote_failures = {(42, operation): failure}
                before = len(self.commands.creations)
                first_call = len(self.commands.calls)
                self.assertEqual(self.execute(), 0, self.errors.getvalue())
                self.assertEqual([c[3] for c in self.commands.creations[before:]], ['42', '43'])
                state = json.loads((Path(self.options['run-dir']) / 'state.json').read_text())
                self.assertEqual(state['rental_count'], 2)
                self.assertEqual(state['excluded_machine_ids'], [])
                first, second = state['rentals']
                self.assertIsNotNone(first['deleted_at'])
                self.assertLessEqual(first['deleted_at'], second['created_at'])
                self.assertIn('準備', first['reason'])
                self.assertFalse(self.commands.machines[first['instance_id']]['started'])
                commands = self.calls()[first_call:]
                deletion = next(i for i, c in enumerate(commands) if c[:3] == ['vastai', 'destroy', 'instance'])
                confirmation = next(i for i, c in enumerate(commands) if i > deletion and
                                    c[:3] == ['vastai', 'show', 'instances'])
                creation = next(i for i, c in enumerate(commands) if c[:4] == ['vastai', 'create', 'instance', '43'])
                self.assertLess(deletion, confirmation)
                self.assertLess(confirmation, creation)
                self.assertFalse(self.commands.instances)

    def test_unreadable_cpu_info_deletes_and_changes_offer(self) -> None:
        for index, response in enumerate(('{', None, [], {}, {'nproc': 256})):
            with self.subTest(response=response):
                self.options['run-dir'] = str(self.root / f'cpu-info-{index}')
                self.commands.cpus = {42: response}
                before = len(self.commands.creations)
                self.assertEqual(self.execute(), 0, self.errors.getvalue())
                self.assertEqual([c[3] for c in self.commands.creations[before:]], ['42', '43'])
                self.assertFalse(self.commands.instances)

    def test_preparation_failure_requires_confirmed_deletion_before_renting(self) -> None:
        self.commands.remote_failures = {(42, 'upload'): OSError('転送できません')}
        self.commands.fail_destroy = True
        self.assertEqual(self.execute(), 1)
        self.assertEqual(len(self.commands.creations), 1)
        self.assertEqual(self.state()['rentals'][0]['instance_id'], 17)
        self.assertIsNone(self.state()['rentals'][0]['deleted_at'])
        self.assertIn('17', self.errors.getvalue())

    def test_local_bundle_failure_does_not_change_offer(self) -> None:
        self.commands.bundle_failure = subprocess.CalledProcessError(128, ['git', 'bundle', 'create'])
        self.assertEqual(self.execute(), 1)
        self.assertEqual(len(self.commands.creations), 1)
        self.assertFalse(self.commands.instances)

    def test_transfers_have_size_based_limits_stats_and_exclusions(self) -> None:
        """事例15: 送受信の両方向で大きさから上限を求め、転送量を保存する。"""
        self.assertEqual(self.execute(), 0)
        sent, received = 0, 0
        for command, seconds, _ in self.commands.calls:
            if command[0] != 'rsync':
                continue
            self.assertIn('--stats', command)
            paths = command[command.index('--') + 1:]
            if paths[0].startswith('root@'):
                self.assertAlmostEqual(seconds, 120.01)
                for pattern in ('*.tmp', '.selfplay_generate.lock', 'teacher.mnsd'):
                    self.assertIn(pattern, command)
                received += 2000
            else:
                self.assertGreater(seconds, 120)
                self.assertLess(seconds, 121)
                sent += 1000
        record = self.state()['rentals'][0]
        self.assertEqual(record['sent_bytes'], sent)
        self.assertEqual(record['received_bytes'], received)

    def test_pre_rental_limits_and_ancestor_fail_before_creation(self) -> None:
        for name, value in (('max-cost', '0.99'), ('deadline',
                datetime.fromtimestamp(self.clock.time() + 1499, timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'))):
            with self.subTest(name=name):
                self.options['run-dir'] = str(self.root / name)
                original = self.options[name]
                self.options[name] = value
                self.assertEqual(self.execute(), 1)
                self.assertFalse(self.commands.creations)
                self.options[name] = original
        self.options['run-dir'] = str(self.root / 'ancestor')
        self.commands.ancestor = False
        self.assertEqual(self.execute(), 1)
        self.assertFalse(self.commands.creations)

    def test_deadline_reserves_ten_minutes_and_does_not_transfer_without_time(self) -> None:
        self.commands.on_start = lambda: self.clock.sleep(6601)
        self.assertEqual(self.execute(), 1)
        self.assertIn('10分未満', self.errors.getvalue())
        self.assertEqual(self.commands.machines[17]['probes'], 1)
        self.assertFalse(any(c[0] == 'rsync' and c[-2].startswith('root@') for c in self.calls()))
        self.assertFalse(self.commands.instances)

    def test_exclusive_lock_prevents_any_external_operation(self) -> None:
        directory = self.root / 'run'
        directory.mkdir()
        descriptor = os.open(directory, os.O_RDONLY)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(self.execute(), 1)
            self.assertFalse(self.commands.calls)
        finally:
            os.close(descriptor)

    def test_every_argument_is_required_and_invalid_values_reject_before_rental(self) -> None:
        for key in self.options:
            with self.subTest(missing=key), self.assertRaises(SystemExit):
                teacher.main(argv({name: value for name, value in self.options.items() if name != key}))
        for key, value in (('commit', 'abc'), ('deadline', '2034-01-01'), ('first-book', '-1'),
                           ('last-book', '1'), ('generation', '-1'), ('max-price', 'nan'),
                           ('run-dir', 'relative'), ('games', '0'), ('seed-base', str(2**64 - 1))):
            with self.subTest(key=key), self.assertRaises(SystemExit):
                teacher.main(argv(self.options | {key: value}))
        self.assertFalse(self.commands.calls)


class DecisionTests(unittest.TestCase):
    """事例8の優先順位と、600秒・1800秒の境界を検証する。"""

    def test_table_order_and_time_boundaries(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            clock = Clock()
            args = teacher.arguments(argv(values(Path(directory), clock)))
            runner = teacher.TeacherGeneration(args, clock)
            record = {'number': 1, 'ssh_at': clock.time(), 'last_collection': None,
                      'nonrunning': 0, 'cpu_seconds': None, 'cpu_increased_at': None}
            runner.state = {'current': 1, 'rentals': [record]}
            alive = {'alive': True, 'exit_status': None, 'cpu_seconds': 1}
            running = {'actual_status': 'running'}
            stopped = {'actual_status': 'stopped'}
            self.assertEqual(runner.decision(stopped, None, False), 'continue')
            clock.now += 1800
            self.assertEqual(runner.decision(stopped, None, False), 'stopped')
            self.assertEqual(runner.decision(running, alive | {'exit_status': '7', 'alive': False}, False),
                             'communication_failure')
            self.assertEqual(record['nonrunning'], 0)
            record['last_collection'] = clock.time()
            self.assertEqual(runner.decision(running, alive | {'exit_status': '0', 'alive': False}, True), 'worker_exit')
            self.assertEqual(runner.decision(running, alive | {'alive': False}, True), 'worker_missing')
            self.assertEqual(runner.decision(running, alive, True), 'continue')
            clock.now += 599
            self.assertEqual(runner.decision(running, alive, True), 'continue')
            clock.now += 1
            self.assertEqual(runner.decision(running, alive, True), 'cpu_stalled')
            self.assertEqual(runner.decision(running, alive | {'cpu_seconds': 2}, True), 'continue')
            record['last_collection'] = clock.time()
            clock.now += 1799
            self.assertEqual(runner.decision(running, None, False), 'continue')
            clock.now += 1
            self.assertEqual(runner.decision(running, None, False), 'communication_failure')
            # 稼働していない照会でも、その回のprobeが通れば移行しない。
            record['nonrunning'] = 1
            self.assertEqual(runner.decision(None, alive | {'cpu_seconds': 3}, True), 'continue')

    def test_budget_and_deadline_boundaries(self) -> None:
        """借用前の超過と稼働中の到達を区別し、残り25分・10分の境界も守る。"""
        clock = Clock()
        args = teacher.arguments(argv(values(Path('/test'), clock)))
        runner = teacher.TeacherGeneration(args, clock)
        rental = {'number': 1, 'instance_id': 17, 'dph_total': 0.8, 'created_at': clock.time(),
                  'deleted_at': None, 'sent_bytes': 0, 'received_bytes': 0,
                  'inet_up_cost': 3, 'inet_down_cost': 5}
        runner.state = {'current': 1, 'rental_count': 1, 'rentals': [rental]}
        args.max_cost = 0.4
        with self.assertRaises(teacher.GenerationStop):
            runner.limits(None)
        args.max_cost = 0.40000001
        runner.limits(None)
        args.max_cost = 1
        runner.limits(offer())
        args.max_cost = 0.999
        with self.assertRaises(teacher.GenerationStop):
            runner.limits(offer())
        args.max_cost = 100
        runner.deadline = clock.time() + 1500
        runner.limits(offer())
        runner.deadline -= 1
        with self.assertRaises(teacher.GenerationStop):
            runner.limits(offer())
        runner.deadline = clock.time() + 600
        runner.limits(None)
        runner.deadline -= 1
        with self.assertRaises(teacher.GenerationStop):
            runner.limits(None)


class SelectionTests(unittest.TestCase):
    """事例13の全条件と、単価を探索速度で割る順位を検証する。"""

    def test_all_offer_conditions_and_price_per_search_speed(self) -> None:
        args = teacher.arguments(argv(values(Path('/test'), Clock())))
        valid = offer()
        failures = {'cpu_arch': 'arm64', 'gpu_frac': 0.5, 'cpu_cores_effective': 255,
                    'cpu_name': 'AMD EPYC 9654P 96-Core Processor', 'cpu_cores': 255,
                    'cpu_ram': 70399, 'direct_port_count': 0, 'rentable': False,
                    'duration': 7199, 'min_bid': 3.01}
        for key, value in failures.items():
            with self.subTest(key=key), self.assertRaises(teacher.GenerationStop):
                teacher.select_offer([valid | {key: value}], args, 7200, [])
        with self.assertRaises(teacher.GenerationStop):
            teacher.select_offer([valid], args, 7200, [142])
        newer = offer(43, cpu_name='AMD EPYC 9654 96-Core Processor', min_bid=0.45, dph_total=1.1)
        self.assertEqual(teacher.select_offer([valid, newer], args, 7200, [])['id'], 43)
        args.rental = 'on-demand'
        self.assertEqual(teacher.select_offer([valid, newer], args, 7200, [])['id'], 43)
        args.max_price = 1
        self.assertEqual(teacher.select_offer([valid, newer], args, 7200, [])['id'], 42)
        self.assertEqual(teacher.select_offer([valid], args, 7200, [])['id'], 42)


class ProbeTests(unittest.TestCase):
    """Linuxのstat欄と塊の仕様から、観測値を検証する。"""

    def test_session_cpu_includes_children_and_reaped_time_and_all_directory_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'worker.pid').write_text('50\n')
            proc = root / 'proc'
            proc.mkdir()
            def process(pid: int, session: int, ticks: tuple[int, int, int, int]) -> None:
                path = proc / str(pid)
                path.mkdir()
                # statの欄3から17。欄6がsession、欄14〜17がCPU時間。
                fields = ['S', '1', '50', str(session), '0', '0', '0', '0', '0', '0', '0', *map(str, ticks)]
                (path / 'stat').write_text(f'{pid} (command with ) spaces) ' + ' '.join(fields))
            process(50, 50, (100, 200, 300, 400))
            process(51, 50, (10, 20, 30, 40))
            process(90, 90, (10000, 10000, 10000, 10000))
            for name in ('work', 'teacher', 'logs'):
                path = root / name / 'gen-3'
                path.mkdir(parents=True)
                (path / 'file').write_bytes(b'abcd')
            (root / 'work/gen-3/file.tmp').write_bytes(b'abc')
            (root / 'logs/exit-status').write_text('7\n')
            self.assertEqual(probe.probe(root, proc, 3, 100),
                             {'pid': 50, 'alive': True, 'exit_status': '7', 'cpu_seconds': 11, 'bytes': 15})
            shutil.rmtree(proc / '50')
            shutil.rmtree(proc / '51')
            (root / 'logs/exit-status').unlink()
            result = probe.probe(root, proc, 3, 100)
            self.assertFalse(result['alive'])
            self.assertIsNone(result['exit_status'])
            self.assertIsNone(result['cpu_seconds'])

    def test_probe_uses_python_311_syntax_and_standard_library_imports(self) -> None:
        tree = ast.parse(Path(probe.__file__).read_text(), feature_version=(3, 11))
        modules = [node.module.split('.')[0] for node in ast.walk(tree) if isinstance(node, ast.ImportFrom)]
        modules += [alias.name.split('.')[0] for node in ast.walk(tree) if isinstance(node, ast.Import) for alias in node.names]
        self.assertTrue(set(modules) <= sys.stdlib_module_names)


if __name__ == '__main__':
    unittest.main()
