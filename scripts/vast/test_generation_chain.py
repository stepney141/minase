#!/usr/bin/env python3
"""世代反復と生成機の契約を、外部サービスを呼ばずに検証する。

使い方: python3 scripts/vast/test_generation_chain.py
本ファイルはgeneration_chain.py、generation_worker.pyと同じディレクトリに置く。
引数と出品の拒否、教師の受理、学習と補正の再開、補正後のネットのコミット、
停止と削除、作業者の公開順序と自己停止を検証する。
期待値は指示書の式とコマンド、およびtraining/records.rsのMNSD版2の形式に基づく。
外部コマンドは注入した実行関数で置き換え、時計も進めるだけにする。
"""

from contextlib import redirect_stderr, redirect_stdout
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
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
import zipfile

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


def cli_values(root: Path) -> dict[str, str | list[str]]:
    """必須条件を省略せずに与える。テストの既定値を本体へ持ち込まない。"""
    return {'run-dir': str(root / 'run'), 'worktree': str(root / 'tree'),
            'minase-bin': str(root / 'inspect'), 'trainer-bin': str(root / 'trainer'),
            'gen0-commit': '0' * 40, 'first-generation': '1', 'last-generation': '2', 'train-place': 'local',
            'ref-prefix': 'refs/nnue-standard-recipe', 'books-per-generation': '1',
            'replay-buffer-generations': '1', 'fit': 'yes', 'lr-linear': '0.67903', 'games': '10', 'nodes': '20000',
            'random-moves': '0', 'max-ply': '4000', 'hash-mb': '16', 'seed-base': '9000000000',
            'seed-generation-step': '10000000', 'seed-book-step': '100000',
            'calibration-books': '1', 'lambda': '1.0', 'feature-set': 'P', 'width': '256',
            'head': 'pairwise-l16-l32', 'batch': '16384', 'epochs': '50', 'patience': '5', 'train-seed': '1',
            'export-epoch': 'best',
            'device': 'cuda', 'validation-interval': '1000', 'lr-schedule': 'cosine:3:0.1', 'train-threads': '8',
            'removal-sample': '10', 'removal-seed': '3', 'bench-depth': '5',
            'fit-tau': ['0.5', '1.5'], 'fit-seed': '0', 'fit-threads': '3',
            'offer': '42', 'image': 'rust:1.98.0-bookworm', 'disk': '50', 'max-price': '0.5',
            'min-cpu-cores': '384', 'cpu-pattern': 'EPYC.*9', 'min-duration-days': '5', 'min-reliability': '0.95',
            'min-inet-up': '100', 'min-disk': '50', 'book-minutes': '12', 'deadline-margin': '1.3',
            'training-wait-hours': '1.2', 'idle-limit-hours': '8', 'poll-seconds': '1',
            'book-stall-minutes': '0.05', 'ssh-failure-minutes': '0.05'}


def argv(values: dict[str, str | list[str]]) -> list[str]:
    result = ['run']
    for key, value in values.items():
        result.extend(['--' + key, *(value if isinstance(value, list) else [value])])
    return result


def option_values(command: list[str], option: str) -> list[str]:
    """外部コマンドの引数を、オプションの順序に依存せず読む。"""
    result = []
    for value in command[command.index(option) + 1:]:
        if value.startswith('--'):
            break
        result.append(value)
    return result


def write_training_result(directory: Path, generation: int, fit: bool) -> None:
    """学習器の出力形式で、実際の重みや損失に依存しない成果物を置く。"""
    (directory / 'train').mkdir(parents=True, exist_ok=True)
    (directory / 'train/summary.json').write_text('{"status": "candidate", "best_epoch": 4}')
    (directory / 'train/nnue.bin').write_bytes(network(generation) + b'unfitted')
    (directory / 'train/best.npz').write_bytes(b'checkpoint')
    (directory / 'removal.json').write_text('{}')
    if fit:
        (directory / 'fit').mkdir(exist_ok=True)
        (directory / 'fit/summary.json').write_text('{"decision": {"d": -0.25}}')
        (directory / 'fit/nnue.bin').write_bytes(network(generation))


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


class Commands:
    """Git、Vast.ai、SSH、学習器の外部契約だけを代行する。"""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.remote = root / 'remote'
        self.remote.mkdir()
        self.calls = []
        self.refs = {}
        self.head = '0' * 40
        self.nets = {self.head: network(0)}
        self.next_generation = 1
        (root / 'tree/crates/minase/nets').mkdir(parents=True)
        (root / 'tree/crates/minase/nets/nnue.bin').write_bytes(network(0))
        self.instances = []
        self.offer = {'id': 42, 'gpu_frac': 1, 'cpu_cores_effective': 384, 'cpu_cores': 384,
                      'num_gpus': 1, 'cuda_max_good': 13.0,
                      'cpu_name': 'AMD EPYC 9R14', 'verification': 'verified', 'reliability2': 0.95,
                      'duration': 432000, 'direct_port_count': 1, 'inet_up': 100, 'disk_space': 50, 'dph_total': 0.5}
        self.cpu = {'nproc': 384, 'cpu_limit': 384}
        self.started = False
        self.stopped = False
        self.created = False
        self.deny_account_after_start = False
        self.fail_stop = False
        self.generated = []
        self.jobs = []
        self.bad_header = {}
        self.bad_log = {}
        self.entries = 120
        self.stall = False
        self.worker_exit = False
        self.transfer_failures = 0
        self.missing_transfers = 0
        self.fail_command = None
        self.interrupt_command = None
        self.train_status = 'candidate'
        self.fit_result = {'decision': {'d': -0.123456789}}
        self.fit_options = []
        self.training_jobs = []
        self.training_exit = None
        self.training_stall = False
        self.interrupt_training = False
        self.training_done_failures = 0
        self.training_missing = []

    def start_training(self, generation: int) -> None:
        directory = self.remote / f'training/gen-{generation}'
        (directory / 'logs').mkdir(parents=True, exist_ok=True)
        (directory / 'logs/train.log').write_text('生成機での学習ログ\n')
        if self.interrupt_training:
            raise chain.Interrupted(signal.SIGTERM)
        if self.training_exit is not None:
            (directory / 'exit-status').write_text(f'{self.training_exit}\n')
            return
        if self.training_stall:
            return
        job = json.loads((directory / 'job.json').read_text())
        write_training_result(directory, generation, any(step['name'] == 'fit' for step in job['steps']))
        for name in self.training_missing:
            (directory / name).unlink()
        (directory / 'done').write_text('done\n')

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
                if self.fail_stop:
                    return self.result(command, '', 7, kwargs)
                self.stopped = True
                return self.result(command, '', 0, kwargs)
            if words[:2] == ['test', '-f']:
                if words[2].endswith('/done') and '/training/' in words[2] and self.training_done_failures:
                    self.training_done_failures -= 1
                    return self.result(command, '', 255, kwargs)
                return self.result(command, '', 0 if self.local(words[2]).is_file() else 1, kwargs)
            if words[0] == 'mkdir':
                for path in words[2:]:
                    self.local(path).mkdir(parents=True, exist_ok=True)
            elif words[0] == 'touch':
                self.local(words[1]).touch()
                self.started = True
            elif words[0] == 'mv':
                self.local(words[1]).replace(self.local(words[2]))
                if words[2] == '/workspace/job/job.json':
                    self.jobs.append(json.loads(self.local(words[2]).read_text()))
                elif words[2].endswith('/job.json'):
                    self.training_jobs.append(json.loads(self.local(words[2]).read_text()))
            elif words[0] == 'cat':
                return self.result(command, self.local(words[1]).read_text(), 0, kwargs)
            elif words[:5] == ['nohup', 'setsid', 'python3', '/workspace/job/generation_worker.py', 'train']:
                self.start_training(int(words[5]))
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
            if '--ignore-existing' in args:
                destination = self.local('/workspace/job/' + args[-1].split('/workspace/job/', 1)[1])
                source = Path(args[-2])
                for path in source.iterdir():
                    target = destination / path.name
                    if not target.exists():
                        shutil.copyfile(path, target)
                return self.result(command, '', 0, kwargs)
            remote = '/workspace/job/' + args[-2].split('/workspace/job/', 1)[1]
            source = self.local(remote)
            if '/training/' in remote and not source.exists():
                return self.result(command, '', 23, kwargs)
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
                (directory / 'nnue.bin').write_bytes(network(generation) + '補正前の重み'.encode())
                (directory / 'best.npz').touch()
            elif args[:2] == ['nnue', 'fit-linear']:
                self.fit_options.append(kwargs)
                directory = Path(args[args.index('--output-dir') + 1])
                if directory.exists() and any(directory.iterdir()):
                    return self.result(command, '補正の出力先が空ではありません', 7, kwargs)
                directory.mkdir(parents=True, exist_ok=True)
                generation = int(directory.parent.name.removeprefix('gen-'))
                (directory / 'nnue.bin').write_bytes(network(generation))
                (directory / 'summary.json').write_text(json.dumps(self.fit_result))
                kwargs['stdout'].write('補正のログ\n')
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

    def finish(self) -> None:
        pass


class ChainTestCase(unittest.TestCase):
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

    def fit_calls(self, generation: int) -> list[list[str]]:
        return [call for call in self.commands.calls if call[:3] == [str(self.root / 'trainer'), 'nnue', 'fit-linear']
                and str(self.root / f'run/gen-{generation}/fit') in call]

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


class ChainTests(ChainTestCase):

    def test_export_epoch_selects_the_matching_float_checkpoint(self) -> None:
        """bestは学習器の既定を使い、lastだけ明示して対応するチェックポイントを診断する。"""
        directory = self.root / 'run/gen-1'
        for choice, checkpoint in (('best', 'best.npz'), ('last', 'latest.npz')):
            with self.subTest(export_epoch=choice):
                runner = chain.Chain(chain.arguments(argv(self.values | {'export-epoch': choice})))
                commands = runner.training_commands(self.root / 'trainer', [self.root / 'book.mnsd'],
                                                    [self.root / 'book.mnsd'], self.root / 'base.bin', directory)
                if choice == 'best':
                    self.assertNotIn('--export-epoch', commands['train'])
                else:
                    self.assertEqual(option_values(commands['train'], '--export-epoch'), ['last'])
                self.assertEqual(option_values(commands['removal'], '--float'),
                                 [str(directory / 'train' / checkpoint)])
                self.assertEqual(option_values(commands['removal'], '--net'),
                                 [str(directory / 'train/nnue.bin')])

    def test_export_epoch_rejects_invalid_choices(self) -> None:
        for value in ('', 'latest', '0', 'Best'):
            with self.subTest(export_epoch=value), self.assertRaises(SystemExit) as raised:
                chain.main(argv(self.values | {'export-epoch': value}))
            self.assertEqual(raised.exception.code, 2)
        self.assertFalse(self.commands.calls)

    def test_every_required_argument_is_required_before_creating(self) -> None:
        """事例1: どの必須条件を省略しても、出品の作成には進まない。"""
        with patch.object(common, 'call') as call, patch.object(common, 'create') as create:
            for name in self.values.keys() - {'offer'}:
                with self.subTest(name=name), self.assertRaises(SystemExit) as raised:
                    chain.main(argv({key: value for key, value in self.values.items() if key != name}))
                self.assertEqual(raised.exception.code, 2)
            call.assert_not_called()
            create.assert_not_called()
        self.assertFalse(self.commands.calls)

    def test_fit_arguments_reject_invalid_values_before_creating(self) -> None:
        """補正には1個以上の正の有限なtau、非負のシード、正のスレッド数が必要である。"""
        cases = [('fit-tau', []), ('fit-tau', '0'), ('fit-tau', '-1'), ('fit-tau', 'nan'),
                 ('fit-tau', 'inf'), ('fit-tau', ['0.5', 'nan']), ('fit-seed', '-1'),
                 ('fit-seed', '0.5'), ('fit-threads', '0'), ('fit-threads', '-1'), ('fit-threads', '1.5')]
        for name, value in cases:
            with self.subTest(name=name, value=value):
                with self.assertRaises(SystemExit) as raised:
                    chain.main(argv(self.values | {name: value}))
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
        """条件の境界値を受理し、生成、較正、学習、補正、限定したコミットを2世代続ける。"""
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.commands.generated, [(1, 0), (2, 0)])
        self.assertEqual(self.commands.refs['refs/nnue-standard-recipe/gen-2'], '2' * 40)
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
            base = 'base.bin' if generation == 1 else 'gen-1/fit/nnue.bin'
            self.assertEqual(call[call.index('--base') + 1], str(self.root / 'run' / base))
            self.assertEqual(call[call.index('--lambda') + 1], '1.0')
            self.assertEqual(call[call.index('--teacher-cp-per-logit') + 1], 'calibrate')
            self.assertNotIn('--export-epoch', call)
            directory = self.root / f'run/gen-{generation}'
            diagnostic = next(call for call in self.commands.calls if call[1:3] == ['diag', 'nnue-removal']
                              and str(directory / 'removal.json') in call)
            self.assertEqual(option_values(diagnostic, '--float'), [str(directory / 'train/best.npz')])
            self.assertEqual(option_values(diagnostic, '--net'), [str(directory / 'train/nnue.bin')])
            self.assertEqual(len(self.fit_calls(generation)), 1)
            self.assertEqual(self.commands.nets[str(generation) * 40], (directory / 'fit/nnue.bin').read_bytes())
        commits = [call for call in self.commands.calls if call[0] == 'git' and call[3] == 'commit']
        self.assertEqual(len(commits), 2)
        self.assertTrue(all(call[-2:] == ['--', 'crates/minase/nets/nnue.bin'] for call in commits))
        self.assertIn('完了', self.output.getvalue())

    def test_fit_uses_all_books_after_training_and_commits_fitted_network(self) -> None:
        """学習後に指定した条件で全教師を補正に使い、補正済みのネットとDをコミットする。"""
        self.values.update({'last-generation': '1', 'books-per-generation': '3'})
        self.commands.entries = 40
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        directory = self.root / 'run/gen-1'
        self.assertEqual(len(self.fit_calls(1)), 1)
        call = self.fit_calls(1)[0]
        expected = {'--base': [str(directory / 'train/nnue.bin')],
                    '--data': [str(directory / f'teacher/book-{number}.mnsd') for number in (0, 1, 2)],
                    '--tau': ['0.5', '1.5'], '--seed': ['0'], '--threads': ['3'],
                    '--output-dir': [str(directory / 'fit')]}
        self.assertEqual({part for part in call if part.startswith('--')}, set(expected))
        for option, values in expected.items():
            self.assertEqual(option_values(call, option), values)
        options = self.commands.fit_options[0]
        self.assertEqual(options['cwd'], self.root / 'tree')
        self.assertEqual(Path(options['stdout'].name), directory / 'fit.log')
        self.assertEqual(options['stderr'], subprocess.STDOUT)
        self.assertIs(options['preexec_fn'], chain.low_priority)
        self.assertEqual((directory / 'fit.log').read_text(), '補正のログ\n')
        commits = [item for item in self.commands.calls if item[0] == 'git' and item[3] == 'commit']
        self.assertEqual(len(commits), 1)
        self.assertLess(self.commands.calls.index(self.train_calls(1)[0]), self.commands.calls.index(call))
        self.assertLess(self.commands.calls.index(call), self.commands.calls.index(commits[0]))
        self.assertIn('教師3本、120局面、最良エポック3、D=-0.123457。', commits[0])
        fitted = (directory / 'fit/nnue.bin').read_bytes()
        self.assertNotEqual(fitted, (directory / 'train/nnue.bin').read_bytes())
        self.assertEqual(self.commands.nets[self.commands.head], fitted)

    def test_nonnegative_fit_stops_before_commit(self) -> None:
        """Dが0以上ならChainStopで終了コード1となり、コミットも次世代の学習もしない。"""
        for d in (0.0, -0.0, 0.125):
            with self.subTest(d=d):
                self.commands.fit_result = {'decision': {'d': d}}
                (self.root / 'run/gen-1/fit/summary.json').unlink(missing_ok=True)
                self.assertEqual(self.execute(), 1, self.errors.getvalue())
                runner = chain.Chain(chain.arguments(argv(self.values)))
                with self.assertRaises(chain.ChainStop):
                    runner.fit(1)
                self.assertNotIn('refs/nnue-standard-recipe/gen-1', self.commands.refs)
                self.assertFalse(any(call[0] == 'git' and call[3] == 'commit' for call in self.commands.calls))
                self.assertFalse(self.train_calls(2))
                self.assertEqual((self.root / 'tree/crates/minase/nets/nnue.bin').read_bytes(), network(0))
                self.assert_stopped()

    def test_fit_rejects_nonfinite_and_invalid_decisions(self) -> None:
        """完了した要約でも、Dが有限の数値でなければ補正結果を採用しない。"""
        directory = self.root / 'run/gen-1/fit'
        directory.mkdir(parents=True)
        runner = chain.Chain(chain.arguments(argv(self.values)))
        summaries = [{'decision': {'d': value}} for value in
                     (float('nan'), float('inf'), float('-inf'), None, True, '-0.1')]
        summaries.extend([{}, {'decision': {}}, {'decision': None}])
        for summary in summaries:
            with self.subTest(summary=summary):
                (directory / 'summary.json').write_text(json.dumps(summary))
                with self.assertRaises(chain.ChainStop):
                    runner.fit(1)
        self.assertFalse(self.fit_calls(1))

    def test_fit_summary_skips_fitting_and_preserves_output(self) -> None:
        """summary.jsonがあれば保存済みの要約を返し、補正を再実行せず成果物を残す。"""
        directory = self.root / 'run/gen-1/fit'
        directory.mkdir(parents=True)
        summary = {'decision': {'d': -0.25}}
        (directory / 'summary.json').write_text(json.dumps(summary))
        (directory / 'nnue.bin').write_bytes(network(1))
        (directory / 'retained.txt').write_text('完了済みの補正結果')
        runner = chain.Chain(chain.arguments(argv(self.values)))
        self.assertEqual(runner.fit(1), summary)
        self.assertFalse(self.fit_calls(1))
        self.assertEqual(json.loads((directory / 'summary.json').read_text()), summary)
        self.assertEqual((directory / 'nnue.bin').read_bytes(), network(1))
        self.assertEqual((directory / 'retained.txt').read_text(), '完了済みの補正結果')

    def test_incomplete_fit_output_is_removed_before_fitting(self) -> None:
        """要約のない出力先だけを消して補正をやり直し、既存のログには追記する。"""
        self.values['last-generation'] = '1'
        directory = self.root / 'run/gen-1/fit'
        (directory / 'partial').mkdir(parents=True)
        (directory / 'partial/weights').write_bytes(b'partial')
        (directory / 'nnue.bin').write_bytes(b'incomplete')
        log = directory.parent / 'fit.log'
        log.write_text('前回のログ\n')
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertFalse((directory / 'partial').exists())
        self.assertEqual(len(self.fit_calls(1)), 1)
        self.assertEqual((directory / 'nnue.bin').read_bytes(), network(1))
        self.assertTrue((directory / 'summary.json').is_file())
        self.assertEqual(log.read_text(), '前回のログ\n補正のログ\n')

    def test_seed_and_remaining_book_deadline_formulas(self) -> None:
        """事例6: 各世代の残りの本数に生成時間と余裕係数を掛ける。"""
        args = chain.arguments(argv(self.values))
        self.assertEqual(chain.Chain(args).seed(2, 3), 9020300000)
        args.books_per_generation = 37
        for count, remaining in ((26, 11), (0, 37)):
            with self.subTest(count=count):
                self.assertAlmostEqual(chain.deadline_hours([count], args), 12 * remaining * 1.3 / 60)
        self.assertAlmostEqual(chain.deadline_hours([26, 0], args), 12 * (11 + 37) * 1.3 / 60 + 1.2)
        self.assertAlmostEqual(chain.deadline_hours([74, 0], args), 12 * 37 * 1.3 / 60 + 1.2)

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
        self.values.update({'last-generation': '1', 'books-per-generation': '3'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.commands.jobs[0]['skip_books'], [0])
        self.assertEqual(self.commands.jobs[0]['target_books'], 3)
        self.assertEqual(self.commands.generated, [(1, 1), (1, 2)])
        self.assertEqual((self.root / 'run/gen-1/teacher/book-0.mnsd').read_bytes(), saved)

    def test_failed_stop_and_destroy_prevent_training(self) -> None:
        """停止要求と削除がともに失敗したら、生成機の停止と課金の確認を求めて学習前に終了する。"""
        self.values['last-generation'] = '1'
        self.commands.fail_stop = True
        self.commands.deny_account_after_start = True
        self.assertEqual(self.execute(), 1, self.errors.getvalue())
        self.assertEqual(self.commands.generated, [(1, 0)])
        self.assertFalse(self.train_calls(1))
        self.assertFalse(self.fit_calls(1))
        self.assertFalse(any(call[0] == 'git' and call[3] == 'commit' for call in self.commands.calls))
        self.assertEqual((self.root / 'run/instance-id').read_text().strip(), '17')
        message = self.errors.getvalue().splitlines()[-1]
        for text in ('生成機 17', '管理画面', '停止', '課金'):
            self.assertIn(text, message)

    def test_confirmed_stop_allows_training_when_destroy_fails(self) -> None:
        """SSH切断で停止を確認できたら学習、補正、コミットを行い、最後の削除失敗で終了コード1となる。"""
        self.values['last-generation'] = '1'
        self.commands.deny_account_after_start = True
        self.assertEqual(self.execute(), 1, self.errors.getvalue())
        self.assertTrue(self.commands.stopped)
        self.assertEqual(len(self.train_calls(1)), 1)
        self.assertEqual(len(self.fit_calls(1)), 1)
        commits = [call for call in self.commands.calls if call[0] == 'git' and call[3] == 'commit']
        self.assertEqual(len(commits), 1)
        self.assertEqual(self.commands.refs['refs/nnue-standard-recipe/gen-1'], '1' * 40)
        destroys = [index for index, call in enumerate(self.commands.calls)
                    if call[:3] == ['vastai', 'destroy', 'instance']]
        self.assertEqual(len(destroys), 2)
        self.assertLess(destroys[0], self.commands.calls.index(self.train_calls(1)[0]))
        self.assertLess(self.commands.calls.index(commits[0]), destroys[-1])
        self.assertTrue((self.root / 'run/instance-id').is_file())
        self.assertIn('削除が未確認', self.errors.getvalue())

    def test_expired_account_allows_generation_and_stop_but_not_completion(self) -> None:
        """事例9: 作成後の認証切れでも2世代を作り、削除未確認を成功にしない。"""
        self.commands.deny_account_after_start = True
        self.assertNotEqual(self.execute(), 0)
        self.assertIn('refs/nnue-standard-recipe/gen-2', self.commands.refs)
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
        self.assertFalse(any(Path(call[0]).name in ('trainer', 'ssh', 'rsync', 'cargo') for call in calls[mark:]))
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

    def test_completed_books_without_machine_ignore_offer(self) -> None:
        """本が目標に達していて保存した生成機がなければ、--offerがあっても検索も作成もしない。"""
        self.save_book(1, 0, 100)
        self.values['last-generation'] = '1'
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertFalse(any(call[0] == 'vastai' and call[1:3] in (['search', 'offers'], ['create', 'instance'])
                             for call in self.commands.calls))
        self.assertFalse((self.root / 'run/instance-id').exists())
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

    def test_failed_bench_restores_network_without_touching_other_files(self) -> None:
        unrelated = self.root / 'tree/other.txt'
        unrelated.write_text('作業中の変更')
        self.commands.fail_command = [str(self.root / 'tree/target/release/minase'), 'dev', 'bench']
        self.assertNotEqual(self.execute(), 0)
        self.assertEqual((self.root / 'tree/crates/minase/nets/nnue.bin').read_bytes(), network(0))
        self.assertEqual(unrelated.read_text(), '作業中の変更')
        self.assertNotIn('refs/nnue-standard-recipe/gen-1', self.commands.refs)

    def test_commit_already_written_only_creates_reference(self) -> None:
        self.save_book(1, 0, 100)
        fit = self.root / 'run/gen-1/fit'
        fit.mkdir()
        (fit / 'nnue.bin').write_bytes(network(1))
        self.commands.head = '1' * 40
        self.commands.nets[self.commands.head] = network(1)
        runner = chain.Chain(chain.arguments(argv(self.values)))
        runner.commit_generation(1, {'best_epoch': 3}, {0: 100}, -0.1)
        self.assertEqual(self.commands.refs['refs/nnue-standard-recipe/gen-1'], '1' * 40)
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

    def test_books_per_generation_ignores_position_counts(self) -> None:
        """事例5: 局面が0でも本数がそろったら、生成を止めて学習へ進む。"""
        self.values.update({'last-generation': '1', 'books-per-generation': '2'})
        self.commands.entries = 0
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.commands.generated, [(1, 0), (1, 1)])
        self.assertTrue(all(job['target_books'] == 2 for job in self.commands.jobs))

    def test_failed_training_diagnostic_or_fit_stops_before_commit(self) -> None:
        for failure in (['nnue', 'train'], ['diag', 'nnue-removal'], ['nnue', 'fit-linear']):
            with self.subTest(failure=failure):
                self.commands.fail_command = [str(self.root / 'trainer'), *failure]
                self.assertNotEqual(self.execute(), 0)
                self.assertNotIn('refs/nnue-standard-recipe/gen-1', self.commands.refs)
                self.assert_stopped()

    def test_non_candidate_summary_stops_before_commit(self) -> None:
        self.commands.train_status = 'error'
        self.assertNotEqual(self.execute(), 0)
        self.assertNotIn('refs/nnue-standard-recipe/gen-1', self.commands.refs)

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

    def test_cpu_quota_boundary_is_accepted(self) -> None:
        self.commands.cpu['cpu_limit'] = 345.6
        self.assertEqual(self.execute(), 0, self.errors.getvalue())

    def test_affinity_shortage_destroys_before_start(self) -> None:
        self.commands.cpu['nproc'] = 383
        self.assertNotEqual(self.execute(), 0)
        self.assertFalse(self.commands.started)
        self.assertFalse((self.root / 'run/instance-id').exists())


class StandardRecipeTests(ChainTestCase):
    """標準レシピの指示書の事例2〜12を、外部コマンドと成果物で検証する。"""

    def disable_fit(self) -> None:
        self.values['fit'] = 'no'
        for key in ('fit-tau', 'fit-seed', 'fit-threads'):
            del self.values[key]

    def remote_commands(self, generation: int) -> dict[str, list[str]]:
        job = json.loads((self.commands.remote / f'training/gen-{generation}/job.json').read_text())
        return {step['name']: step['command'] for step in job['steps']}

    def local_commands(self, generation: int) -> dict[str, list[str]]:
        removal = str(self.root / f'run/gen-{generation}/removal.json')
        return {'train': self.train_calls(generation)[0], 'fit': self.fit_calls(generation)[0],
                'removal': next(call for call in self.commands.calls
                                if call[1:3] == ['diag', 'nnue-removal'] and removal in call)}

    def test_fit_yes_requires_each_fit_argument(self) -> None:
        """事例2: 補正ありではtau、seed、threadsのどれも省略できない。"""
        for key in ('fit-tau', 'fit-seed', 'fit-threads'):
            with self.subTest(key=key), self.assertRaises(SystemExit) as raised:
                chain.main(argv({name: value for name, value in self.values.items() if name != key}))
            self.assertEqual(raised.exception.code, 2)
        self.assertFalse(self.commands.calls)

    def test_fit_no_rejects_each_fit_argument(self) -> None:
        """事例2: 補正なしで使わない引数を1個でも指定すれば拒否する。"""
        self.disable_fit()
        for key, value in (('fit-tau', ['10', '20']), ('fit-seed', '0'), ('fit-threads', '1')):
            with self.subTest(key=key), self.assertRaises(SystemExit) as raised:
                chain.main(argv(self.values | {key: value}))
            self.assertEqual(raised.exception.code, 2)
        self.assertFalse(self.commands.calls)

    def test_recipe_arguments_reject_invalid_values_before_creating(self) -> None:
        """事例2: リプレイバッファ、補正、学習率、検証間隔、参照名の不正を起動前に拒否する。"""
        cases = [('replay-buffer-generations', '3'), ('fit', 'maybe'), ('lr-linear', '0'), ('lr-linear', 'nan'),
                 ('lr-linear', 'inf'), ('lr-linear', '-1'), ('validation-interval', '-1'),
                 ('books-per-generation', '0'), ('books-per-generation', '1.5')]
        cases.extend(('ref-prefix', prefix) for prefix in
                     ('nnue/x', 'refs/x/', 'refs//x', 'refs/a..b', 'refs/a b', 'refs/a\tb',
                      'refs/.x', 'refs/x.lock', 'refs/x@{y', 'refs/x:y', 'refs/x\\y', 'refs/x[y',
                      'refs/x~y', 'refs/x^y', 'refs/x?y', 'refs/x*y', 'refs/x\x7fy'))
        with patch.object(common, 'call') as call, patch.object(common, 'create') as create:
            for key, value in cases:
                with self.subTest(key=key, value=value), self.assertRaises(SystemExit) as raised:
                    chain.main(argv(self.values | {key: value}))
                self.assertEqual(raised.exception.code, 2)
            call.assert_not_called()
            create.assert_not_called()
        self.assertFalse(self.commands.calls)

    def test_validation_interval_zero_is_accepted(self) -> None:
        """事例2: 途中検証なしの0を、既定値で置き換えず保持する。"""
        args = chain.arguments(argv(self.values | {'validation-interval': '0'}))
        self.assertEqual(args.validation_interval, 0)
        self.assertFalse(self.commands.calls)

    def test_calibration_and_seed_bounds_use_books_per_generation(self) -> None:
        """指定本数を超える較正と、最後の本のシードが2^64になる条件を拒否する。"""
        with self.assertRaises(SystemExit):
            chain.arguments(argv(self.values | {'books-per-generation': '37', 'calibration-books': '38'}))
        values = self.values | {'books-per-generation': '37', 'calibration-books': '37',
                               'seed-generation-step': '1', 'seed-book-step': '1',
                               'seed-base': str(2**64 - 39)}
        args = chain.arguments(argv(values))
        self.assertEqual(chain.Chain(args).seed(2, 36), 2**64 - 1)
        with self.assertRaises(SystemExit):
            chain.arguments(argv(values | {'seed-base': str(2**64 - 38)}))
        self.assertFalse(self.commands.calls)

    def test_two_generations_preserve_legacy_refs_and_use_requested_prefix(self) -> None:
        """事例3: 新しい接頭辞だけを更新し、既存の系列の参照は維持する。"""
        self.values['ref-prefix'] = 'refs/recipe-test/series-a'
        legacy = 'refs/nnue-generation-reset/gen-1'
        self.commands.refs[legacy] = 'f' * 40
        self.commands.nets['f' * 40] = network(9)
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.commands.refs, {legacy: 'f' * 40,
                                              'refs/recipe-test/series-a/gen-0': '0' * 40,
                                              'refs/recipe-test/series-a/gen-1': '1' * 40,
                                              'refs/recipe-test/series-a/gen-2': '2' * 40})
        git = [call[3:] for call in self.commands.calls if call[0] == 'git']
        self.assertEqual([call[1:] for call in git if call[0] == 'update-ref'],
                         [['refs/recipe-test/series-a/gen-0', '0' * 40],
                          ['refs/recipe-test/series-a/gen-1', 'HEAD'],
                          ['refs/recipe-test/series-a/gen-2', 'HEAD']])
        self.assertEqual([call[-1] for call in git if call[:2] == ['bundle', 'create']],
                         ['refs/recipe-test/series-a/gen-0', 'refs/recipe-test/series-a/gen-1'])
        self.assertFalse(any('refs/nnue-generation-reset/' in argument for call in git for argument in call))

    def test_generation_jobs_carry_the_previous_generation_reference(self) -> None:
        """事例4: 各生成ジョブが、bundleから取得する完全な参照名を持つ。"""
        self.values['ref-prefix'] = 'refs/recipe-test/series-b'
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual([(job['generation'], job['reference']) for job in self.commands.jobs],
                         [(1, 'refs/recipe-test/series-b/gen-0'), (2, 'refs/recipe-test/series-b/gen-1')])

    def test_resume_26_books_generates_only_26_through_36(self) -> None:
        """事例5: 既存26本の局面数によらず、37本までの11本だけを生成して受理する。"""
        self.values.update({'last-generation': '1', 'books-per-generation': '37'})
        for number in range(26):
            self.save_book(1, number, 100_000_000)
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.commands.generated, [(1, number) for number in range(26, 37)])
        self.assertEqual(self.commands.jobs[0]['skip_books'], list(range(26)))
        self.assertTrue(all(job['target_books'] == 37 for job in self.commands.jobs))
        inspected = [Path(call[-1]).name for call in self.commands.calls if Path(call[0]).name == 'inspect']
        self.assertEqual(inspected, [f'book-{number}.mnsd' for number in range(26, 37)])
        self.assertEqual({path.name for path in (self.root / 'run/gen-1/teacher').iterdir()},
                         {f'book-{number}.mnsd' for number in range(37)})

    def test_received_book_37_is_rejected_even_after_37_books_are_complete(self) -> None:
        """事例5: 上限外の本はログの有無や目標達成にかかわらず拒否し、リンクしない。"""
        self.values['books-per-generation'] = '37'
        runner = chain.Chain(chain.arguments(argv(self.values)))
        directory = self.root / 'run/gen-1'
        write_book(directory / 'received/book-37.mnsd', header_fields(1, 37, 1))
        for count in (26, 37):
            with self.subTest(count=count):
                for number in range(count):
                    self.save_book(1, number, 0)
                books = {number: 0 for number in range(count)}
                with self.assertRaises(chain.ChainStop):
                    runner.accept_books(1, '0' * 40, '00' * 32, books, False)
                self.assertEqual(len(books), count)
                self.assertFalse((directory / 'teacher/book-37.mnsd').exists())
        self.assertFalse(self.commands.calls)

    def test_verified_books_outside_target_do_not_satisfy_generation(self) -> None:
        """本数だけが合っていても、番号0〜指定本数の直前の集合でなければ止める。"""
        self.values.update({'last-generation': '1', 'books-per-generation': '2'})
        self.save_book(1, 0, 120)
        self.save_book(1, 2, 120)
        runner = chain.Chain(chain.arguments(argv(self.values)))
        with self.assertRaises(chain.ChainStop):
            runner.execute()
        self.assertFalse(self.train_calls(1))

    def test_salvage_does_not_fetch_when_required_book_count_is_complete(self) -> None:
        """事例5: 局面数が0でも、指定本数がそろった学習入力を終了時の回収で増やさない。"""
        self.save_book(1, 0, 0)
        self.commands.refs['refs/nnue-standard-recipe/gen-0'] = '0' * 40
        runner = chain.Chain(chain.arguments(argv(self.values)))
        runner.current = 1
        runner.address = ('192.0.2.1', 2222)
        runner.salvage()
        self.assertFalse(any(call[0] == 'rsync' for call in self.commands.calls))

    def test_machine_deadline_counts_saved_books_in_every_pending_generation(self) -> None:
        """事例6: 未完了の2世代に26本と5本がある場合、残り11本と32本から期限を決める。"""
        self.values['books-per-generation'] = '37'
        for generation, count in ((1, 26), (2, 5)):
            for number in range(count):
                self.save_book(generation, number, 120)
        runner = chain.Chain(chain.arguments(argv(self.values)))
        runner.prepare_machine([1, 2], True)
        # (12分 × (11本 + 32本) × 1.3 / 60 + 学習待ち1.2時間) × 3600 = 44568秒。
        self.assertEqual(int((self.root / 'run/deadline').read_text()), 2_000_001_000 + 44_568)

    def test_replay_buffer_two_local_training_uses_previous_then_current_books(self) -> None:
        """事例7: 2世代分のリプレイバッファで学習するときだけ前世代を加え、各世代の番号は数値順に並べる。"""
        self.values.update({'replay-buffer-generations': '2', 'books-per-generation': '12'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        commands = self.local_commands(2)
        previous = [str(self.root / f'run/gen-1/teacher/book-{number}.mnsd') for number in range(12)]
        current = [str(self.root / f'run/gen-2/teacher/book-{number}.mnsd') for number in range(12)]
        self.assertEqual(option_values(commands['train'], '--data'), previous + current)
        for name in ('removal', 'fit'):
            self.assertEqual(option_values(commands[name], '--data'), current)

    def test_replay_buffer_two_generator_uploads_and_trains_previous_then_current_books(self) -> None:
        """事例7: 空の生成機へ前世代から順に転送し、遠隔パスで学習、診断、補正する。"""
        self.values.update({'train-place': 'generator', 'replay-buffer-generations': '2', 'books-per-generation': '12',
                            'last-generation': '1'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        mark = len(self.commands.calls)
        self.values.update({'first-generation': '2', 'last-generation': '2'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        commands = self.remote_commands(2)
        previous = [f'/workspace/job/teacher/gen-1/book-{number}.mnsd' for number in range(12)]
        current = [f'/workspace/job/teacher/gen-2/book-{number}.mnsd' for number in range(12)]
        self.assertEqual(option_values(commands['train'], '--data'), previous + current)
        for name in ('removal', 'fit'):
            self.assertEqual(option_values(commands[name], '--data'), current)
        pushes = [call[-2:] for call in self.commands.calls[mark:]
                  if call[0] == 'rsync' and '--ignore-existing' in call]
        self.assertEqual(pushes, [[str(self.root / f'run/gen-{generation}/teacher') + '/',
                                   f'root@192.0.2.1:/workspace/job/teacher/gen-{generation}/']
                                  for generation in (1, 2)])
        for number in range(12):
            self.assertEqual((self.commands.remote / f'teacher/gen-1/book-{number}.mnsd').read_bytes(),
                             (self.root / f'run/gen-1/teacher/book-{number}.mnsd').read_bytes())

    def test_replay_buffer_two_rejects_missing_previous_books_before_training(self) -> None:
        """事例7: 前世代の教師がなければリプレイバッファを縮めず、学習前に止める。"""
        self.values.update({'replay-buffer-generations': '2', 'last-generation': '1'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        (self.root / 'run/gen-1/teacher/book-0.mnsd').unlink()
        self.save_book(2, 0, 120)
        runner = chain.Chain(chain.arguments(argv(self.values)))
        with self.assertRaises(chain.ChainStop):
            runner.train(2)
        self.assertFalse(self.train_calls(2))

    def test_replay_buffer_two_checks_previous_books_against_their_generation_commit(self) -> None:
        """事例7: 前世代の教師は、その生成コミットとネットの検査和で照合する。"""
        self.values.update({'replay-buffer-generations': '2', 'last-generation': '1'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.save_book(2, 0, 120)
        runner = chain.Chain(chain.arguments(argv(self.values)))
        for field, value in (('generation_commit', '1' * 40), ('network_checksum', '01' * 32)):
            with self.subTest(field=field):
                write_book(self.root / 'run/gen-1/teacher/book-0.mnsd', header_fields(1, 0, 120) | {field: value})
                with self.assertRaises(chain.ChainStop):
                    runner.train(2)
                self.assertFalse(self.train_calls(2))

    def test_replay_buffer_two_preserves_a_larger_previous_generation(self) -> None:
        """前世代を増量していても、その教師を当世代の指定本数で切り詰めない。"""
        self.values.update({'replay-buffer-generations': '2', 'books-per-generation': '3', 'last-generation': '1'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.values.update({'books-per-generation': '1', 'first-generation': '2', 'last-generation': '2'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(option_values(self.train_calls(2)[0], '--data'),
                         [str(self.root / f'run/gen-1/teacher/book-{number}.mnsd') for number in range(3)]
                         + [str(self.root / 'run/gen-2/teacher/book-0.mnsd')])

    def test_replay_buffer_one_second_generation_uses_only_current_books(self) -> None:
        """事例8: 1世代分のリプレイバッファを使う第2世代は当世代の教師だけを使う。"""
        self.values.update({'replay-buffer-generations': '1', 'books-per-generation': '12'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(option_values(self.train_calls(2)[0], '--data'),
                         [str(self.root / f'run/gen-2/teacher/book-{number}.mnsd') for number in range(12)])

    def test_replay_buffer_two_first_generation_uses_only_current_books(self) -> None:
        """事例8: 2世代分のリプレイバッファでも第1世代には前世代の教師を要求しない。"""
        self.values.update({'replay-buffer-generations': '2', 'books-per-generation': '12', 'last-generation': '1'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(option_values(self.train_calls(1)[0], '--data'),
                         [str(self.root / f'run/gen-1/teacher/book-{number}.mnsd') for number in range(12)])

    def test_fit_no_local_commits_trained_network_without_fitting_or_d_check(self) -> None:
        """事例9: 古い補正結果のDが正でも検査せず、当世代の学習ネットをコミットする。"""
        self.disable_fit()
        self.values['last-generation'] = '1'
        fit = self.root / 'run/gen-1/fit'
        fit.mkdir(parents=True)
        (fit / 'summary.json').write_text('{"decision": {"d": 1}}')
        (fit / 'nnue.bin').write_bytes(network(9))
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(len(self.train_calls(1)), 1)
        self.assertFalse(self.fit_calls(1))
        self.assertEqual(self.commands.nets[self.commands.head],
                         (self.root / 'run/gen-1/train/nnue.bin').read_bytes())
        commit = next(call for call in self.commands.calls if call[0] == 'git' and call[3] == 'commit')
        self.assertIn('教師1本、120局面、最良エポック3。', commit)
        self.assertFalse(any('D=' in argument for argument in commit))

    def test_fit_no_generator_completes_and_receives_without_fit_artifacts(self) -> None:
        """事例9: 補正ステップと成果物を要求せず、trainとremovalだけで完了する。"""
        self.disable_fit()
        self.values.update({'train-place': 'generator', 'last-generation': '1'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual([step['name'] for step in self.commands.training_jobs[0]['steps']],
                         ['build', 'train', 'removal'])
        self.assertFalse((self.commands.remote / 'training/gen-1/fit').exists())
        self.assertFalse((self.root / 'run/gen-1/fit').exists())
        runner = chain.Chain(chain.arguments(argv(self.values)))
        self.assertTrue(runner.training_complete(1))
        self.assertEqual(self.commands.nets[self.commands.head],
                         (self.root / 'run/gen-1/train/nnue.bin').read_bytes())
        commit = next(call for call in self.commands.calls if call[0] == 'git' and call[3] == 'commit')
        self.assertIn('教師1本、120局面、最良エポック4。', commit)
        self.assertFalse(any('D=' in argument for argument in commit))
        (self.root / 'run/gen-1/removal.json').unlink()
        self.assertFalse(runner.training_complete(1))

    def test_linear_learning_rate_and_zero_validation_reach_local_training(self) -> None:
        """事例10: 作業機の学習コマンドに線形の学習率と検証間隔0を渡す。"""
        self.values.update({'last-generation': '1', 'lr-linear': '0.67903', 'validation-interval': '0'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        command = self.train_calls(1)[0]
        self.assertEqual(option_values(command, '--lr-linear'), ['0.67903'])
        self.assertEqual(option_values(command, '--validation-interval'), ['0'])

    def test_linear_learning_rate_and_zero_validation_reach_generator_training(self) -> None:
        """事例10: 生成機の学習ジョブにも線形の学習率と検証間隔0を渡す。"""
        self.values.update({'train-place': 'generator', 'last-generation': '1',
                            'lr-linear': '0.67903', 'validation-interval': '0'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        command = self.remote_commands(1)['train']
        self.assertEqual(option_values(command, '--lr-linear'), ['0.67903'])
        self.assertEqual(option_values(command, '--validation-interval'), ['0'])

    def test_fit_no_two_local_generations_use_previous_trained_network_as_base(self) -> None:
        """事例11: 補正なしの第2世代は、第1世代でコミットした学習ネットを引き継ぐ。"""
        self.disable_fit()
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        base = self.root / 'run/gen-1/train/nnue.bin'
        self.assertEqual(option_values(self.train_calls(2)[0], '--base'), [str(base)])
        self.assertEqual(base.read_bytes(), self.commands.nets['1' * 40])
        self.assertEqual(self.commands.refs['refs/nnue-standard-recipe/gen-2'], '2' * 40)

    def test_fit_no_two_generator_generations_upload_previous_trained_network_as_base(self) -> None:
        """事例11: 補正なしの第2世代に、第1世代の学習ネットをbase.binとして送る。"""
        self.disable_fit()
        self.values['train-place'] = 'generator'
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        base = self.root / 'run/gen-1/train/nnue.bin'
        self.assertEqual(option_values(self.remote_commands(2)['train'], '--base'),
                         ['/workspace/job/training/gen-2/base.bin'])
        self.assertIn(['scp', '192.0.2.1', '2222', str(base), '/workspace/job/training/gen-2/base.bin'],
                      self.commands.calls)
        self.assertEqual((self.commands.remote / 'training/gen-2/base.bin').read_bytes(), base.read_bytes())
        self.assertEqual(base.read_bytes(), self.commands.nets['1' * 40])
        self.assertEqual(self.commands.refs['refs/nnue-standard-recipe/gen-2'], '2' * 40)

    def assert_fit_switch_rejects_uncommitted_base(self) -> None:
        self.values['last-generation'] = '1'
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.disable_fit()
        self.values.update({'first-generation': '2', 'last-generation': '2'})
        runner = chain.Chain(chain.arguments(argv(self.values)))
        with self.assertRaises(chain.ChainStop):
            runner.execute()
        self.assertFalse(self.train_calls(2))
        self.assertFalse((self.commands.remote / 'training/gen-2/job.json').exists())
        self.assertNotIn('refs/nnue-standard-recipe/gen-2', self.commands.refs)

    def test_changed_fit_rejects_uncommitted_base_before_local_training(self) -> None:
        """事例12: 補正ありからなしへ変えたとき、未コミットの重みでは作業機で学ばない。"""
        self.assert_fit_switch_rejects_uncommitted_base()

    def test_changed_fit_rejects_uncommitted_base_before_generator_training(self) -> None:
        """事例12: 生成機へ学習ジョブを公開する前にも、前世代のネット全体を照合する。"""
        self.values['train-place'] = 'generator'
        self.assert_fit_switch_rejects_uncommitted_base()


class GeneratorTrainingTests(ChainTestCase):
    """生成機での学習、取得、停止、再開を、指示書の順序と外部入出力で検証する。"""

    def setUp(self) -> None:
        super().setUp()
        self.values.update({'train-place': 'generator', 'last-generation': '1'})

    def saved_machine(self) -> None:
        directory = self.root / 'run'
        directory.mkdir(exist_ok=True)
        (directory / 'instance-id').write_text('17\n')
        (directory / 'ssh-address').write_text('192.0.2.1 2222\n')
        del self.values['offer']

    def completed_books(self) -> None:
        self.save_book(1, 0, 100)
        (self.root / 'run/base.bin').write_bytes(network(0))

    def assert_no_local_training(self) -> None:
        self.assertEqual([call for call in self.commands.calls
                          if call[0] == str(self.root / 'trainer') and call[1:3] in
                          (['nnue', 'train'], ['diag', 'nnue-removal'], ['nnue', 'fit-linear'])], [])

    def assert_no_commit(self) -> None:
        self.assertNotIn('refs/nnue-standard-recipe/gen-1', self.commands.refs)
        self.assertFalse(any(call[0] == 'git' and call[3] == 'commit' for call in self.commands.calls))

    def test_generator_receives_fitted_network_before_stopping_and_committing(self) -> None:
        """事例1: 検査済みの本だけを番号順に渡し、全結果の取得後に停止して補正済みネットを使う。"""
        self.values['books-per-generation'] = '3'
        for number in (2, 0, 1):
            self.save_book(1, number, 40)
        self.saved_machine()
        write_book(self.commands.remote / 'teacher/gen-1/book-9.mnsd', header_fields(1, 9, 120))
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(self.commands.jobs[-1]['target_books'], 3)
        self.assertEqual(self.commands.jobs[-1]['skip_books'], [0, 1, 2])
        self.assertEqual(len(self.commands.training_jobs), 1)
        steps = self.commands.training_jobs[0]['steps']
        directory = '/workspace/job/training/gen-1'
        trainer = '/workspace/job/minase/target/release/minase-trainer'
        books = [f'/workspace/job/teacher/gen-1/book-{number}.mnsd' for number in (0, 1, 2)]
        expected = [
            ['cargo', 'build', '--release', '--locked', '--bin', 'minase-trainer'],
            [trainer, 'nnue', 'train', '--data', *books, '--base', directory + '/base.bin',
             '--lambda', '1.0', '--teacher-cp-per-logit', 'calibrate', '--seed', '1', '--threads', '8',
             '--output-dir', directory + '/train', '--feature-set', 'P', '--width', '256',
             '--head', 'pairwise-l16-l32', '--batch', '16384', '--epochs', '50', '--patience', '5',
             '--device', 'cuda', '--validation-interval', '1000', '--lr-schedule', 'cosine:3:0.1',
             '--lr-linear', '0.67903'],
            [trainer, 'diag', 'nnue-removal', '--float', directory + '/train/best.npz',
             '--net', directory + '/train/nnue.bin', '--data', *books, '--sample', '10', '--seed', '3',
             '--output', directory + '/removal.json'],
            [trainer, 'nnue', 'fit-linear', '--base', directory + '/train/nnue.bin', '--data', *books,
             '--tau', '0.5', '1.5', '--seed', '0', '--threads', '3', '--output-dir', directory + '/fit'],
        ]
        self.assertEqual([step['name'] for step in steps], ['build', 'train', 'removal', 'fit'])
        self.assertEqual([step['command'] for step in steps], expected)
        self.assertEqual([step['done'] for step in steps],
                         [None, directory + '/train/summary.json', directory + '/removal.json',
                          directory + '/fit/summary.json'])
        self.assertEqual([step['clean'] for step in steps], [None, directory + '/train', None, directory + '/fit'])
        self.assertEqual([step['resume_file'] for step in steps],
                         [None, directory + '/train/latest.npz', None, None])
        self.assertEqual([step['resume_args'] for step in steps],
                         [None, ['--resume', directory + '/train'], None, None])
        calls = self.commands.calls
        received = next(index for index, call in enumerate(calls)
                        if call[0] == 'rsync' and call[-1] == str(self.root / 'run/gen-1/training-received') + '/')
        stopped = next(index for index, call in enumerate(calls)
                       if call[0] == 'ssh' and call[-1].endswith('generation_worker.py stop'))
        committed = next(index for index, call in enumerate(calls) if call[0] == 'git' and call[3] == 'commit')
        self.assertLess(received, stopped)
        self.assertLess(stopped, committed)
        fitted = (self.root / 'run/gen-1/fit/nnue.bin').read_bytes()
        self.assertEqual(self.commands.nets[self.commands.head], fitted)
        self.assertNotEqual(fitted, (self.root / 'run/gen-1/train/nnue.bin').read_bytes())
        self.assertEqual((self.root / 'run/gen-1/training-logs/train.log').read_text(), '生成機での学習ログ\n')
        self.assertFalse((self.root / 'run/gen-1/training-received').exists())
        self.assertTrue(any(call[1:3] == ['dev', 'bench'] for call in calls))
        self.assert_no_local_training()

    def test_training_worker_upload_replaces_final_file_after_transfer(self) -> None:
        """学習用の作業者は一時名へ転送してからmvで公開する。"""
        self.completed_books()
        self.saved_machine()
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        calls = self.commands.calls
        uploads = [index for index, call in enumerate(calls)
                   if call[0] == 'scp' and Path(call[-2]).name == 'generation_worker.py']
        self.assertEqual(len(uploads), 1)
        uploaded = uploads[0]
        self.assertEqual(calls[uploaded][-1], '/workspace/job/generation_worker.py.tmp')
        replacements = [index for index, call in enumerate(calls)
                        if call[0] == 'ssh' and call[-1] ==
                        'mv /workspace/job/generation_worker.py.tmp /workspace/job/generation_worker.py']
        self.assertEqual(len(replacements), 1)
        self.assertLess(uploaded, replacements[0])
        self.assertEqual((self.commands.remote / 'generation_worker.py').read_bytes(),
                         Path(worker.__file__).read_bytes())
        self.assertFalse((self.commands.remote / 'generation_worker.py.tmp').exists())

    def test_training_failure_stops_without_committing_or_local_training(self) -> None:
        """事例2: 生成機の終了コードとログの場所を示して止め、localでの再開を案内する。"""
        self.commands.training_exit = 7
        self.assertEqual(self.execute(), 1)
        self.assert_stopped()
        self.assert_no_commit()
        self.assert_no_local_training()
        for value in ('終了コード7', str(self.root / 'run/gen-1/training-logs'), '--train-place local'):
            self.assertIn(value, self.errors.getvalue())

    def test_training_timeout_stops_without_committing(self) -> None:
        """事例3: 接続できても学習の待ち時間を超えたら停止する。"""
        self.values['training-wait-hours'] = '0.001'
        self.commands.training_stall = True
        self.assertEqual(self.execute(), 1)
        self.assertEqual(sum(self.clock.sleeps), 4)
        self.assertIn('学習待ちが上限時間', self.errors.getvalue())
        self.assertIn('--train-place local', self.errors.getvalue())
        self.assert_stopped()
        self.assert_no_commit()
        self.assert_no_local_training()

    def test_interrupted_training_reuses_published_job_and_base(self) -> None:
        """事例4: 中断では生成機を止めず、再実行は仕事と初期値を送り直さずに完了する。"""
        self.commands.interrupt_training = True
        self.assertEqual(self.execute(), 128 + signal.SIGTERM)
        self.assertFalse(self.commands.stopped)
        self.assertTrue((self.root / 'run/instance-id').is_file())
        self.assertFalse(any(call[:3] == ['vastai', 'destroy', 'instance'] for call in self.commands.calls))
        job = (self.commands.remote / 'training/gen-1/job.json').read_bytes()
        base = (self.commands.remote / 'training/gen-1/base.bin').read_bytes()
        mark = len(self.commands.calls)
        self.commands.interrupt_training = False
        del self.values['offer']
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        later = self.commands.calls[mark:]
        self.assertFalse(any(call[0] == 'scp' and call[-1] in
                             ('/workspace/job/training/gen-1/job.json.tmp', '/workspace/job/training/gen-1/base.bin')
                             for call in later))
        self.assertFalse(any(call[0] == 'rsync' and '--ignore-existing' in call for call in later))
        self.assertEqual((self.commands.remote / 'training/gen-1/job.json').read_bytes(), job)
        self.assertEqual((self.commands.remote / 'training/gen-1/base.bin').read_bytes(), base)
        self.assertEqual(self.commands.refs['refs/nnue-standard-recipe/gen-1'], '1' * 40)
        self.assertEqual(len(self.commands.training_jobs), 1)
        self.assert_no_local_training()

    def test_completed_books_without_machine_stop_without_account_queries(self) -> None:
        """事例5: 本だけがある場合は出品指定の有無にかかわらず借りず、localを案内する。"""
        self.completed_books()
        for offer in (True, False):
            with self.subTest(offer=offer):
                if not offer:
                    del self.values['offer']
                runner = chain.Chain(chain.arguments(argv(self.values)))
                with self.assertRaisesRegex(chain.ChainStop, '--train-place local'):
                    runner.execute()
                self.assertFalse(any(call[0] in ('vastai', 'ssh', 'scp', 'rsync') for call in self.commands.calls))
                self.assert_no_local_training()
                self.assert_no_commit()

    def test_completed_results_skip_remote_training(self) -> None:
        """事例7: 3つの要約がそろっていれば生成機に接続せず、検査とコミットを行う。"""
        self.completed_books()
        write_training_result(self.root / 'run/gen-1', 1, True)
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertFalse(any(call[0] in ('vastai', 'ssh', 'scp', 'rsync') for call in self.commands.calls))
        self.assertEqual(self.commands.refs['refs/nnue-standard-recipe/gen-1'], '1' * 40)
        self.assert_no_local_training()

    def test_received_results_resume_stops_via_saved_ssh_when_account_fails(self) -> None:
        """最終結果の取得後に再開すると、接続確認を挟まず保存先へSSHで停止を要求する。"""
        self.completed_books()
        self.saved_machine()
        write_training_result(self.root / 'run/gen-1', 1, True)
        self.commands.fail_command = ['vastai']
        self.assertEqual(self.execute(), 1)
        calls = self.commands.calls
        ssh_calls = [call for call in calls if call[0] == 'ssh']
        self.assertTrue(ssh_calls)
        self.assertEqual(ssh_calls[0][-2:], ['root@192.0.2.1', 'python3 /workspace/job/generation_worker.py stop'])
        self.assertEqual(ssh_calls[0][ssh_calls[0].index('-p') + 1], '2222')
        self.assertTrue(self.commands.stopped)
        self.assertTrue(any(call[:3] == ['vastai', 'destroy', 'instance'] for call in calls))
        self.assertFalse(any(call[0] in ('scp', 'rsync') for call in calls))
        self.assertEqual(self.commands.refs['refs/nnue-standard-recipe/gen-1'], '1' * 40)
        self.assertIn('削除が未確認', self.errors.getvalue())
        self.assert_no_local_training()

    def test_result_move_interruption_fetches_again_and_replaces_old_outputs(self) -> None:
        """事例8: trainの移動直後に中断しても完了扱いせず、全結果を取得し直す。"""
        self.completed_books()
        self.saved_machine()
        directory = self.root / 'run/gen-1'
        write_training_result(directory, 1, True)
        (directory / 'removal.json').unlink()
        (directory / 'train/stale').touch()
        (directory / 'fit/stale').touch()
        replace = os.replace

        def interrupt(source: Path, destination: Path) -> None:
            replace(source, destination)
            if Path(destination) == directory / 'train':
                raise chain.Interrupted(signal.SIGTERM)

        with patch.object(chain.os, 'replace', side_effect=interrupt):
            self.assertEqual(self.execute(), 128 + signal.SIGTERM)
        self.assertTrue((directory / 'train/summary.json').is_file())
        self.assertFalse((directory / 'removal.json').exists())
        self.assertFalse((directory / 'fit').exists())
        self.assertFalse(chain.Chain(chain.arguments(argv(self.values))).training_complete(1))
        self.assertFalse(self.commands.stopped)
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        received = [call for call in self.commands.calls if call[0] == 'rsync'
                    and call[-1] == str(directory / 'training-received') + '/']
        self.assertEqual(len(received), 2)
        self.assertFalse((directory / 'train/stale').exists())
        self.assertFalse((directory / 'fit/stale').exists())
        self.assertFalse((directory / 'training-received').exists())
        self.assertEqual(self.commands.refs['refs/nnue-standard-recipe/gen-1'], '1' * 40)
        self.assert_no_local_training()

    def test_replacement_machine_receives_previously_validated_books(self) -> None:
        """事例9: 借り直した生成機にない本を送り、初期値には前世代の補正結果を使う。"""
        saved = self.save_book(1, 0, 40).read_bytes()
        (self.root / 'run/instance-id').write_text('99\n')
        self.commands.entries = 40
        self.values.update({'last-generation': '2', 'books-per-generation': '3'})
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        pushes = [call for call in self.commands.calls if call[0] == 'rsync' and '--ignore-existing' in call]
        self.assertEqual(len(pushes), 2)
        self.assertEqual(pushes[0][-2:], [str(self.root / 'run/gen-1/teacher') + '/',
                                        'root@192.0.2.1:/workspace/job/teacher/gen-1/'])
        self.assertEqual((self.commands.remote / 'teacher/gen-1/book-0.mnsd').read_bytes(), saved)
        self.assertNotIn((1, 0), self.commands.generated)
        self.assertEqual((self.commands.remote / 'training/gen-1/base.bin').read_bytes(),
                         (self.root / 'run/base.bin').read_bytes())
        self.assertEqual((self.commands.remote / 'training/gen-2/base.bin').read_bytes(),
                         (self.root / 'run/gen-1/fit/nnue.bin').read_bytes())
        self.assert_no_local_training()

    def test_done_check_transport_failures_retry_and_recover(self) -> None:
        """事例10: doneの照会の失敗も再試行し、短い障害では学習結果を取得できる。"""
        self.completed_books()
        self.saved_machine()
        self.commands.training_done_failures = 2
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertGreaterEqual(sum(self.clock.sleeps), 2)
        self.assertEqual(len(self.commands.training_jobs), 1)
        self.assert_no_local_training()

    def test_done_check_transport_failures_stop_at_limit(self) -> None:
        """事例10: 他の通信が成功していても、doneの照会の連続失敗を上限で打ち切る。"""
        self.completed_books()
        self.saved_machine()
        self.commands.training_done_failures = 20
        self.assertEqual(self.execute(), 1)
        self.assertEqual(sum(self.clock.sleeps), 3)
        self.assertIn('SSHまたはrsyncの失敗が上限時間', self.errors.getvalue())
        self.assertIn('--train-place local', self.errors.getvalue())
        self.assert_stopped()
        self.assert_no_commit()
        self.assert_no_local_training()

    def test_teacher_upload_failure_retries_preparation(self) -> None:
        """準備中の教師転送が失敗しても、job.jsonを公開するまで再試行する。"""
        self.completed_books()
        self.saved_machine()
        self.commands.transfer_failures = 1
        self.assertEqual(self.execute(), 0, self.errors.getvalue())
        self.assertEqual(len(self.commands.training_jobs), 1)
        self.assert_no_local_training()

    def test_missing_received_summary_never_starts_local_training(self) -> None:
        """結果のdoneだけでは完了にせず、不足を検出したら作業機の学習器を呼ばず停止する。"""
        self.completed_books()
        self.saved_machine()
        self.commands.training_missing = ['removal.json']
        stale = self.root / 'run/gen-1/training-received'
        write_training_result(stale, 1, True)
        self.assertEqual(self.execute(), 1)
        self.assertIn('学習結果がそろっていません', self.errors.getvalue())
        self.assert_stopped()
        self.assert_no_commit()
        self.assert_no_local_training()

    def test_gpu_offer_requirements_apply_only_to_generator_training(self) -> None:
        """事例11: GPU数とCUDA版の下限を生成機学習だけに適用し、作成前に拒否する。"""
        original = self.commands.offer.copy()
        for field, value in (('num_gpus', 0), ('cuda_max_good', 12.9)):
            with self.subTest(field=field):
                self.commands.offer = original | {field: value}
                self.assertEqual(self.execute(), 1)
                self.assertFalse(any(call[:3] == ['vastai', 'create', 'instance'] for call in self.commands.calls))
                local = chain.arguments(argv(self.values | {'train-place': 'local'}))
                self.assertEqual(chain.select_offer([self.commands.offer], local)['id'], 42)
        self.commands.offer = original
        self.assertEqual(self.execute(), 0, self.errors.getvalue())

    def test_deadline_includes_last_training_only_on_generator(self) -> None:
        """事例12: 生成時間が57.72時間なら6世代の学習待ち7.2時間または6時間を加える。"""
        args = chain.arguments(argv(self.values))
        args.books_per_generation = 37
        self.assertAlmostEqual(chain.deadline_hours([0] * 6, args), 64.92)
        self.assertAlmostEqual(chain.deadline_hours([37], args), 1.2)
        args.train_place = 'local'
        self.assertAlmostEqual(chain.deadline_hours([0] * 6, args), 63.72)
        with self.assertRaises(chain.ChainStop):
            chain.deadline_hours([37], args)


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
                    'reference': 'refs/nnue-standard-recipe/gen-0',
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

    def test_prepare_commit_fetches_the_exact_job_reference(self) -> None:
        """事例4: 作業者は名前空間を補わず、ジョブで受け取った完全な参照名を取得する。"""
        self.job['reference'] = 'refs/recipe-test/worker/gen-0'
        with patch.object(subprocess, 'run', side_effect=self.command):
            worker.prepare_commit(self.root, self.job, io.StringIO())
        self.assertEqual([call for call in self.calls if call[:2] == ['git', 'fetch']],
                         [['git', 'fetch', '/workspace/job/bundles/gen-0.bundle',
                           'refs/recipe-test/worker/gen-0']])
        self.assertFalse(any('refs/nnue-generation-reset/' in argument
                             for call in self.calls if call[0] == 'git' for argument in call))

    def test_missing_job_reference_stops_worker_and_writes_exit_status(self) -> None:
        """事例4: 参照名の欠落は生成待ちでも拒否し、終了コードを公開する。"""
        del self.job['reference']
        for skipped in ([0], [0, 1, 2]):
            with self.subTest(skipped=skipped):
                (self.root / 'job.json').write_text(json.dumps(self.job | {'skip_books': skipped}))
                with patch.object(subprocess, 'run', side_effect=self.command), \
                        patch.object(worker, 'stop_instance') as stop:
                    self.assertEqual(worker.run_worker(self.root), 1)
                self.assertEqual((self.root / 'logs/exit-status').read_text(), '1\n')
                self.assertFalse(list(self.root.glob('teacher/**/*.mnsd')))
                self.assertFalse(self.calls)
                stop.assert_not_called()
        with self.assertRaises(KeyError), patch.object(subprocess, 'run', side_effect=self.command):
            worker.prepare_commit(self.root, self.job, io.StringIO())

    def test_next_book_resumes_26_verified_books_until_37_are_complete(self) -> None:
        """事例5: 26本が検査済みなら番号26から36を返し、番号37へ進まない。"""
        self.job.update({'target_books': 37, 'skip_books': list(range(26))})
        teacher = self.root / 'teacher/gen-1'
        teacher.mkdir(parents=True)
        for number in range(26, 37):
            self.assertEqual(worker.next_book(self.job, self.root), number)
            (teacher / f'book-{number}.mnsd').touch()
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
        self.assertIn(['git', 'fetch', '/workspace/job/bundles/gen-0.bundle', 'refs/nnue-standard-recipe/gen-0'], self.calls)

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


class TrainingWorkerTests(unittest.TestCase):
    """事例13〜17: 世代ごとの排他制御、ステップの再開、終了状態の公開を検証する。"""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.directory = self.root / 'training/gen-3'
        self.directory.mkdir(parents=True)
        self.library = self.root / 'nvrtc/nvidia/cu13/lib'
        self.install = self.enterContext(patch.object(worker, 'install_nvrtc', return_value=self.library))
        self.steps = []
        for name, done in (('build', None), ('train', 'train/summary.json'),
                           ('removal', 'removal.json'), ('fit', 'fit/summary.json')):
            self.steps.append({'name': name, 'command': ['tool', name],
                               'done': str(self.directory / done) if done is not None else None,
                               'clean': str(self.directory / name) if name in ('train', 'fit') else None,
                               'resume_file': str(self.directory / 'train/latest.npz') if name == 'train' else None,
                               'resume_args': ['--resume', str(self.directory / 'train')] if name == 'train' else None})
        (self.directory / 'job.json').write_text(json.dumps({'steps': self.steps}))
        self.calls = []
        self.failure = None
        self.missing_done = None
        self.errors = io.StringIO()
        self.enterContext(redirect_stderr(self.errors))
        self.enterContext(patch.object(subprocess, 'run', side_effect=self.command))
        self.enterContext(patch.object(subprocess, 'Popen', side_effect=AssertionError('実コマンドは禁止です')))

    def command(self, command: list[str], **kwargs: object) -> subprocess.CompletedProcess:
        self.calls.append((command, kwargs))
        name = command[1]
        self.assertFalse((self.directory / 'done').exists())
        self.assertFalse((self.directory / 'exit-status').exists())
        self.assertFalse((self.directory / 'finished' / name).exists())
        self.assertEqual(kwargs['cwd'], self.root / 'minase')
        self.assertEqual(kwargs['stderr'], subprocess.STDOUT)
        self.assertTrue(kwargs['check'])
        self.assertEqual(Path(kwargs['stdout'].name), self.directory / f'logs/{name}.log')
        kwargs['stdout'].write(name + '\n')
        if name == 'train' and self.failure is not None:
            raise subprocess.CalledProcessError(self.failure, command)
        if name == 'train' and '--resume' not in command:
            self.assertFalse((self.directory / 'train').exists())
        if name == 'fit':
            self.assertFalse((self.directory / 'fit').exists())
        if name != 'build' and name != self.missing_done:
            relative = {'train': 'train/summary.json', 'removal': 'removal.json', 'fit': 'fit/summary.json'}[name]
            done = self.directory / relative
            done.parent.mkdir(parents=True, exist_ok=True)
            done.write_text('{}')
        return subprocess.CompletedProcess(command, 0)

    def test_steps_run_in_order_with_environment_and_publish_done_last(self) -> None:
        """事例13: 全ステップに環境を渡し、追記ログと成果物の完成後にdoneを置く。"""
        (self.directory / 'logs').mkdir()
        (self.directory / 'logs/train.log').write_text('previous\n')
        publications = []
        replace = Path.replace

        def publish(source: Path, destination: Path) -> Path:
            publications.append(destination)
            self.assertEqual(destination, self.directory / 'done')
            self.assertEqual([command[1] for command, _ in self.calls], ['build', 'train', 'removal', 'fit'])
            self.assertTrue(source.is_file())
            self.assertFalse(destination.exists())
            for name in ('train/summary.json', 'removal.json', 'fit/summary.json'):
                self.assertTrue((self.directory / name).is_file())
            return replace(source, destination)

        with patch.dict(os.environ, {'LD_LIBRARY_PATH': '/existing/lib', 'TRAINING_TEST_VALUE': 'retained'}), \
                patch.object(Path, 'replace', publish):
            self.assertEqual(worker.run_training(self.root, 3), 0)
        self.assertEqual(publications, [self.directory / 'done'])
        self.assertTrue((self.directory / 'done').is_file())
        self.assertFalse((self.directory / '.done.tmp').exists())
        self.assertEqual({path.name for path in (self.directory / 'finished').iterdir()}, {'train', 'removal', 'fit'})
        for _, options in self.calls:
            self.assertEqual(options['env']['LD_LIBRARY_PATH'], str(self.library) + ':/existing/lib')
            self.assertEqual(options['env']['TRAINING_TEST_VALUE'], 'retained')
        self.assertEqual((self.directory / 'logs/train.log').read_text(), 'previous\ntrain\n')
        self.install.assert_called_once_with(self.root)

    def test_environment_without_library_path_gets_nvrtc_directory(self) -> None:
        """LD_LIBRARY_PATHが未設定でも、空の探索先を足さずにNVRTCの場所を渡す。"""
        with patch.dict(os.environ, {'TRAINING_TEST_VALUE': 'retained'}, clear=True):
            self.assertEqual(worker.run_training(self.root, 3), 0)
        for _, options in self.calls:
            self.assertEqual(options['env']['LD_LIBRARY_PATH'], str(self.library))
            self.assertEqual(options['env']['TRAINING_TEST_VALUE'], 'retained')

    def test_output_without_finished_marker_does_not_skip_step(self) -> None:
        """成果物だけでは成功と判定せず、目印のないステップを実行する。"""
        for relative in ('train/summary.json', 'removal.json', 'fit/summary.json'):
            path = self.directory / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('{}')
        self.assertEqual(worker.run_training(self.root, 3), 0, self.errors.getvalue())
        self.assertEqual([command[1] for command, _ in self.calls], ['build', 'train', 'removal', 'fit'])

    def test_finished_steps_skip_cleaning_and_null_done_runs_each_time(self) -> None:
        """事例14: 目印があれば実行と削除を省き、doneがnullなら毎回実行して目印を作らない。"""
        train = self.directory / 'train'
        train.mkdir()
        (train / 'summary.json').write_text('{}')
        (train / 'latest.npz').touch()
        (self.directory / 'finished').mkdir()
        (self.directory / 'finished/train').touch()
        fit = self.directory / 'fit'
        fit.mkdir()
        (fit / 'partial').touch()
        self.assertEqual(worker.run_training(self.root, 3), 0)
        self.assertEqual([command for command, _ in self.calls], [['tool', 'build'], ['tool', 'removal'], ['tool', 'fit']])
        self.assertFalse((fit / 'partial').exists())
        (fit / 'retained').touch()
        (self.directory / 'done').unlink()
        (train / 'summary.json').unlink()
        self.calls.clear()
        self.assertEqual(worker.run_training(self.root, 3), 0)
        self.assertEqual([command for command, _ in self.calls], [['tool', 'build']])
        self.assertTrue((train / 'latest.npz').is_file())
        self.assertTrue((fit / 'retained').is_file())
        self.assertFalse((self.directory / 'finished/build').exists())

    def test_training_without_checkpoint_cleans_config_before_running(self) -> None:
        """チェックポイントの保存前に中断したtrainは、config.jsonを消して最初から実行する。"""
        train = self.directory / 'train'
        train.mkdir()
        (train / 'config.json').write_text('{}')
        self.assertEqual(worker.run_training(self.root, 3), 0)
        self.assertIn(['tool', 'train'], [command for command, _ in self.calls])
        self.assertFalse((train / 'config.json').exists())

    def test_training_with_checkpoint_preserves_output_and_resumes(self) -> None:
        """latest.npzがあれば出力を消さず、resume_argsを付けて再開する。"""
        train = self.directory / 'train'
        train.mkdir()
        (train / 'config.json').write_text('{"seed": 1}')
        (train / 'latest.npz').write_bytes(b'checkpoint')
        self.assertEqual(worker.run_training(self.root, 3), 0)
        self.assertIn(['tool', 'train', '--resume', str(train)], [command for command, _ in self.calls])
        self.assertEqual((train / 'config.json').read_text(), '{"seed": 1}')
        self.assertEqual((train / 'latest.npz').read_bytes(), b'checkpoint')

    def test_steps_inherit_generation_lock(self) -> None:
        """各ステップのpass_fdsに、保持中の世代ロックの記述子を渡す。"""
        def command(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess:
            self.assertIn('pass_fds', list(kwargs))
            self.assertIsInstance(kwargs['pass_fds'], tuple)
            self.assertEqual(len(kwargs['pass_fds']), 1)
            self.assertTrue(os.path.samestat(os.fstat(kwargs['pass_fds'][0]), (self.directory / 'lock').stat()))
            return self.command(argv, **kwargs)

        with patch.object(subprocess, 'run', side_effect=command):
            self.assertEqual(worker.run_training(self.root, 3), 0, self.errors.getvalue())
        self.assertEqual([argv[1] for argv, _ in self.calls], ['build', 'train', 'removal', 'fit'])

    def test_failed_diagnostic_report_does_not_mark_step_finished(self) -> None:
        """失敗時の報告には成功の目印を付けず、再開時には診断を実行し直す。"""
        def command(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess:
            result = self.command(argv, **kwargs)
            if argv[1] == 'removal':
                raise subprocess.CalledProcessError(7, argv)
            return result

        with patch.object(subprocess, 'run', side_effect=command):
            self.assertEqual(worker.run_training(self.root, 3), 7)
        self.assertTrue((self.directory / 'removal.json').is_file())
        self.assertFalse((self.directory / 'finished/removal').exists())
        self.assertFalse((self.directory / 'done').exists())
        (self.directory / 'exit-status').unlink()
        self.calls.clear()
        self.assertEqual(worker.run_training(self.root, 3), 0)
        self.assertEqual([argv[1] for argv, _ in self.calls], ['build', 'removal', 'fit'])
        self.assertTrue((self.directory / 'finished/removal').is_file())

    def test_step_failure_publishes_exit_status_and_does_not_run_later_steps(self) -> None:
        """事例15: 失敗とシグナル終了のコードを1行で公開し、後続を動かさない。"""
        replace = Path.replace
        for failure, expected in ((7, 7), (-9, 137)):
            with self.subTest(failure=failure):
                self.failure = failure
                self.calls.clear()
                (self.directory / 'exit-status').unlink(missing_ok=True)
                publications = []

                def publish(source: Path, destination: Path) -> Path:
                    publications.append(destination)
                    self.assertEqual(destination, self.directory / 'exit-status')
                    self.assertEqual(source.read_text(), f'{expected}\n')
                    self.assertFalse(destination.exists())
                    return replace(source, destination)

                with patch.object(Path, 'replace', publish):
                    self.assertEqual(worker.run_training(self.root, 3), expected)
                self.assertEqual([command[1] for command, _ in self.calls], ['build', 'train'])
                self.assertEqual(publications, [self.directory / 'exit-status'])
                self.assertEqual((self.directory / 'exit-status').read_text(), f'{expected}\n')
                self.assertFalse((self.directory / '.exit-status.tmp').exists())
                self.assertFalse((self.directory / 'done').exists())
                self.assertFalse((self.directory / 'finished/train').exists())

    def test_missing_step_output_is_failure(self) -> None:
        """事例16: 終了コード0でも指定した完了ファイルがなければ失敗する。"""
        self.missing_done = 'train'
        self.assertEqual(worker.run_training(self.root, 3), 1)
        self.assertEqual([command[1] for command, _ in self.calls], ['build', 'train'])
        self.assertEqual((self.directory / 'exit-status').read_text(), '1\n')
        self.assertFalse((self.directory / 'done').exists())
        self.assertFalse((self.directory / 'finished/train').exists())
        self.assertIn('完了ファイルがありません', self.errors.getvalue())

    def test_existing_lock_prevents_installation_and_execution(self) -> None:
        """事例17: 別の呼び出しが世代のロックを持つ間は、何も実行せず0を返す。"""
        with (self.directory / 'lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(worker.run_training(self.root, 3), 0)
        self.assertFalse(self.calls)
        self.install.assert_not_called()
        self.assertFalse((self.directory / 'done').exists())
        self.assertFalse((self.directory / 'exit-status').exists())
        self.assertEqual(worker.run_training(self.root, 3), 0)

    def test_finished_or_failed_jobs_do_not_execute_again(self) -> None:
        """事例17: doneまたはexit-statusがあれば、既存の結果を変えずに0を返す。"""
        for name, content in (('done', 'done\n'), ('exit-status', '7\n')):
            with self.subTest(name=name):
                marker = self.directory / name
                marker.write_text(content)
                self.assertEqual(worker.run_training(self.root, 3), 0)
                self.assertEqual(marker.read_text(), content)
                self.assertFalse(self.calls)
                self.install.assert_not_called()
                marker.unlink()

    def test_installation_failure_is_published_without_running_steps(self) -> None:
        """NVRTCの準備失敗も終了コードを公開し、学習を開始しない。"""
        self.install.side_effect = ValueError('SHA-256 mismatch')
        self.assertEqual(worker.run_training(self.root, 3), 1)
        self.assertFalse(self.calls)
        self.assertEqual((self.directory / 'exit-status').read_text(), '1\n')
        self.assertFalse((self.directory / 'done').exists())

    def test_train_subcommand_uses_script_directory(self) -> None:
        """trainはスクリプトの配置先をrootとし、指定した世代の仕事を実行する。"""
        with patch.object(worker, '__file__', str(self.root / 'generation_worker.py')):
            self.assertEqual(worker.main(['train', '3']), 0)
        self.assertTrue((self.directory / 'done').is_file())


class NvrtcInstallTests(unittest.TestCase):
    """事例18: ダウンロードをメモリ上のホイールで代行し、照合と再利用を検証する。"""

    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, 'w') as archive:
            archive.writestr('nvidia/cu13/lib/libnvrtc.so.13', b'test library')
            archive.writestr('nvidia/cu13/lib/libnvrtc-builtins.so.13.0', b'test builtins')
        self.wheel = buffer.getvalue()
        self.checksum = hashlib.sha256(self.wheel).hexdigest()
        self.download = self.enterContext(patch.object(worker.urllib.request, 'urlopen',
                                                       side_effect=lambda *args, **kwargs: io.BytesIO(self.wheel)))

    def test_verified_wheel_installs_symlink_and_is_reused(self) -> None:
        """一致するホイールは展開してcompleteを作り、2回目のダウンロードを省く。"""
        directory = self.root / 'nvrtc'
        directory.mkdir()
        (directory / 'partial').touch()
        self.assertEqual(worker.NVRTC_SHA256, 'ad9b6d2ead2435f11cbb6868809d2adeeee302e9bb94bcf0539c7a40d80e8575')
        with patch.object(worker, 'NVRTC_SHA256', self.checksum):
            library = worker.install_nvrtc(self.root)
            self.assertEqual(library, directory / 'nvidia/cu13/lib')
            self.assertTrue((library / 'libnvrtc.so').is_symlink())
            self.assertEqual((library / 'libnvrtc.so').readlink(), Path('libnvrtc.so.13'))
            self.assertEqual((library / 'libnvrtc.so').read_bytes(), b'test library')
            self.assertEqual((library / 'libnvrtc-builtins.so.13.0').read_bytes(), b'test builtins')
            self.assertTrue((directory / 'complete').is_file())
            self.assertFalse((directory / 'partial').exists())
            self.assertFalse(list(directory.glob('*.whl')))
            self.assertEqual(worker.install_nvrtc(self.root), library)
        self.download.assert_called_once()
        self.assertEqual(self.download.call_args.args[0],
                         'https://files.pythonhosted.org/packages/c3/68/'
                         '483a78f5e8f31b08fb1bb671559968c0ca3a065ac7acabfc7cee55214fd6/'
                         'nvidia_cuda_nvrtc-13.0.88-py3-none-manylinux2010_x86_64.manylinux_2_12_x86_64.whl')

    def test_checksum_mismatch_does_not_publish_complete_or_extract(self) -> None:
        """SHA-256が違うとValueErrorとなり、未検証の内容は展開せず、次回は取り直す。"""
        with patch.object(worker, 'NVRTC_SHA256', '0' * 64):
            with self.assertRaisesRegex(ValueError, 'SHA-256'):
                worker.install_nvrtc(self.root)
        self.assertFalse((self.root / 'nvrtc/complete').exists())
        self.assertFalse((self.root / 'nvrtc/nvidia').exists())
        with patch.object(worker, 'NVRTC_SHA256', self.checksum):
            library = worker.install_nvrtc(self.root)
        self.assertEqual(self.download.call_count, 2)
        self.assertTrue((library / 'libnvrtc.so').is_symlink())
        self.assertTrue((self.root / 'nvrtc/complete').is_file())


if __name__ == '__main__':
    unittest.main()
