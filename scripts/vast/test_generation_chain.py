#!/usr/bin/env python3
"""指示書Bの世代反復と生成機の契約を、外部サービスを呼ばずに検証する。

使い方: python3 scripts/vast/test_generation_chain.py
本ファイルはgeneration_chain.py、generation_worker.pyと同じディレクトリに置く。
指示書の事例1〜14を、引数と出品の拒否、教師の受理、学習と検定の再開、
停止と削除、作業者の公開順序と自己停止に分けて検証する。
期待値は指示書の式とコマンド、およびtraining/records.rsのMNSD版2の形式に基づく。
外部コマンドは注入した実行関数で置き換え、時計も進めるだけにする。
"""

from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import re
import shlex
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common
import generation_chain as chain
import generation_worker as worker


def network(generation: int) -> bytes:
    """MNUEの144〜176バイトを世代ごとに区別できるようにする。"""
    return b'MNUE' + bytes(140) + bytes([generation]) * 32 + '重みの代用'.encode()


def header_fields(generation: int, number: int, entries: int) -> dict:
    """指示書のシードと、1本10局の試験条件を与える。"""
    return {'rule_set': 'L0,P0,R1,E0', 'generation_commit': str(generation - 1) * 40,
            'network_checksum': (bytes([generation - 1]) * 32).hex(),
            'teacher_search_budget': 20000, 'seed': 9000000000 + 10000000 * generation + 100000 * number,
            'game_count': 9, 'entry_count': entries}


def write_book(path: Path, fields: dict) -> None:
    """公開されたMNSD版2のヘッダを組み立てる。本文の復号は偽inspectの担当とする。"""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(struct.pack('<4sI32s40s32sB7sQQQQ', b'MNSD', 2, fields['rule_set'].encode(),
                                 fields['generation_commit'].encode(), bytes.fromhex(fields['network_checksum']),
                                 0, bytes(7), fields['teacher_search_budget'], fields['seed'],
                                 fields['game_count'], fields['entry_count']))


def inspect_book(path: Path) -> dict:
    """偽のinspectが本の固定ヘッダから表示する値を読む。"""
    data = path.read_bytes()
    return {'rule_set': data[8:40].rstrip(b'\0').decode(), 'generation_commit': data[40:80].decode(),
            'network_checksum': data[80:112].hex(), 'teacher_search_budget': int.from_bytes(data[120:128], 'little'),
            'seed': int.from_bytes(data[128:136], 'little'), 'game_count': int.from_bytes(data[136:144], 'little'),
            'entry_count': int.from_bytes(data[144:152], 'little'),
            'eligible_positions': int.from_bytes(data[144:152], 'little')}


def cli_values(root: Path) -> dict[str, str]:
    """必須条件を省略せずに与える。テストの既定値を本体へ持ち込まない。"""
    return {'run-dir': str(root / 'run'), 'worktree': str(root / 'tree'), 'match-root': str(root / 'matches'),
            'minase-bin': str(root / 'inspect'), 'match-runner': str(root / 'match'), 'trainer-bin': str(root / 'trainer'),
            'gen0-commit': '0' * 40, 'first-generation': '1', 'last-generation': '2',
            'positions-per-generation': '100', 'books-estimate': '2', 'games': '10', 'nodes': '20000',
            'random-moves': '0', 'max-ply': '4000', 'hash-mb': '16', 'seed-base': '9000000000',
            'seed-generation-step': '10000000', 'seed-book-step': '100000', 'max-books': '100',
            'calibration-books': '1', 'lambda': '0.75', 'feature-set': 'P', 'width': '256',
            'head': 'pairwise-l16-l32', 'batch': '16384', 'epochs': '50', 'patience': '5', 'train-seed': '1',
            'device': 'cuda', 'validation-interval': '1000', 'lr-schedule': 'cosine:3:0.1', 'train-threads': '8',
            'removal-sample': '10', 'removal-seed': '3', 'bench-depth': '5', 'stc-seed-base': '20261200000000',
            'stc-seed-step': '1000000', 'stc-each': 'time=10000+100', 'stc-concurrency': '16', 'stc-max-pairs': '3000',
            'offer': '42', 'image': 'rust:1.98.0-bookworm', 'disk': '50', 'max-price': '0.5',
            'min-cpu-cores': '384', 'cpu-pattern': 'EPYC.*9', 'min-duration-days': '5', 'min-reliability': '0.95',
            'min-inet-up': '100', 'min-disk': '50', 'book-minutes': '12', 'deadline-margin': '1.3',
            'training-wait-hours': '1.2', 'idle-limit-hours': '8', 'poll-seconds': '1',
            'book-stall-minutes': '0.05', 'ssh-failure-minutes': '0.05'}


def argv(values: dict[str, str]) -> list[str]:
    return ['run', *[part for key, value in values.items() for part in ('--' + key, value)]]


class Clock:
    """実時間を使わず、sleepの要求だけ経過させる。"""

    def __init__(self) -> None:
        self.now = 1000.0
        self.sleeps = []

    def monotonic(self) -> float:
        return self.now

    def time(self) -> float:
        return self.now + 2000000000

    def sleep(self, seconds: float) -> None:
        self.sleeps.append(seconds)
        self.now += seconds


class MatchProcess:
    """所定回数の状態確認後に検定ログを書いて終了する。"""

    def __init__(self, log: Path, output: str, code: int, polls: int) -> None:
        self.log, self.output, self.code, self.remaining = log, output, code, polls
        self.returncode = None
        self.written = False

    def poll(self) -> int | None:
        self.remaining -= 1
        if self.remaining <= 0:
            if not self.written:
                with self.log.open('a') as output:
                    output.write(self.output)
                self.written = True
            self.returncode = self.code
        return self.returncode


class Commands:
    """Git、Vast.ai、SSH、学習器の外部契約だけを代行する。"""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.remote = root / 'remote'
        self.remote.mkdir()
        self.calls = []
        self.processes = []
        self.refs = {}
        self.head = '0' * 40
        self.nets = {self.head: network(0)}
        self.next_generation = 1
        (root / 'tree/crates/minase/nets').mkdir(parents=True)
        (root / 'tree/crates/minase/nets/nnue.bin').write_bytes(network(0))
        self.instances = []
        self.offer = {'id': 42, 'gpu_frac': 1, 'cpu_cores_effective': 384, 'cpu_cores': 384,
                      'cpu_name': 'AMD EPYC 9R14', 'verification': 'verified', 'reliability2': 0.95,
                      'duration': 432000, 'direct_port_count': 1, 'inet_up': 100, 'disk_space': 50, 'dph_total': 0.5}
        self.cpu = {'nproc': 384, 'cpu_limit': 384}
        self.started = False
        self.stopped = False
        self.created = False
        self.deny_account_after_start = False
        self.generated = []
        self.jobs = []
        self.bad_header = {}
        self.bad_log = {}
        self.entries = 120
        self.stall = False
        self.worker_exit = False
        self.transfer_failures = 0
        self.missing_transfers = 0
        self.match_output = 'decision: H1\n'
        self.match_code = 0
        self.match_polls = 2
        self.fail_command = None
        self.interrupt_command = None
        self.train_status = 'candidate'

    def local(self, remote: str) -> Path:
        return self.remote / remote.removeprefix('/workspace/job/').rstrip('/')

    def result(self, command: list[str], text: str, code: int, kwargs: dict) -> subprocess.CompletedProcess:
        output = text if kwargs.get('text') else text.encode()
        result = subprocess.CompletedProcess(command, code, output, '')
        result.check_returncode()
        return result

    def timed(self, command: list[str], seconds: float, **kwargs: object) -> subprocess.CompletedProcess:
        return self.run(command, text=True, **kwargs)

    def run(self, command: list[str], **kwargs: object) -> subprocess.CompletedProcess:
        self.calls.append(command)
        name = Path(command[0]).name
        args = command[1:]
        if self.interrupt_command is not None and command[:len(self.interrupt_command)] == self.interrupt_command:
            raise chain.Interrupted(signal.SIGTERM)
        if self.fail_command is not None and command[:len(self.fail_command)] == self.fail_command:
            return self.result(command, '', 7, kwargs)
        if name == 'vastai':
            if self.started and self.deny_account_after_start:
                return self.result(command, '', 1, kwargs)
            if args[:2] == ['show', 'instances']:
                if self.created:
                    assert (self.root / 'run/instance-id').read_text().strip() == '17'
                return self.result(command, json.dumps(self.instances), 0, kwargs)
            if args[:2] == ['search', 'offers']:
                return self.result(command, json.dumps([self.offer]), 0, kwargs)
            if args[:2] == ['show', 'user']:
                return self.result(command, '{"credit": 50}', 0, kwargs)
            if args[:2] == ['create', 'instance']:
                self.instances = [{'id': 17, 'label': args[args.index('--label') + 1], 'actual_status': 'running',
                                   'public_ipaddr': '192.0.2.1', 'ports': {'22/tcp': [{'HostPort': '2222'}]}}]
                self.stopped = self.started = False
                self.created = True
                shutil.rmtree(self.remote)
                self.remote.mkdir()
                return self.result(command, '{"new_contract": 17}', 0, kwargs)
            if args[:2] == ['destroy', 'instance']:
                self.instances = []
                self.created = False
                return self.result(command, '', 0, kwargs)
        if name == 'git':
            assert args[0] == '-C'
            args = args[2:]
            if args[0] == 'rev-parse':
                value = args[-1].removesuffix('^{commit}')
                if value in self.refs:
                    value = self.refs[value]
                if value == 'HEAD':
                    value = self.head
                return self.result(command, value + '\n', 0 if value in self.nets else 1, kwargs)
            if args[0] == 'update-ref':
                self.refs[args[1]] = self.head if args[2] == 'HEAD' else args[2]
            elif args[0] == 'show':
                commit = args[1].split(':')[0]
                data = self.nets[self.head if commit == 'HEAD' else commit]
                return subprocess.CompletedProcess(command, 0, data, b'')
            elif args[:2] == ['bundle', 'create']:
                Path(args[2]).write_bytes('履歴の代用'.encode())
            elif args[0] == 'commit':
                self.head = str(self.next_generation) * 40
                self.nets[self.head] = (self.root / 'tree/crates/minase/nets/nnue.bin').read_bytes()
                self.next_generation += 1
            elif args[0] == 'checkout':
                (self.root / 'tree/crates/minase/nets/nnue.bin').write_bytes(self.nets[self.head])
            elif args[0] != 'add':
                raise AssertionError(command)
            return self.result(command, '', 0, kwargs)
        if name == 'ssh':
            words = shlex.split(args[-1])
            if self.stopped:
                return self.result(command, '', 255, kwargs)
            if words == ['true']:
                return self.result(command, '', 0, kwargs)
            if words[-1] == 'cpu-info':
                return self.result(command, json.dumps(self.cpu), 0, kwargs)
            if words[-1] == 'stop':
                self.stopped = True
                return self.result(command, '', 0, kwargs)
            if words[:2] == ['test', '-f']:
                return self.result(command, '', 0 if self.local(words[2]).is_file() else 1, kwargs)
            if words[0] == 'mkdir':
                self.local(words[-1]).mkdir(parents=True, exist_ok=True)
            elif words[0] == 'touch':
                self.local(words[1]).touch()
                self.started = True
            elif words[0] == 'mv':
                self.local(words[1]).replace(self.local(words[2]))
                if words[2].endswith('/job.json'):
                    self.jobs.append(json.loads(self.local(words[2]).read_text()))
            elif words[0] == 'cat':
                return self.result(command, self.local(words[1]).read_text(), 0, kwargs)
            else:
                raise AssertionError(command)
            return self.result(command, '', 0, kwargs)
        if name == 'rsync':
            if self.transfer_failures:
                self.transfer_failures -= 1
                return self.result(command, '', 255, kwargs)
            if self.missing_transfers:
                self.missing_transfers -= 1
                return self.result(command, '', 23, kwargs)
            remote = '/workspace/job/' + args[-2].split('/workspace/job/', 1)[1]
            source = self.local(remote)
            source.mkdir(parents=True, exist_ok=True)
            if '/teacher/' in remote:
                if self.worker_exit:
                    self.local('/workspace/job/logs').mkdir(exist_ok=True)
                    self.local('/workspace/job/logs/exit-status').write_text('7\n')
                if not self.stall:
                    job = json.loads(self.local('/workspace/job/job.json').read_text())
                    for number in range(job['target_books']):
                        if number not in job['skip_books'] and not (source / f'book-{number}.mnsd').exists():
                            fields = header_fields(job['generation'], number, self.entries) | self.bad_header
                            write_book(source / f'book-{number}.mnsd', fields)
                            log = self.local(f'/workspace/job/logs/gen-{job["generation"]}/book-{number}.log')
                            log.parent.mkdir(parents=True, exist_ok=True)
                            values = {'games_completed': 10, 'nodes': 20000, 'max_ply': 4000,
                                      'hash_mb': 16, 'random_moves_max': 0, 'games_discarded_max_ply': 1,
                                      'recorded_positions': self.entries} | self.bad_log
                            log.write_text(''.join(f'{key}: {value}\n' for key, value in values.items()))
                            self.generated.append((job['generation'], number))
                            break
            shutil.copytree(source, Path(args[-1]), dirs_exist_ok=True)
            return self.result(command, '', 0, kwargs)
        if name == 'inspect':
            fields = inspect_book(Path(args[-1]))
            return self.result(command, ''.join(f'{key}: {value}\n' for key, value in fields.items()), 0, kwargs)
        if name == 'trainer':
            if args[:2] == ['data', 'calibrate']:
                Path(args[args.index('--output') + 1]).write_text('{"classes": [{"teacher_cp_per_logit": 1234, "at_search_bound": false}]}')
            elif args[:2] == ['nnue', 'export-handcrafted']:
                Path(args[args.index('--output') + 1]).write_bytes(network(0))
            elif args[:2] == ['nnue', 'train']:
                directory = Path(args[args.index('--output-dir') + 1])
                directory.mkdir(parents=True, exist_ok=True)
                generation = int(directory.parent.name.removeprefix('gen-'))
                (directory / 'summary.json').write_text(json.dumps({'status': self.train_status, 'best_epoch': 3}))
                (directory / 'nnue.bin').write_bytes(network(generation))
                (directory / 'best.npz').touch()
            elif args[:2] == ['diag', 'nnue-removal']:
                Path(args[args.index('--output') + 1]).write_text('{}')
            else:
                raise AssertionError(command)
            return self.result(command, '', 0, kwargs)
        if name in ('cargo', 'minase'):
            return self.result(command, '', 0, kwargs)
        raise AssertionError(command)

    def scp(self, host: str, port: int, destination: str, source: Path) -> None:
        self.calls.append(['scp', host, str(port), str(source), destination])
        target = self.local(destination)
        if destination.endswith('/'):
            target.mkdir(parents=True, exist_ok=True)
            target /= source.name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)

    def start(self, command: list[str], **kwargs: object) -> MatchProcess:
        self.calls.append(command)
        assert Path(command[0]).name == 'match'
        assert kwargs['cwd'] == self.root / 'tree'
        flag = '--resume' if '--resume' in command else '--run-dir'
        Path(command[command.index(flag) + 1]).mkdir(parents=True, exist_ok=True)
        process = MatchProcess(Path(kwargs['stdout'].name), self.match_output, self.match_code, self.match_polls)
        self.processes.append(process)
        return process

    def finish(self) -> None:
        self.processes.clear()


class ChainTests(unittest.TestCase):
    """指示書の作業機側の観測可能な結果と外部コマンドを確かめる。"""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.values = cli_values(self.root)
        self.commands = Commands(self.root)
        self.clock = Clock()
        self.output, self.errors = io.StringIO(), io.StringIO()
        self.enterContext(redirect_stdout(self.output))
        self.enterContext(redirect_stderr(self.errors))
        self.enterContext(patch.object(chain, 'Children', return_value=self.commands))
        self.enterContext(patch.object(chain, 'time', self.clock))
        self.enterContext(patch.object(common, 'time', self.clock))
        self.enterContext(patch.object(common, 'run_timed', side_effect=self.commands.timed))
        self.enterContext(patch.object(common, 'scp', side_effect=self.commands.scp))
        # 注入漏れで実コマンドが起動すること自体をテストの失敗にする。
        self.enterContext(patch.object(subprocess, 'Popen', side_effect=AssertionError('実コマンドは禁止です')))
        self.enterContext(patch.object(subprocess, 'run', side_effect=AssertionError('実コマンドは禁止です')))

    def execute(self) -> int:
        return chain.main(argv(self.values))

    def train_calls(self, generation: int) -> list[list[str]]:
        return [call for call in self.commands.calls if call[:3] == [str(self.root / 'trainer'), 'nnue', 'train']
                and str(self.root / f'run/gen-{generation}/train') in call]

    def assert_stopped(self) -> None:
        self.assertTrue(any(call[0] == 'ssh' and call[-1].endswith('generation_worker.py stop')
                            for call in self.commands.calls))

    def save_book(self, generation: int, number: int, entries: int) -> Path:
        path = self.root / f'run/gen-{generation}/teacher/book-{number}.mnsd'
        write_book(path, header_fields(generation, number, entries))
        log = self.root / f'run/gen-{generation}/logs/book-{number}.log'
        log.parent.mkdir(parents=True, exist_ok=True)
        log.write_text(f'recorded_positions: {entries}\n')
        return path

    def test_every_required_argument_is_required_before_creating(self) -> None:
        """事例1: どの必須条件を省略しても、出品の作成には進まない。"""
        for name in self.values.keys() - {'offer'}:
            with self.subTest(name=name), self.assertRaises(SystemExit) as raised:
                chain.main(argv({key: value for key, value in self.values.items() if key != name}))
            self.assertEqual(raised.exception.code, 2)
        self.assertFalse(self.commands.calls)

    def test_duplicate_label_is_not_created_or_destroyed(self) -> None:
        """事例2: 同じラベルの既存機を、新規の作成や失敗時の削除の対象にしない。"""
        self.commands.instances = [{'id': 99, 'label': 'minase-generation-chain-run'}]
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(any(call[1:3] in (['create', 'instance'], ['destroy', 'instance'])
                             for call in self.commands.calls))

    def test_each_offer_condition_is_required(self) -> None:
        """事例3: 出品の全11条件について、境界の外側を作成前に拒否する。"""
        failures = {'gpu_frac': 0.5, 'cpu_cores_effective': 383, 'cpu_cores': 383,
                    'cpu_name': 'AMD EPYC 7R13', 'verification': 'unverified', 'reliability2': 0.949,
                    'duration': 431999, 'direct_port_count': 0, 'inet_up': 99.9,
                    'disk_space': 49.9, 'dph_total': 0.501}
        original = self.commands.offer.copy()
        for key, value in failures.items():
            with self.subTest(key=key):
                self.commands.offer = original | {key: value}
                self.assertNotEqual(self.execute(), 0)
        self.assertFalse(any(call[1:3] == ['create', 'instance'] for call in self.commands.calls))

    def test_two_generations_use_validated_books_and_generation_commits(self) -> None:
        """条件の境界値を受理し、生成、較正、学習、限定したコミットを2世代続ける。"""
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.commands.generated, [(1, 0), (2, 0)])
        self.assertEqual(self.commands.refs['refs/nnue-generation-reset/gen-2'], '2' * 40)
        self.assertEqual(self.commands.jobs[-1]['commit'], '1' * 40)
        path = self.root / 'run/gen-2/teacher/book-0.mnsd'
        self.assertEqual(inspect_book(path)['network_checksum'], '01' * 32)
        self.assertEqual(path.stat().st_ino, (path.parent.parent / 'received/book-0.mnsd').stat().st_ino)
        self.assertFalse((self.root / 'run/instance-id').exists())
        exports = [call for call in self.commands.calls if call[1:3] == ['nnue', 'export-handcrafted']]
        self.assertEqual(len(exports), 1)
        self.assertEqual(exports[0][exports[0].index('--cp-per-logit') + 1], '1234')
        for generation in (1, 2):
            call = self.train_calls(generation)[0]
            base = 'base.bin' if generation == 1 else 'gen-1/train/nnue.bin'
            self.assertEqual(call[call.index('--base') + 1], str(self.root / 'run' / base))
            self.assertEqual(call[call.index('--lambda') + 1], '0.75')
            self.assertEqual(call[call.index('--teacher-cp-per-logit') + 1], 'calibrate')
        commits = [call for call in self.commands.calls if call[0] == 'git' and call[3] == 'commit']
        self.assertEqual(len(commits), 2)
        self.assertTrue(all(call[-2:] == ['--', 'crates/minase/nets/nnue.bin'] for call in commits))
        self.assertIn('完了', self.output.getvalue())

    def test_stc_failures_prevent_second_training_and_stop_machine(self) -> None:
        """事例4: 判定なし、H0、pending、非0終了のいずれでも第2世代を学習しない。"""
        for output, code in (('', 0), ('decision: H0\n', 0), ('decision: pending\n', 0), ('decision: H1\n', 7)):
            with self.subTest(output=output, code=code):
                self.commands.match_output, self.commands.match_code = output, code
                self.assertNotEqual(self.execute(), 0)
                self.assertFalse(self.train_calls(2))
                self.assert_stopped()
                self.assertFalse(self.commands.processes)
                # 保存した本と第1世代のコミットを使う、同じ中断点からの再開を試す。
                self.commands.stopped = False

    def test_stc_resume_restores_h1_and_allows_second_training(self) -> None:
        """事例5: 中断後の--resumeがH1を出した場合だけ、第2世代の学習へ進む。"""
        self.commands.match_output = ''
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(self.train_calls(2))
        self.commands.match_output = 'decision: H1\n'
        del self.values['offer']
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        matches = [call for call in self.commands.calls if Path(call[0]).name == 'match']
        self.assertEqual(matches[-1][1:], ['match', 'run', '--resume',
                                         str(self.root / 'matches/nnue-generation-reset-gen1-stc')])
        self.assertEqual(len(self.train_calls(2)), 1)

    def test_seed_book_count_and_deadline_formulas(self) -> None:
        """事例6、13: 指示書の数式を数値例と境界で確かめる。"""
        args = chain.arguments(argv(self.values))
        self.assertEqual(chain.Chain(args).seed(2, 3), 9020300000)
        self.assertEqual(chain.target_books(0, 0, 100000000, 37), 37)
        self.assertEqual(chain.target_books(10, 27500000, 100000000, 37), 37)
        self.assertEqual(chain.target_books(10, 100000000, 100000000, 37), 10)
        with self.assertRaises(chain.ChainStop):
            chain.target_books(1, 0, 100, 2)
        args.books_estimate = 37
        self.assertAlmostEqual(chain.deadline_hours(8, 0, args), 85.36)
        self.assertAlmostEqual(chain.deadline_hours(6, 7, args), 61.9)

    def test_invalid_book_is_never_linked(self) -> None:
        """事例7: 不一致の本を検査済みの入力へ入れず、反復を停止する。"""
        self.commands.bad_header = {'generation_commit': 'f' * 40}
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(list((self.root / 'run').glob('gen-*/teacher/*.mnsd')))
        self.assertFalse(self.train_calls(1))
        self.assert_stopped()

    def test_generation_log_must_account_for_all_games(self) -> None:
        """事例7: 保存対局数と最大手数で捨てた局数の和も照合する。"""
        self.commands.bad_log = {'games_discarded_max_ply': 0}
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(list((self.root / 'run').glob('gen-*/teacher/*.mnsd')))

    def test_resume_skips_verified_books_on_replacement_machine(self) -> None:
        """事例8: 作業機に保存済みの本はskip_booksへ入り、生成し直さない。"""
        saved = self.save_book(1, 0, 40).read_bytes()
        self.commands.entries = 40
        self.values['last-generation'] = '1'
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.commands.jobs[0]['skip_books'], [0])
        self.assertEqual(self.commands.jobs[0]['target_books'], 3)
        self.assertEqual(self.commands.generated, [(1, 1), (1, 2)])
        self.assertEqual((self.root / 'run/gen-1/teacher/book-0.mnsd').read_bytes(), saved)

    def test_expired_account_allows_generation_and_stop_but_not_completion(self) -> None:
        """事例9: 作成後の認証切れでも2世代を作り、削除未確認を成功にしない。"""
        self.commands.deny_account_after_start = True
        self.assertNotEqual(self.execute(), 0)
        self.assertIn('refs/nnue-generation-reset/gen-2', self.commands.refs)
        self.assert_stopped()
        self.assertTrue((self.root / 'run/instance-id').is_file())
        calls = self.commands.calls
        started = next(index for index, call in enumerate(calls) if call[-1] == 'touch /workspace/job/started')
        later_api = [call[1:3] for call in calls[started + 1:] if call[0] == 'vastai']
        self.assertEqual(later_api, [['destroy', 'instance'], ['destroy', 'instance']])
        self.assertIn('同じコマンド', self.errors.getvalue())
        self.commands.deny_account_after_start = False
        mark = len(calls)
        self.assertEqual(self.execute(), 0)
        self.assertFalse(any(Path(call[0]).name in ('trainer', 'ssh', 'rsync', 'match', 'cargo') for call in calls[mark:]))
        self.assertFalse((self.root / 'run/instance-id').exists())

    def test_worker_exit_status_stops_chain(self) -> None:
        """事例10: 目標の本が同時に届いても、作業者の失敗を無視しない。"""
        self.commands.worker_exit = True
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(self.train_calls(1))
        self.assert_stopped()

    def test_no_new_book_exceeds_stall_limit(self) -> None:
        """事例11: 接続できていても新しい本が届かなければ止まる。"""
        self.commands.stall = True
        self.assertNotEqual(self.execute(), 0)
        self.assertGreaterEqual(sum(self.clock.sleeps), 3)
        self.assert_stopped()

    def test_transport_failures_retry_until_limit(self) -> None:
        """一時的な取得失敗は再試行し、連続する取得失敗は上限で停止する。"""
        self.commands.transfer_failures = 1
        self.assertEqual(self.execute(), 0, self.errors.getvalue())

    def test_transport_failure_limit_stops(self) -> None:
        self.commands.transfer_failures = 20
        self.values['book-stall-minutes'] = '1'
        self.assertNotEqual(self.execute(), 0)
        self.assertIn('SSHまたはrsyncの失敗が上限時間', self.errors.getvalue())
        self.assert_stopped()

    def test_rsync_23_allows_directory_to_appear_later(self) -> None:
        self.commands.missing_transfers = 2
        self.assertEqual(self.execute(), 0, self.errors.getvalue())

    def test_completed_books_freeze_training_input_on_resume(self) -> None:
        """目標達成済みの世代は生成も取得もせず、学習入力を変えない。"""
        self.save_book(1, 0, 100)
        (self.root / 'run/base.bin').write_bytes(network(0))
        self.values['last-generation'] = '1'
        del self.values['offer']
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertFalse(any(call[0] in ('vastai', 'scp', 'ssh', 'rsync') for call in self.commands.calls))
        self.assertEqual(len(self.train_calls(1)), 1)

    def test_training_summary_skips_train_and_latest_resumes(self) -> None:
        """事例14: 完成した要約は再学習せず、途中のチェックポイントは再開する。"""
        self.save_book(1, 0, 100)
        (self.root / 'run/base.bin').write_bytes(network(0))
        train = self.root / 'run/gen-1/train'
        train.mkdir()
        (train / 'latest.npz').touch()
        runner = chain.Chain(chain.arguments(argv(self.values)))
        runner.train(1)
        self.assertEqual(self.train_calls(1)[0][-2:], ['--resume', str(train)])
        runner.train(1)
        self.assertEqual(len(self.train_calls(1)), 1)
        self.assertEqual(len([call for call in self.commands.calls if call[1:3] == ['diag', 'nnue-removal']]), 1)

    def test_existing_offer_is_refused_and_unreachable_resume_never_queries_api(self) -> None:
        directory = self.root / 'run'
        directory.mkdir()
        (directory / 'instance-id').write_text('99\n')
        (directory / 'ssh-address').write_text('192.0.2.1 2222\n')
        self.commands.instances = [{'id': 99, 'label': 'minase-generation-chain-run'}]
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(any(call[1:3] in (['create', 'instance'], ['destroy', 'instance'])
                             for call in self.commands.calls))
        del self.values['offer']
        self.commands.stopped = True
        mark = len(self.commands.calls)
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(any(call[0] == 'vastai' for call in self.commands.calls[mark:]))
        self.assertTrue((directory / 'instance-id').is_file())

    def test_deleted_instance_is_replaced_and_cpu_shortage_destroys(self) -> None:
        directory = self.root / 'run'
        directory.mkdir()
        for name, value in (('instance-id', '99'), ('ssh-address', '192.0.2.99 1'), ('deadline', '1')):
            (directory / name).write_text(value + '\n')
        self.commands.cpu = {'nproc': 384, 'cpu_limit': 345.5}
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse((directory / 'instance-id').exists())
        self.assertFalse(self.commands.started)
        self.assertTrue(any(call[1:3] == ['destroy', 'instance'] for call in self.commands.calls))

    def test_signal_leaves_machine_for_resume(self) -> None:
        self.commands.interrupt_command = [str(self.root / 'trainer'), 'nnue', 'train']
        self.assertEqual(self.execute(), 128 + signal.SIGTERM)
        self.assertFalse(any(call[0] == 'ssh' and call[-1].endswith('generation_worker.py stop')
                             for call in self.commands.calls))
        self.assertTrue((self.root / 'run/instance-id').exists())
        self.assertFalse(self.commands.processes)

    def test_failed_bench_restores_network_without_touching_other_files(self) -> None:
        unrelated = self.root / 'tree/other.txt'
        unrelated.write_text('作業中の変更')
        self.commands.fail_command = [str(self.root / 'tree/target/release/minase'), 'dev', 'bench']
        self.assertNotEqual(self.execute(), 0)
        self.assertEqual((self.root / 'tree/crates/minase/nets/nnue.bin').read_bytes(), network(0))
        self.assertEqual(unrelated.read_text(), '作業中の変更')
        self.assertNotIn('refs/nnue-generation-reset/gen-1', self.commands.refs)

    def test_commit_already_written_only_creates_reference(self) -> None:
        self.save_book(1, 0, 100)
        train = self.root / 'run/gen-1/train'
        train.mkdir()
        (train / 'nnue.bin').write_bytes(network(1))
        self.commands.head = '1' * 40
        self.commands.nets[self.commands.head] = network(1)
        runner = chain.Chain(chain.arguments(argv(self.values)))
        runner.commit_generation(1, {'best_epoch': 3}, {0: 100})
        self.assertEqual(self.commands.refs['refs/nnue-generation-reset/gen-1'], '1' * 40)
        self.assertFalse(any(call[0] == 'cargo' or (call[0] == 'git' and call[3] == 'commit')
                             for call in self.commands.calls))

    def test_all_header_and_log_conditions_are_checked(self) -> None:
        """不正な各項目は、生成コミットの不一致と同じく入力への採用を阻止する。"""
        headers = {'rule_set': 'L0,P0,R2,E0', 'network_checksum': 'ff' * 32,
                   'teacher_search_budget': 19999, 'seed': 9010000001}
        logs = {'games_completed': 9, 'nodes': 19999, 'max_ply': 3999,
                'hash_mb': 15, 'random_moves_max': 1, 'recorded_positions': 1}
        for field, value in [*headers.items(), *logs.items()]:
            with self.subTest(field=field):
                self.commands.bad_header = {field: value} if field in headers else {}
                self.commands.bad_log = {field: value} if field in logs else {}
                self.assertNotEqual(self.execute(), 0)
                self.assertFalse(list((self.root / 'run').glob('gen-*/teacher/*.mnsd')))
                self.assertFalse(self.train_calls(1))

    def test_no_offer_without_machine_cannot_generate(self) -> None:
        del self.values['offer']
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(any(call[0] in ('vastai', 'ssh') for call in self.commands.calls))

    def test_reachable_resume_uses_saved_address_without_account_query(self) -> None:
        self.commands.interrupt_command = [str(self.root / 'trainer'), 'nnue', 'train']
        self.assertEqual(self.execute(), 128 + signal.SIGTERM)
        self.commands.interrupt_command = None
        del self.values['offer']
        mark = len(self.commands.calls)
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        api = [call[1:3] for call in self.commands.calls[mark:] if call[0] == 'vastai']
        self.assertEqual(api, [['destroy', 'instance'], ['show', 'instances']])

    def test_max_books_boundary_and_overflow(self) -> None:
        self.values.update({'last-generation': '1', 'books-estimate': '1', 'max-books': '2',
                            'positions-per-generation': '80'})
        self.commands.entries = 20
        self.assertNotEqual(self.execute(), 0)
        self.assertIn('max-books', self.errors.getvalue())
        self.assertFalse(self.train_calls(1))
        self.values.update({'positions-per-generation': '40', 'books-estimate': '2'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertTrue(all(job['target_books'] <= 2 for job in self.commands.jobs))

    def test_failed_training_or_diagnostic_stops_before_commit(self) -> None:
        for failure in (['nnue', 'train'], ['diag', 'nnue-removal']):
            with self.subTest(failure=failure):
                self.commands.fail_command = [str(self.root / 'trainer'), *failure]
                self.assertNotEqual(self.execute(), 0)
                self.assertNotIn('refs/nnue-generation-reset/gen-1', self.commands.refs)
                self.assert_stopped()

    def test_non_candidate_summary_stops_before_commit(self) -> None:
        self.commands.train_status = 'error'
        self.assertNotEqual(self.execute(), 0)
        self.assertNotIn('refs/nnue-generation-reset/gen-1', self.commands.refs)

    def test_calibration_reuses_report_and_rejects_multiple_classes(self) -> None:
        self.save_book(1, 0, 100)
        path = self.root / 'run/calibration.json'
        path.write_text('{"classes": [{"teacher_cp_per_logit": 777, "at_search_bound": false}]}')
        runner = chain.Chain(chain.arguments(argv(self.values)))
        runner.calibrate(1)
        self.assertFalse(any(call[1:3] == ['data', 'calibrate'] for call in self.commands.calls))
        self.assertIn('777', self.commands.calls[-1])
        (self.root / 'run/base.bin').unlink()
        path.write_text('{"classes": [{}, {}]}')
        with self.assertRaises(chain.ChainStop):
            runner.calibrate(1)
        path.write_text('{"classes": [{"teacher_cp_per_logit": 4000, "at_search_bound": true}]}')
        with self.assertRaises(chain.ChainStop):
            runner.calibrate(1)

    def test_partial_stc_line_and_early_h1_do_not_allow_training(self) -> None:
        self.commands.refs['refs/nnue-generation-reset/gen-0'] = '0' * 40
        self.commands.refs['refs/nnue-generation-reset/gen-1'] = '1' * 40
        self.commands.nets['1' * 40] = network(1)
        self.commands.match_polls = 5
        self.commands.match_output = ''
        self.commands.match_code = 7
        runner = chain.Chain(chain.arguments(argv(self.values)))
        runner.start_stc()
        path = self.root / 'matches/nnue-generation-reset-gen1-stc.log'
        path.write_bytes(b'decision: H')
        self.assertFalse(runner.check_stc())
        path.write_bytes(b'decision: H1\n\xe7')
        self.assertFalse(runner.check_stc())
        with self.assertRaises(chain.ChainStop):
            runner.wait_stc(2)

    def test_stale_h1_is_not_a_decision_for_resumed_process(self) -> None:
        directory = self.root / 'matches/nnue-generation-reset-gen1-stc'
        directory.mkdir(parents=True)
        directory.with_suffix('.log').write_text('decision: H1\n')
        self.commands.match_output = ''
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(self.train_calls(2))

    def test_cpu_quota_boundary_is_accepted(self) -> None:
        self.commands.cpu['cpu_limit'] = 345.6
        self.assertEqual(self.execute(), 0, self.errors.getvalue())

    def test_affinity_shortage_destroys_before_start(self) -> None:
        self.commands.cpu['nproc'] = 383
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(self.commands.started)
        self.assertFalse((self.root / 'run/instance-id').exists())


class WorkerTests(unittest.TestCase):
    """事例12の公開順序、失敗、CPU上限、自己停止を外部契約から検証する。"""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.clock = Clock()
        self.output, self.errors = io.StringIO(), io.StringIO()
        self.enterContext(redirect_stdout(self.output))
        self.enterContext(redirect_stderr(self.errors))
        self.enterContext(patch.object(worker, 'time', self.clock))
        self.enterContext(patch.object(subprocess, 'Popen', side_effect=AssertionError('実コマンドは禁止です')))
        self.job = {'generation': 1, 'commit': '0' * 40, 'bundle': '/workspace/job/bundles/gen-0.bundle',
                    'generation_seed': 9010000000, 'book_step': 100000, 'target_books': 3, 'skip_books': [0],
                    'games': 10, 'nodes': 20000, 'random_moves': 0, 'max_ply': 4000, 'hash_mb': 16, 'concurrency': 384}
        (self.root / 'worker.json').write_text('{"idle_limit_seconds": 5}')
        self.environ = self.root / 'environ'
        self.environ.write_bytes(b'CONTAINER_ID=17\0CONTAINER_API_KEY=private-test-key\0')
        self.calls = []
        self.fail = None
        self.head = None
        self.publications = []

    def command(self, command: list[str], **kwargs: object) -> subprocess.CompletedProcess:
        self.calls.append(command)
        if self.fail is not None and command[:len(self.fail)] == self.fail:
            raise subprocess.CalledProcessError(7, command)
        if command[:2] == ['git', 'init']:
            Path(command[2]).mkdir()
        if command[:2] == ['git', 'checkout']:
            self.head = command[-1]
        if command[:2] == ['git', 'rev-parse']:
            return subprocess.CompletedProcess(command, 128 if self.head is None else 0,
                                               '' if self.head is None else self.head + '\n')
        if Path(command[0]).name == 'minase':
            path = Path(command[command.index('--output') + 1])
            self.assertTrue(path.is_absolute())
            self.assertTrue(path.is_relative_to(self.root / 'work'))
            self.assertEqual(kwargs['cwd'], self.root / 'minase')
            self.assertNotIn('--allow-dirty', command)
            self.assertNotIn('--rules', command)
            self.assertEqual(command[command.index('--concurrency') + 1], '384')
            path.write_bytes('本'.encode())
            kwargs['stdout'].write('生成のログ\n')
            self.clock.now += 4
        return subprocess.CompletedProcess(command, 0, '')

    def test_next_book_excludes_local_and_remote_books(self) -> None:
        path = self.root / 'teacher/gen-1/book-1.mnsd'
        path.parent.mkdir(parents=True)
        path.touch()
        self.assertEqual(worker.next_book(self.job, self.root), 2)
        (path.parent / 'book-2.mnsd').touch()
        self.assertIsNone(worker.next_book(self.job, self.root))

    def test_worker_publishes_log_before_book_and_stops_after_idle(self) -> None:
        (self.root / 'job.json').write_text(json.dumps(self.job))
        work = self.root / 'work'
        work.mkdir()
        (work / 'partial.mnsd').touch()
        logs = self.root / 'logs'
        logs.mkdir()
        (logs / 'exit-status').write_text('99\n')
        replace = Path.replace

        def publish(source: Path, destination: Path) -> Path:
            self.publications.append(str(destination.relative_to(self.root)))
            if destination.suffix == '.mnsd':
                self.assertTrue((self.root / 'logs/gen-1' / (destination.stem + '.log')).is_file())
            return replace(source, destination)

        with patch.object(subprocess, 'run', side_effect=self.command), patch.object(Path, 'replace', publish), \
                patch.object(worker, 'stop_instance', return_value=0) as stop:
            self.assertEqual(worker.run_worker(self.root), 0)
        self.assertEqual(self.publications, ['logs/gen-1/book-1.log', 'teacher/gen-1/book-1.mnsd',
                                            'logs/gen-1/book-2.log', 'teacher/gen-1/book-2.mnsd'])
        self.assertFalse((work / 'partial.mnsd').exists())
        self.assertFalse((logs / 'exit-status').exists())
        stop.assert_called_once_with(Path('/proc/1/environ'))
        self.assertEqual(sum(self.clock.sleeps), 5)
        generated = [call for call in self.calls if Path(call[0]).name == 'minase']
        self.assertEqual([call[call.index('--seed') + 1] for call in generated], ['9010100000', '9010200000'])
        self.assertEqual([call for call in self.calls if call[0] == 'cargo'],
                         [['cargo', 'build', '--release', '--locked', '--bin', 'minase']])
        self.assertIn(['git', 'fetch', '/workspace/job/bundles/gen-0.bundle', 'refs/nnue-generation-reset/gen-0'], self.calls)

    def test_preparation_failure_writes_failed_log_and_atomic_status(self) -> None:
        (self.root / 'job.json').write_text(json.dumps(self.job))
        self.fail = ['cargo', 'build']
        with patch.object(subprocess, 'run', side_effect=self.command):
            self.assertEqual(worker.run_worker(self.root), 7)
        self.assertEqual((self.root / 'logs/exit-status').read_text(), '7\n')
        self.assertFalse((self.root / 'logs/.exit-status.tmp').exists())
        self.assertTrue((self.root / 'logs/gen-1/book-1.failed.log').is_file())
        self.assertFalse(list((self.root / 'teacher').glob('**/*.mnsd')))

    def test_generation_failure_does_not_publish_book(self) -> None:
        (self.root / 'job.json').write_text(json.dumps(self.job))
        self.fail = [str(self.root / 'minase/target/release/minase')]
        with patch.object(subprocess, 'run', side_effect=self.command):
            self.assertEqual(worker.run_worker(self.root), 7)
        self.assertTrue((self.root / 'logs/gen-1/book-1.failed.log').is_file())
        self.assertFalse(list((self.root / 'teacher').glob('**/*.mnsd')))

    def test_restart_builds_even_if_checkout_was_finished(self) -> None:
        (self.root / 'job.json').write_text(json.dumps(self.job))
        (self.root / 'minase').mkdir()
        self.head = self.job['commit']
        with patch.object(subprocess, 'run', side_effect=self.command), patch.object(worker, 'stop_instance', return_value=0):
            self.assertEqual(worker.run_worker(self.root), 0)
        self.assertTrue(any(call[0] == 'cargo' for call in self.calls))
        self.assertFalse(any(call[:2] == ['git', 'fetch'] for call in self.calls))

    def test_no_initial_job_stops_at_idle_limit(self) -> None:
        with patch.object(worker, 'stop_instance', return_value=0) as stop:
            self.assertEqual(worker.run_worker(self.root), 0)
        stop.assert_called_once()
        self.assertEqual(sum(self.clock.sleeps), 5)

    def test_cpu_info_reads_v1_v2_and_unlimited(self) -> None:
        with patch.object(worker.os, 'sched_getaffinity', return_value=set(range(384))):
            (self.root / 'cpu.max').write_text('34560000 100000\n')
            self.assertEqual(worker.cpu_info(self.root), {'nproc': 384, 'cpu_limit': 345.6})
            (self.root / 'cpu.max').write_text('max 100000\n')
            self.assertIsNone(worker.cpu_info(self.root)['cpu_limit'])
            (self.root / 'cpu.max').unlink()
            (self.root / 'cpu').mkdir()
            (self.root / 'cpu/cpu.cfs_quota_us').write_text('19200000\n')
            (self.root / 'cpu/cpu.cfs_period_us').write_text('100000\n')
            self.assertEqual(worker.cpu_info(self.root), {'nproc': 384, 'cpu_limit': 192})
            (self.root / 'cpu/cpu.cfs_quota_us').write_text('-1\n')
            self.assertIsNone(worker.cpu_info(self.root)['cpu_limit'])

    def test_http_success_and_client_errors_do_not_retry(self) -> None:
        for status, expected in ((200, 0), (204, 0), (401, 1), (403, 1)):
            with self.subTest(status=status):
                response = unittest.mock.MagicMock()
                response.__enter__.return_value.status = status
                with patch.object(worker.urllib.request, 'urlopen', return_value=response) as request:
                    self.assertEqual(worker.stop_instance(self.environ), expected)
                request.assert_called_once()
                actual = request.call_args.args[0]
                self.assertEqual(actual.full_url, 'https://console.vast.ai/api/v0/instances/17/')
                self.assertEqual(actual.method, 'PUT')
                self.assertEqual(json.loads(actual.data), {'state': 'stopped'})
                self.assertEqual(actual.get_header('Authorization'), 'Bearer private-test-key')
        self.assertNotIn('private-test-key', self.output.getvalue() + self.errors.getvalue())
        self.assertFalse(self.clock.sleeps)

    def test_http_server_and_transport_errors_retry_for_at_most_thirty_minutes(self) -> None:
        failures = [urllib.error.HTTPError('https://example.invalid', 503, '', {}, io.BytesIO()),
                    urllib.error.URLError('private-test-key')]
        response = unittest.mock.MagicMock()
        response.__enter__.return_value.status = 200
        with patch.object(worker.urllib.request, 'urlopen', side_effect=[*failures, response]) as request:
            self.assertEqual(worker.stop_instance(self.environ), 0)
        self.assertEqual(request.call_count, 3)
        self.assertEqual(self.clock.sleeps, [60, 60])
        self.clock.sleeps.clear()
        with patch.object(worker.urllib.request, 'urlopen', side_effect=urllib.error.URLError('private-test-key')):
            self.assertEqual(worker.stop_instance(self.environ), 1)
        self.assertEqual(sum(self.clock.sleeps), 1800)
        self.assertNotIn('private-test-key', self.output.getvalue() + self.errors.getvalue())


if __name__ == '__main__':
    unittest.main()
