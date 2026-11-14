#!/usr/bin/env python3
"""vast.ai共通処理の外部コマンドと状態確認を検査する。

使い方: python3 scripts/vast/test_common.py
指示書の状態遷移、転送完了、時間制限、排他制御を偽コマンドで確かめる。
"""

from contextlib import redirect_stdout, redirect_stderr
import io
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import common

FAKE = r'''#!/usr/bin/env python3
import json, os, shutil, sys
from pathlib import Path
name = Path(sys.argv[0]).name
root = Path(os.environ['FAKE_ROOT'])
with (root / 'calls.jsonl').open('a') as log:
    log.write(json.dumps([name, *sys.argv[1:]]) + '\n')
path = root / 'responses.json'
responses = json.loads(path.read_text())
if not responses:
    sys.exit('予定外の呼び出し: ' + str(sys.argv))
response = responses.pop(0)
path.write_text(json.dumps(responses))
assert response['tool'] == name, (response, sys.argv)
assert sys.argv[1:1 + len(response['prefix'])] == response['prefix'], (response, sys.argv)
if 'transfer' in response:
    source = sys.argv[-2].split(':', 1)[1]
    destination = Path(sys.argv[-1].removeprefix('local:'))
    source_path = Path(response['transfer']) / source.removeprefix('/workspace/job/')
    target = destination if source.endswith('/') else destination / source_path.name
    shutil.copytree(source_path, target, dirs_exist_ok=True)
if 'upload' in response:
    destination = Path(response['upload'])
    destination.mkdir(exist_ok=True)
    for source in sys.argv[14:-1]:
        path = Path(source)
        if path.is_file():
            shutil.copy2(path, destination / path.name)
if 'signal_parent' in response:
    import signal
    def finish(signum, frame):
        (root / 'child-signal').write_text(str(signum))
        sys.exit(0)
    signal.signal(signal.SIGTERM, finish)
    signal.signal(signal.SIGINT, finish)
    os.kill(os.getppid(), response['signal_parent'])
    signal.pause()
print(response['stdout'], end='')
print(response['stderr'], end='', file=sys.stderr)
sys.exit(response['code'])
'''


class FakeCommands:
    """PATHで差し替えたコマンドへ応答を渡し、呼び出しを記録する。"""

    def __init__(self, root: Path) -> None:
        """空の応答列とコマンド群を作る。"""
        self.root = root
        self.bin = root / 'bin'
        self.bin.mkdir()
        for name in ('vastai', 'ssh', 'scp', 'rsync', 'git', 'cargo', 'rustc', 'minase'):
            path = self.bin / name
            path.write_text(FAKE)
            path.chmod(0o755)
        self.responses: list[dict] = []
        (root / 'responses.json').write_text('[]')
        self.environment = patch.dict(os.environ, {'PATH': str(self.bin) + os.pathsep + os.environ['PATH'],
                                                   'FAKE_ROOT': str(root)})

    def add(self, tool: str, prefix: list[str], stdout: str = '', code: int = 0,
            stderr: str = '', **extra: object) -> None:
        """次の外部コマンドの応答を追加する。"""
        self.responses.append(dict(tool=tool, prefix=prefix, stdout=stdout, code=code, stderr=stderr, **extra))
        (self.root / 'responses.json').write_text(json.dumps(self.responses))

    def calls(self) -> list[list[str]]:
        """記録された外部コマンドの引数を返す。"""
        path = self.root / 'calls.jsonl'
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    def remaining(self) -> list[dict]:
        """まだ実行されていない応答を返す。"""
        return json.loads((self.root / 'responses.json').read_text())


def state(status: str, intended: str = 'running') -> dict:
    """照会応答として使用する1台分の状態を作る。"""
    return {'id': 17, 'label': 'test', 'actual_status': status, 'intended_status': intended,
            'public_ipaddr': '192.0.2.1', 'ports': {'22/tcp': [{'HostPort': '2222'}]}}


class CommonTests(unittest.TestCase):
    """指示書の共通操作の契約を実際の子プロセスで検証する。"""

    def setUp(self) -> None:
        """偽コマンドと短い検査間隔を準備する。"""
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.fake = FakeCommands(self.root)
        self.fake.environment.start()
        self.addCleanup(self.fake.environment.stop)
        self.enterContext(patch.object(common, 'POLL_SECONDS', 0))
        self.enterContext(redirect_stdout(io.StringIO()))
        self.enterContext(redirect_stderr(io.StringIO()))

    def show(self, states: list[dict]) -> None:
        """状態照会への応答を予約する。"""
        self.fake.add('vastai', ['show', 'instances', '--raw'], json.dumps(states))

    def test_ids_instance_and_create(self) -> None:
        """ラベルとIDで選び、作成応答の正のIDを返す。"""
        self.show([state('running')])
        self.show([])
        self.fake.add('vastai', ['create', 'instance', '42'], '{"new_contract": 18}')
        self.assertEqual(common.ids('test'), [17])
        self.assertIsNone(common.instance(17, 2))
        self.assertEqual(common.create('42', 'label', self.root / 'start', 'rust:1.90.0-bookworm', 50), 18)
        self.assertFalse(self.fake.remaining())

    def test_bad_create_response_and_duplicate_instance(self) -> None:
        """不正な作成IDと重複するインスタンスIDを拒否する。"""
        for value in (True, 0, -1, '17'):
            self.fake.add('vastai', ['create'], json.dumps({'new_contract': value}))
        self.show([state('running'), state('running')])
        for _ in range(4):
            with self.assertRaises(common.VastError):
                common.create('42', 'label', self.root / 'start', 'rust:1.90.0-bookworm', 50)
        with self.assertRaises(common.VastError):
            common.instance(17, 2)

    def test_stop_requires_both_status_fields(self) -> None:
        """要求成功だけでは終わらず、実状態と予定状態が停止になるまで照会する。"""
        self.fake.add('vastai', ['stop', 'instance', '17'])
        self.show([state('running', 'stopped')])
        self.show([state('stopped', 'running')])
        self.show([state('exited', 'stopped')])
        common.stop(17)
        self.assertFalse(self.fake.remaining())

    def test_destroy_requires_disappearance(self) -> None:
        """削除要求の後も一覧に残っている間は成功しない。"""
        self.fake.add('vastai', ['destroy', 'instance', '17', '-y'])
        self.show([state('stopped', 'stopped')])
        self.show([])
        common.destroy(17)
        self.assertFalse(self.fake.remaining())

    def test_unconfirmed_operations_fail(self) -> None:
        """確認時間を使い切った停止と削除を失敗にする。"""
        for action in ('stop', 'destroy'):
            with self.subTest(action=action):
                self.fake.responses = []
                self.fake.add('vastai', [action, 'instance', '17'])
                self.show([state('running')])
                with patch.object(common.time, 'monotonic', side_effect=[0, 1, 2, 601]):
                    with self.assertRaises(common.VastError):
                        getattr(common, action)(17)

    def test_query_failure_does_not_confirm_stop(self) -> None:
        """状態の照会に失敗しても停止済みとみなさない。"""
        self.fake.add('vastai', ['stop'])
        self.fake.add('vastai', ['show'], code=1)
        with self.assertRaisesRegex(common.VastError, '状態照会'):
            common.stop(17)

    def test_failed_start_destroys(self) -> None:
        """起動不能の状態では削除し、その消失を確認してから失敗する。"""
        for status in ('stopped', 'exited', 'offline', 'unknown'):
            with self.subTest(status=status):
                self.fake.responses = []
                self.show([state(status)])
                self.fake.add('vastai', ['destroy', 'instance', '17', '-y'])
                self.show([])
                with self.assertRaises(common.VastError):
                    common.wait_running(17, 2)
                self.assertFalse(self.fake.remaining())

    def test_loading_then_running(self) -> None:
        """読込中から稼働中になれば削除せず成功する。"""
        self.show([state('loading')])
        self.show([state('running')])
        common.wait_running(17, 2)
        self.assertFalse(self.fake.remaining())

    def test_ssh_retry_and_transfer_arguments(self) -> None:
        """SSHの再試行と、SCPとrsyncの接続先を検証する。"""
        self.fake.add('ssh', ['-i'], code=1)
        self.fake.add('ssh', ['-i'])
        self.fake.add('scp', ['-r'])
        self.show([state('running')])
        self.fake.add('rsync', ['-a', '-e'])
        common.wait_ssh(17, '192.0.2.1', 2222)
        common.scp('192.0.2.1', 2222, '/workspace/job/', self.root / 'file with spaces')
        common.rsync(17, '/workspace/job/logs/', self.root / 'logs')
        calls = self.fake.calls()
        self.assertEqual(calls[2][-2:], [str(self.root / 'file with spaces'), 'root@[192.0.2.1]:/workspace/job/'])
        self.assertEqual(calls[-1][-2:], ['root@192.0.2.1:/workspace/job/logs/', str(self.root / 'logs') + '/'])
        self.assertFalse(self.fake.remaining())

    def test_copy_requires_completion_without_rsync_errors(self) -> None:
        """終了コード0でも完了行がなければ失敗し、エラーや警告を拒否する。"""
        outputs = ('total size is 1,234  speedup is 2.50\n', 'sending incremental file list\n',
                   'total size is 1,234  speedup is 2.50\nrsync error: failed\n',
                   'total size is 1,234  speedup is 2.50\nrsync warning: skipped\n')
        for output in outputs:
            self.fake.add('vastai', ['copy'], output)
        common.copy(17, '/workspace/job/logs/', self.root)
        for _ in range(3):
            with self.assertRaises(common.VastError):
                common.copy(17, '/workspace/job/logs/', self.root)
        self.assertFalse(self.fake.remaining())

    def test_rsync_to_uses_saved_address_without_account_query(self) -> None:
        """保存済みのIPv4とIPv6の接続先を使い、口座への照会を行わない。"""
        for host, expected in (('192.0.2.1', '192.0.2.1'), ('2001:db8::1', '[2001:db8::1]')):
            self.fake.add('rsync', ['-a', '-e'])
            common.rsync_to(host, 2222, '/workspace/job/teacher/', self.root / 'received')
            self.assertEqual(self.fake.calls()[-1][-2:],
                             [f'root@{expected}:/workspace/job/teacher/', str(self.root / 'received') + '/'])
        self.assertEqual([call[0] for call in self.fake.calls()], ['rsync', 'rsync'])
        with self.assertRaises(common.VastError):
            common.rsync_to('192.0.2.1', 2222, '/workspace/job/teacher/', Path('relative'))

    def test_rsync_to_preserves_failure(self) -> None:
        """転送元がまだない場合も終了コード23を呼び出し元へ返す。"""
        self.fake.add('rsync', ['-a', '-e'], code=23)
        with self.assertRaises(subprocess.CalledProcessError) as raised:
            common.rsync_to('192.0.2.1', 2222, '/workspace/job/teacher/', self.root / 'received')
        self.assertEqual(raised.exception.returncode, 23)

    def test_rsync_push_ignores_existing_files_and_uses_saved_ssh_address(self) -> None:
        """生成機への送信には既存ファイルの保護、末尾のスラッシュ、3,600秒の上限を指定する。"""
        for host, expected in (('192.0.2.1', '192.0.2.1'), ('2001:db8::1', '[2001:db8::1]')):
            with self.subTest(host=host):
                self.fake.add('rsync', ['-a', '--ignore-existing', '-e'])
                with patch.object(common, 'run_timed', wraps=common.run_timed) as timed:
                    common.rsync_push(host, 2222, self.root / 'books with spaces', '/workspace/job/teacher/gen-1/')
                call = self.fake.calls()[-1]
                self.assertEqual(call[:4], ['rsync', '-a', '--ignore-existing', '-e'])
                self.assertEqual(shlex.split(call[4]),
                                 ['ssh', '-i', str(Path.home() / '.ssh/id_vastai'), '-p', '2222',
                                  '-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=accept-new',
                                  '-o', 'ConnectTimeout=20', '-o', 'ServerAliveInterval=30'])
                self.assertEqual(call[-2:], [str(self.root / 'books with spaces') + '/',
                                            f'root@{expected}:/workspace/job/teacher/gen-1/'])
                self.assertEqual(timed.call_args.args[1], 3600)
        self.assertEqual([call[0] for call in self.fake.calls()], ['rsync', 'rsync'])

    def test_nonzero_copy_exit_is_preserved(self) -> None:
        """完了行があっても外部コマンドの失敗を伝える。"""
        self.fake.add('vastai', ['copy'], 'total size is 1 speedup is 1\n', code=23)
        with self.assertRaises(subprocess.CalledProcessError) as raised:
            common.copy(17, '/workspace/job/logs/', self.root)
        self.assertEqual(raised.exception.returncode, 23)

    def test_timeout_terminates_child(self) -> None:
        """時間超過では子を終了し、失敗として返す。"""
        executable = self.fake.bin / 'vastai'
        executable.write_text('#!/usr/bin/env python3\nimport signal\nsignal.pause()\n')
        with self.assertRaises(subprocess.TimeoutExpired):
            common.call(0.05, 'show', 'instances')

    def test_session_lock_is_exclusive(self) -> None:
        """同一セッションの競合を拒み、解放後は取得できる。"""
        path = self.root / '.lock'
        with common.session_lock(path):
            with self.assertRaisesRegex(common.VastError, '同じセッションの起動または取得が実行中です'):
                with common.session_lock(path):
                    self.fail('競合するロックを取得しました')
        with common.session_lock(path):
            pass


class TeacherCommonTests(unittest.TestCase):
    """フェーズ1の入札、CLI応答、転送上限、および受け入れ条件を検証する。"""

    def test_plain_authentication_errors_stop_without_polling(self) -> None:
        message = 'Failed with error 401: Two Factor Authentication\n'
        for action in ('stop', 'destroy'):
            for code in (0, 1):
                for stream in ('stdout', 'stderr'):
                    with self.subTest(action=action, code=code, stream=stream):
                        output, errors = (message, '') if stream == 'stdout' else ('', message)
                        result = subprocess.CompletedProcess(['vastai'], code, output, errors)
                        failure = subprocess.CalledProcessError(code, ['vastai'], output=output, stderr=errors)
                        with patch.object(common, 'run_timed', side_effect=failure if code else None,
                                          return_value=result) as run, patch.object(common.time, 'sleep') as sleep, \
                                redirect_stderr(io.StringIO()):
                            with self.assertRaises(common.AuthenticationExpired):
                                common.change_state(17, action)
                        run.assert_called_once()
                        sleep.assert_not_called()

    def test_plain_rate_limit_errors_retry_then_confirm_stop_or_deletion(self) -> None:
        message = 'Failed with error 429: API requests too frequent\n'
        for action in ('stop', 'destroy'):
            state = [] if action == 'destroy' else [
                {'id': 17, 'actual_status': 'stopped', 'intended_status': 'stopped'}]
            for code in (0, 1):
                for stream in ('stdout', 'stderr'):
                    with self.subTest(action=action, code=code, stream=stream):
                        output, errors = (message, '') if stream == 'stdout' else ('', message)
                        failure = (subprocess.CalledProcessError(code, ['vastai'], output=output, stderr=errors)
                                   if code else subprocess.CompletedProcess(['vastai'], code, output, errors))
                        responses = [failure, subprocess.CompletedProcess(['vastai'], 0, 'accepted\n', ''),
                                     subprocess.CompletedProcess(['vastai'], 0, json.dumps(state), '')]
                        with patch.object(common, 'run_timed', side_effect=responses) as run, \
                                patch.object(common.time, 'sleep') as sleep, redirect_stderr(io.StringIO()):
                            common.change_state(17, action)
                        self.assertEqual(run.call_count, 3)
                        self.assertEqual(run.call_args_list[0].args, run.call_args_list[1].args)
                        self.assertNotIn('--raw', run.call_args_list[0].args[0])
                        sleep.assert_called_once_with(60)

    def test_plain_rate_limit_errors_exhaust_without_polling(self) -> None:
        for action in ('stop', 'destroy'):
            with self.subTest(action=action), patch.object(common, 'run_timed', return_value=
                    subprocess.CompletedProcess(['vastai'], 0, '', 'Failed with error 429: too frequent\n')) as run, \
                    patch.object(common.time, 'sleep') as sleep, redirect_stderr(io.StringIO()):
                with self.assertRaisesRegex(common.RateLimitExceeded, '429'):
                    common.change_state(17, action)
                self.assertEqual(run.call_count, 5)
                self.assertEqual([call.args for call in sleep.call_args_list], [(60,)] * 4)

    def test_rate_limit_on_either_stream_retries_four_times_then_fails(self) -> None:
        response = '{"error": true, "status_code": 429, "msg": "API requests too frequent"}'
        for stream in ('stdout', 'stderr'):
            with self.subTest(stream=stream):
                result = subprocess.CompletedProcess(['vastai'], 0,
                                                     response if stream == 'stdout' else '',
                                                     response if stream == 'stderr' else '')
                with patch.object(common, 'run_timed', return_value=result) as run, \
                        patch.object(common.time, 'sleep') as sleep, redirect_stderr(io.StringIO()):
                    with self.assertRaises(common.RateLimitExceeded):
                        common.call(60, 'show', 'instances', '--raw')
                self.assertEqual(run.call_count, 5)
                self.assertEqual([call.args for call in sleep.call_args_list], [(60,)] * 4)
                self.assertTrue(all(call.args == run.call_args_list[0].args for call in run.call_args_list))

    def test_rate_limit_nonzero_exit_then_success_preserves_stderr(self) -> None:
        message = '{"status_code": 429}\n'
        failure = subprocess.CalledProcessError(1, ['vastai'], output='', stderr=message)
        result = subprocess.CompletedProcess(['vastai'], 0, '[]', '照会が完了しました\n')
        errors = io.StringIO()
        with patch.object(common, 'run_timed', side_effect=[failure, result]), \
                patch.object(common.time, 'sleep') as sleep, redirect_stderr(errors):
            self.assertEqual(common.call(60, 'show', 'instances', '--raw'), '[]')
        sleep.assert_called_once_with(60)
        self.assertEqual(errors.getvalue(), message + '照会が完了しました\n')

    def test_authentication_expiry_on_either_stream_never_retries(self) -> None:
        message = '{"status_code": 401, "msg": "Two Factor Authentication"}'
        for code in (0, 1):
            for stream in ('stdout', 'stderr'):
                with self.subTest(code=code, stream=stream):
                    output, errors = (message, '') if stream == 'stdout' else ('', message)
                    result = subprocess.CompletedProcess(['vastai'], code, output, errors)
                    failure = subprocess.CalledProcessError(code, ['vastai'], output=output, stderr=errors)
                    with patch.object(common, 'run_timed', side_effect=failure if code else None,
                                      return_value=result) as run, patch.object(common.time, 'sleep') as sleep, \
                            redirect_stderr(io.StringIO()):
                        with self.assertRaises(common.AuthenticationExpired):
                            common.call(60, 'show', 'instances', '--raw')
                    run.assert_called_once()
                    sleep.assert_not_called()

    def test_successful_exit_requires_json_for_raw_response(self) -> None:
        for output in ('', 'not json', '{'):
            with self.subTest(output=output), patch.object(common, 'run_timed', return_value=
                    subprocess.CompletedProcess(['vastai'], 0, output, '')):
                with self.assertRaises(common.InvalidResponse):
                    common.call(60, 'search', 'offers', 'rentable=true', '--raw')

    def test_bid_price_is_passed_only_for_interruptible_rental(self) -> None:
        for price in (None, 0.4):
            with self.subTest(price=price), patch.object(common, 'run_timed', return_value=
                    subprocess.CompletedProcess(['vastai'], 0, '{"new_contract": 17}', '')) as run:
                self.assertEqual(common.create('42', 'label', Path('/start'), 'rust:1.98.0-bookworm',
                                               30, bid_price=price), 17)
                command = run.call_args.args[0]
                if price is None:
                    self.assertNotIn('--bid_price', command)
                else:
                    self.assertEqual(command[command.index('--bid_price') + 1], '0.4')

    def test_transfer_limit_uses_decimal_megabytes_and_reserves_ten_minutes(self) -> None:
        self.assertEqual(common.transfer_seconds(53_000_000, 3600), 173)
        self.assertEqual(common.transfer_seconds(53_000_000, 700), 100)
        self.assertEqual(common.transfer_seconds(0, 1000), 120)
        self.assertEqual(common.transfer_seconds(0, 600.25), 0.25)
        for remaining in (600, 599, -1):
            with self.subTest(remaining=remaining), self.assertRaises(common.VastError):
                common.transfer_seconds(1, remaining)
        for size in (-1, True, 1.5):
            with self.subTest(size=size), self.assertRaises(common.VastError):
                common.transfer_seconds(size, 1000)

    def test_inspection_validates_every_header_and_summary_field(self) -> None:
        expected = {'generation_commit': 'a' * 40, 'network_checksum': '12' * 32,
                    'rule_set': 'L0,P0,R1,E0', 'seed': 9700100000, 'teacher_search_budget': 20000}
        settings = {'games': 10, 'nodes': 20000, 'max_ply': 4000, 'hash_mb': 16, 'random_moves': 0}
        header = expected | {'game_count': 9, 'eligible_positions': 120}
        summary = {'games_completed': 10, 'nodes': 20000, 'max_ply': 4000, 'hash_mb': 16,
                   'random_moves_max': 0, 'games_discarded_max_ply': 1, 'recorded_positions': 120}
        def lines(values: dict) -> str:
            return ''.join(f'{key}: {value}\n' for key, value in values.items())
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / 'book-1.log'
            cases = [(None, None)]
            cases += [('header', key) for key in header]
            cases += [('log', key) for key in summary]
            for location, key in cases:
                with self.subTest(location=location, key=key):
                    actual, reported = header.copy(), summary.copy()
                    if location == 'header':
                        actual[key] = -1 if key in ('game_count', 'eligible_positions') else '違う値'
                    if location == 'log':
                        reported[key] -= 1
                    log.write_text(lines(summary | {'nodes': 1}) + lines(reported))
                    def run(command: list, **kwargs: object) -> subprocess.CompletedProcess:
                        self.assertEqual(command, ['/minase', 'data', 'selfplay', 'inspect', '/book-1.mnsd'])
                        with patch.object(common.os, 'nice') as nice:
                            kwargs['preexec_fn']()
                        nice.assert_called_once_with(19)
                        return subprocess.CompletedProcess(command, 0, lines(actual))
                    if location is None:
                        self.assertEqual(common.inspect_teacher(Path('/minase'), Path('/book-1.mnsd'), log,
                                                                expected, settings, run), (9, 120))
                    else:
                        with self.assertRaises(common.VastError):
                            common.inspect_teacher(Path('/minase'), Path('/book-1.mnsd'), log, expected, settings, run)


if __name__ == '__main__':
    unittest.main()
