#!/usr/bin/env python3
"""SPSAの設定、計算、記録の取得と調整機の終了処理を検査する。

使い方: python3 scripts/vast-spsa/test_spsa.py
指示書とdocs/plans/search-forms.mdの式を期待値の根拠とする。
"""

from contextlib import redirect_stdout, redirect_stderr
from decimal import Decimal
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR.parent / 'vast'))
import common
import test_common
import fetch
import launch
import remote
from sync_run import IncompleteRun, check_run, sync_run

CONFIG = {'commit': 'a' * 40, 'runner_commit': 'b' * 40, 'seed': 0, 'iterations': 3750,
          'pairs_per_iteration': 8, 'rules': 'jCSA', 'precheck': False,
          'local_nps': 1264584, 'game_rss_bytes': 613000000}
OFFER = {'id': 42, 'cpu_arch': 'amd64', 'verification': 'verified', 'cpu_cores': 192,
         'duration': 172800, 'reliability2': 0.95, 'direct_port_count': 1, 'inet_up': 100,
         'disk_space': 50, 'gpu_frac': 1, 'cpu_cores_effective': 192}


def make_run(path: Path, applications: list[int]) -> None:
    """実行記録の契約に沿ってmanifestと適用番号つき反復を作る。"""
    path.mkdir(parents=True, exist_ok=True)
    (path / 'iterations').mkdir(exist_ok=True)
    manifest = {'engine': {'kind': 'commit', 'hash': CONFIG['commit']},
                'settings': {'seed': 0, 'iterations': 3750, 'pairs_per_iteration': 8},
                'rules_source': 'jCSA'}
    (path / 'manifest.json').write_text(json.dumps(manifest))
    (path / 'invocations.json').write_text('[{}]')
    for number in applications:
        (path / 'iterations' / f'{number}.json').write_text(json.dumps({'application': number}))


class SpsaTests(unittest.TestCase):
    """設定、計算、要約、manifestと写しの更新を検査する。"""

    def setUp(self) -> None:
        """設定とパラメーターファイルを一時ディレクトリへ作る。"""
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / 'params').write_text('parameter 1\n')
        self.write_config(CONFIG)

    def write_config(self, config: dict) -> None:
        """入力用の平らなTOMLを保存する。"""
        lines = []
        for key, value in config.items():
            if type(value) is str:
                value = json.dumps(value)
            elif type(value) is bool:
                value = str(value).lower()
            else:
                value = str(value)
            lines.append(f'{key} = {value}')
        (self.root / 'session.toml').write_text('\n'.join(lines) + '\n')

    def test_offer_accepts_all_boundaries(self) -> None:
        """選定条件の下限をすべて満たす出品を受け入れる。"""
        launch.check_offer([OFFER], '42')

    def test_offer_rejects_every_condition_and_missing_field(self) -> None:
        """10条件のどれかが違う出品と、各欄が欠ける出品を拒否する。"""
        invalid = {'cpu_arch': 'arm64', 'verification': 'unverified', 'cpu_cores': 191,
                   'duration': 172799, 'reliability2': 0.949, 'direct_port_count': 0,
                   'inet_up': 99, 'disk_space': 49, 'gpu_frac': 0.5, 'cpu_cores_effective': 191}
        for key, value in invalid.items():
            for absent in (False, True):
                with self.subTest(key=key, absent=absent):
                    offer = dict(OFFER)
                    if absent:
                        del offer[key]
                    else:
                        offer[key] = value
                    with self.assertRaises(ValueError):
                        launch.check_offer([offer], '42')
        for offers in ([], [OFFER, OFFER]):
            with self.assertRaises(ValueError):
                launch.check_offer(offers, '42')
        for value in (float('nan'), float('inf'), True, '192'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                launch.check_offer([{**OFFER, 'cpu_cores': value}], '42')

    def test_session_round_trip_and_integer_boundaries(self) -> None:
        """最大整数、seedの0、正の整数NPSを受け入れ、TOMLを書き戻せる。"""
        config = {**CONFIG, 'seed': 2**64 - 1, 'iterations': 2**64 - 1,
                  'precheck': True, 'precheck_seed': 0, 'rules': '規則"\\\n\t\x7f'}
        (self.root / 'safety.params').write_text('safe\n')
        self.write_config(config)
        self.assertEqual(remote.load_session(self.root), config)
        launch.write_session(self.root / 'session.toml', config)
        self.assertEqual(remote.load_session(self.root), config)

    def test_session_missing_unknown_and_wrong_types(self) -> None:
        """必須キーの欠落、未知のキーと型の違いを拒否する。"""
        for key in CONFIG:
            with self.subTest(missing=key):
                config = dict(CONFIG)
                del config[key]
                self.write_config(config)
                with self.assertRaises(ValueError):
                    remote.load_session(self.root)
        invalid = [('extra', 1), ('commit', 4), ('runner_commit', ''), ('rules', False),
                   ('precheck', 'false'), ('local_nps', True), ('local_nps', 0),
                   ('local_nps', float('nan')), ('local_nps', float('inf'))]
        for key, value in invalid:
            with self.subTest(key=key, value=value):
                self.write_config({**CONFIG, key: value})
                with self.assertRaises(ValueError):
                    remote.load_session(self.root)

    def test_integer_fields_reject_bool_float_and_out_of_range(self) -> None:
        """整数欄の真偽値、小数、負数、上限超過と正数欄の0を拒否する。"""
        (self.root / 'safety.params').write_text('safe\n')
        for key in ('seed', 'iterations', 'pairs_per_iteration', 'game_rss_bytes', 'precheck_seed'):
            for value in (True, 1.0, -1, 2**64):
                with self.subTest(key=key, value=value):
                    config = {**CONFIG, 'precheck': True, 'precheck_seed': 1, key: value}
                    self.write_config(config)
                    with self.assertRaises(ValueError):
                        remote.load_session(self.root)
        for key in ('iterations', 'pairs_per_iteration', 'game_rss_bytes'):
            self.write_config({**CONFIG, key: 0})
            with self.subTest(key=key), self.assertRaises(ValueError):
                remote.load_session(self.root)

    def test_precheck_and_parameter_files(self) -> None:
        """事前確認のシードとパラメーターファイルの必須条件を守る。"""
        for config in ({**CONFIG, 'precheck': True}, {**CONFIG, 'precheck_seed': 1},
                       {**CONFIG, 'precheck': True, 'precheck_seed': 1}):
            self.write_config(config)
            with self.assertRaises(ValueError):
                remote.load_session(self.root)
        self.write_config(CONFIG)
        (self.root / 'params').write_text('')
        with self.assertRaises(ValueError):
            remote.load_session(self.root)
        (self.root / 'params').unlink()
        with self.assertRaises(ValueError):
            remote.load_session(self.root)

    def test_concurrency_cpu_and_memory_limits(self) -> None:
        """CPUの1コア分とメモリの25%の余裕を確保する。"""
        self.assertEqual(remote.concurrency(96, 100_000_000_000, 613_000_000), 95)
        self.assertEqual(remote.concurrency(192, 126_000_000_000, 613_000_000), 164)
        for values in ((1, 10000, 1), (8, 1249, 1000), (0, 10000, 1)):
            with self.subTest(values=values), self.assertRaises(ValueError):
                remote.concurrency(*values)
        self.assertEqual(remote.concurrency(8, 1250, 1000), 1)

    def test_sizing_matches_hand_calculation(self) -> None:
        """1264584÷866000を1.46に丸め、6342240÷342000時間と照合する。"""
        factor, each, hours = remote.sizing(1264584, Decimal(866000), 3750, 8, 95)
        self.assertEqual(factor, Decimal('1.46'))
        self.assertEqual(each, 'time=14600+146')
        self.assertEqual(hours, Decimal('18.54456140'))
        self.assertEqual(remote.sizing(1005, Decimal(1000), 1, 1, 1)[:2],
                         (Decimal('1.01'), 'time=10100+101'))
        with self.assertRaises(ValueError):
            remote.sizing(1, Decimal(1000), 1, 1, 1)

    def test_summary_rejects_incomplete_and_each_precheck_failure(self) -> None:
        """完了数の不一致と、5種類の異常が各1件ある事前確認を拒否する。"""
        log = self.root / 'summary.log'
        names = ('illegal_moves', 'crashes', 'timeouts', 'time_forfeits', 'rejected_moves')
        for failed in (None, *names):
            text = 'summary: applied=13 valid_pairs=104 discarded_pairs=0\nengine_failures: '
            text += ' '.join(f'{name}={int(name == failed)}' for name in names) + '\n'
            log.write_text(text)
            remote.summary_ok(log, 13, False)
            if failed is None:
                remote.summary_ok(log, 13, True)
            else:
                with self.subTest(failure=failed), self.assertRaises(ValueError):
                    remote.summary_ok(log, 13, True)
            with self.assertRaises(ValueError):
                remote.summary_ok(log, 14, False)
        log.write_text('')
        with self.assertRaises(ValueError):
            remote.summary_ok(log, 13, False)

    def test_manifest_rejects_each_mismatch(self) -> None:
        """対象コミット、seed、反復数、ペア数と規則を個別に照合する。"""
        run = self.root / 'run'
        make_run(run, [1])
        path = run / 'manifest.json'
        remote.check_manifest(path, CONFIG)
        for key, value in (('commit', 'c' * 40), ('seed', 1), ('iterations', 3749),
                           ('pairs_per_iteration', 7), ('rules', 'different')):
            with self.subTest(key=key), self.assertRaises(ValueError):
                remote.check_manifest(path, {**CONFIG, key: value})

    def test_sync_preserves_previous_until_applications_are_contiguous(self) -> None:
        """適用番号に欠けがある取得では前回の写しを保ち、連続時だけ交換する。"""
        first, partial, complete = (self.root / name for name in ('first', 'partial', 'complete'))
        destination = self.root / 'run'
        make_run(first, [1])
        make_run(partial, [1, 3])
        make_run(complete, [1, 2, 3])
        self.assertEqual(sync_run(first, destination), 1)
        inode = (destination / 'manifest.json').stat().st_ino
        with self.assertRaises(IncompleteRun):
            sync_run(partial, destination)
        self.assertEqual((destination / 'manifest.json').stat().st_ino, inode)
        self.assertEqual(len(list((destination / 'iterations').iterdir())), 1)
        self.assertEqual(sync_run(complete, destination), 3)
        self.assertEqual((destination / 'manifest.json').stat().st_ino,
                         (complete / 'manifest.json').stat().st_ino)
        check_run(destination, 3)
        with self.assertRaises(IncompleteRun):
            check_run(destination, 4)

    def test_onstart_deadline_length_and_shell_syntax(self) -> None:
        """期限、Pythonの起動行、4048文字の上限とBash構文を検査する。"""
        script = launch.onstart(1800000000, 'trial-1')
        self.assertIn('bash -s -- 1800000000', script)
        self.assertNotIn('@DEADLINE@', script)
        self.assertIn('exec python3 /workspace/job/remote.py trial-1', script)
        self.assertLessEqual(len(script), 4048)
        subprocess.run(['bash', '-n'], input=script, text=True, check=True)
        with self.assertRaises(ValueError):
            launch.onstart(1800000000, 'a' * 4048)

    def test_bench_cli_outputs_mean_and_rejects_invalid_counts(self) -> None:
        """2本の中央値100と301の平均を出力し、不正な本数では実行しない。"""
        binary = self.root / 'engine'
        counter = binary.with_suffix('.count')
        counter.write_text('0')
        binary.write_text(
            '#!/usr/bin/env python3\n'
            'import fcntl\nfrom pathlib import Path\nimport sys\n'
            'assert sys.argv[1:] == ["dev", "bench", "--depth", "9", "--repetitions", "3"]\n'
            'with Path(__file__).with_suffix(".count").open("r+") as counter:\n'
            '    fcntl.flock(counter, fcntl.LOCK_EX)\n'
            '    index = int(counter.read())\n'
            '    counter.seek(0)\n'
            '    counter.write(str(index + 1))\n'
            '    counter.truncate()\n'
            'print(f"median: nps={(100, 301)[index]}")\n')
        binary.chmod(0o755)
        command = [sys.executable, str(SCRIPT_DIR / 'bench.py'), 'engine']
        result = subprocess.run([*command, '2', 'logs'], cwd=self.root,
                                capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, '200.5\n')
        self.assertEqual(result.stderr, '')
        for count in ('0', '-1', '1.5', 'abc'):
            with self.subTest(count=count):
                result = subprocess.run([*command, count, 'logs'], cwd=self.root,
                                        capture_output=True, text=True, timeout=5)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, '')
                self.assertIn('本数には正の整数が必要です', result.stderr)
        self.assertEqual(counter.read_text(), '2')


class WorkflowTests(unittest.TestCase):
    """偽の外部コマンドを通して取得、起動とシグナルの結果を確かめる。"""

    def setUp(self) -> None:
        """外部コマンドと取得元のファイルを一時領域へ準備する。"""
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.fake = test_common.FakeCommands(self.root)
        self.fake.environment.start()
        self.addCleanup(self.fake.environment.stop)
        self.enterContext(redirect_stdout(io.StringIO()))
        self.enterContext(redirect_stderr(io.StringIO()))
        self.enterContext(patch.object(common, 'POLL_SECONDS', 0))
        self.enterContext(patch.object(fetch, 'FETCH_SECONDS', 0))
        self.directory = self.root / 'local'
        self.directory.mkdir()
        self.source = self.root / 'job'
        make_run(self.source / 'trial', [1, 2])
        make_run(self.source / 'trial-safety', [1])
        (self.source / 'logs').mkdir()
        (self.source / 'logs/exit-status').write_text('0\n')

    def transfers(self, status: str, precheck: bool = False) -> None:
        """1回の取得に必要な状態応答と転送を予約する。"""
        self.fake.add('vastai', ['show'], json.dumps([test_common.state(status)]))
        for _ in range(3 if precheck else 2):
            if status == 'running':
                self.fake.add('vastai', ['show'], json.dumps([test_common.state(status)]))
                self.fake.add('rsync', ['-a'], transfer=str(self.source))
            else:
                self.fake.add('vastai', ['copy'], 'total size is 123 speedup is 1.0\n', transfer=str(self.source))

    def test_running_fetch_uses_ssh_rsync(self) -> None:
        """稼働中の取得はSSH経由で行い、ログと再開用の写しを保存する。"""
        self.transfers('running', True)
        fetch.fetch_once(17, 'trial', self.directory, True)
        check_run(self.directory / 'run', 2)
        self.assertEqual((self.directory / 'logs/exit-status').read_text(), '0\n')
        self.assertTrue((self.directory / 'safety/trial-safety/manifest.json').is_file())
        self.assertEqual(sum(call[0] == 'rsync' for call in self.fake.calls()), 3)
        self.assertFalse(any(call[1:2] == ['copy'] for call in self.fake.calls()))
        self.assertFalse(self.fake.remaining())

    def test_stopped_fetch_uses_vast_copy(self) -> None:
        """停止中の取得はvastai copyで行う。"""
        self.transfers('stopped')
        fetch.fetch_once(17, 'trial', self.directory, False)
        check_run(self.directory / 'run', 2)
        self.assertEqual(sum(call[1:2] == ['copy'] for call in self.fake.calls()), 2)
        self.assertFalse(any(call[0] == 'rsync' for call in self.fake.calls()))
        self.assertFalse(self.fake.remaining())

    def test_partial_fetch_keeps_previous_run_and_failed_transfer_has_no_fallback(self) -> None:
        """部分取得ではrunを保ち、通信失敗で転送方式を切り替えない。"""
        make_run(self.directory / 'run', [1])
        inode = (self.directory / 'run/manifest.json').stat().st_ino
        (self.source / 'trial/iterations/2.json').rename(self.source / 'trial/iterations/3.json')
        (self.source / 'trial/iterations/3.json').write_text('{"application": 3}')
        self.transfers('running')
        fetch.fetch_once(17, 'trial', self.directory, False)
        self.assertEqual((self.directory / 'run/manifest.json').stat().st_ino, inode)
        self.fake.responses = []
        self.fake.add('vastai', ['show'], json.dumps([{**test_common.state('running'), 'label': 'minase-spsa-trial'}]))
        self.fake.add('vastai', ['show'], json.dumps([{**test_common.state('running'), 'label': 'minase-spsa-trial'}]))
        self.fake.add('rsync', ['-a'], code=23)
        with self.assertRaises(subprocess.CalledProcessError):
            fetch.fetch_once(17, 'trial', self.directory, False)
        self.assertFalse((self.directory / 'logs/exit-status').exists())
        self.assertEqual((self.directory / 'run/manifest.json').stat().st_ino, inode)
        self.assertFalse(any(call[1:2] == ['copy'] for call in self.fake.calls()))

    def test_six_query_failures_do_not_stop_or_destroy(self) -> None:
        """6回の通信失敗で取得を打ち切り、調整機への操作を送らない。"""
        for _ in range(6):
            self.fake.add('vastai', ['show'], code=1)
        with self.assertRaisesRegex(ValueError, '6回連続'):
            fetch.fetch_loop('trial', self.root, self.directory, CONFIG, 1800000000)
        self.assertEqual(len(self.fake.calls()), 6)
        self.assertFalse(self.fake.remaining())

    def test_remote_forwards_signals_and_records_exit_status(self) -> None:
        """SIGTERMとSIGINTで子をSIGTERMにより終了し、143と130を保存する。"""
        for sig, expected in ((signal.SIGTERM, 143), (signal.SIGINT, 130)):
            with self.subTest(signal=sig):
                self.fake.responses = []
                self.fake.add('minase', ['spsa'], signal_parent=int(sig))
                driver = (
                    'import sys\nfrom pathlib import Path\n'
                    f'sys.path.insert(0, {str(SCRIPT_DIR)!r})\nimport remote\n'
                    'def execute(session, job, children):\n'
                    '    children.run(["minase", "spsa"])\n    return 0\n'
                    'remote.execute = execute\n'
                    f'sys.exit(remote.remote_main("trial", Path({str(self.source)!r})))\n')
                result = subprocess.run([sys.executable, '-c', driver], capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode, expected, result.stderr)
                self.assertEqual((self.source / 'logs/exit-status').read_text(), f'{expected}\n')
                self.assertEqual((self.root / 'child-signal').read_text(), str(int(signal.SIGTERM)))
                self.assertFalse((self.source / 'logs/.exit-status.tmp').exists())

    def test_remote_duplicate_does_not_overwrite_exit_status(self) -> None:
        """重複起動では進行中の終了記録を変更しない。"""
        with common.session_lock(self.source / '.remote.lock'):
            with self.assertRaisesRegex(ValueError, 'すでに実行中'):
                remote.remote_main('trial', self.source)
        self.assertEqual((self.source / 'logs/exit-status').read_text(), '0\n')

    def test_complete_fetch_validates_before_destroying(self) -> None:
        """全反復を再取得してapplyが通った場合だけ調整機を削除する。"""
        self.fake.add('vastai', ['show'], json.dumps([{**test_common.state('running'), 'label': 'minase-spsa-trial'}]))
        self.transfers('running')
        self.fake.add('vastai', ['show'], json.dumps([{**test_common.state('running'), 'label': 'minase-spsa-trial'}]))
        self.transfers('running')
        manifest_path = self.source / 'trial/manifest.json'
        manifest = json.loads(manifest_path.read_text())
        manifest['settings']['iterations'] = 2
        manifest_path.write_text(json.dumps(manifest))
        self.fake.add('git', ['show', CONFIG['commit'] + ':crates/minase/src/search/alphabeta/params.rs'], 'source')
        binary = self.root / 'target/release/minase'
        binary.parent.mkdir(parents=True)
        binary.symlink_to(self.fake.bin / 'minase')
        self.fake.add('minase', ['spsa', 'apply', '--run-dir', str(self.directory / 'run')])
        self.fake.add('vastai', ['destroy', 'instance', '17', '-y'])
        self.fake.add('vastai', ['show'], '[]')
        self.assertEqual(fetch.fetch_loop('trial', self.root, self.directory,
                                         {**CONFIG, 'iterations': 2}, 2**63), 0)
        self.assertFalse(self.fake.remaining())
        self.assertFalse(list(self.directory.glob('.params.*.rs')))

    def test_failed_production_stops_and_slow_machine_is_destroyed(self) -> None:
        """本番失敗では停止し、見込み時間の超過では削除して3を返す。"""
        for exit_status, action in (('5', 'stop'), ('3', 'destroy')):
            with self.subTest(exit_status=exit_status):
                self.fake.responses = []
                (self.source / 'logs/exit-status').write_text(exit_status + '\n')
                self.fake.add('vastai', ['show'], json.dumps([{**test_common.state('stopped'), 'label': 'minase-spsa-trial'}]))
                self.transfers('stopped')
                self.fake.add('vastai', ['show'], json.dumps([{**test_common.state('stopped'), 'label': 'minase-spsa-trial'}]))
                self.transfers('stopped')
                self.fake.add('vastai', [action, 'instance', '17'])
                states = [] if action == 'destroy' else [test_common.state('stopped', 'stopped')]
                self.fake.add('vastai', ['show'], json.dumps(states))
                if exit_status == '3':
                    self.assertEqual(fetch.fetch_loop('trial', self.root, self.directory, CONFIG, 2**63), 3)
                else:
                    with self.assertRaisesRegex(ValueError, '未完了'):
                        fetch.fetch_loop('trial', self.root, self.directory, CONFIG, 2**63)
                self.assertFalse(self.fake.remaining())

    def test_launch_transfers_resolved_toml_before_starting(self) -> None:
        """完全ハッシュの設定とPythonジョブを送り、転送後に開始を通知する。"""
        directory = self.root / 'data/vast-spsa/trial'
        directory.mkdir(parents=True)
        (directory / 'params').write_text('parameter 1\n')
        config = {**CONFIG, 'commit': 'topic', 'runner_commit': 'runner'}
        launch.write_session(directory / 'session.toml', config)
        uploaded = self.root / 'uploaded'
        key = self.root / '.ssh/id_vastai'
        key.parent.mkdir()
        key.touch()
        base = str(SCRIPT_DIR.parent.parent)
        self.fake.add('git', ['-C', base, 'rev-parse', '--show-toplevel'], str(self.root) + '\n')
        self.fake.add('git', ['-C', str(self.root), 'rev-parse', '--verify', '--end-of-options', 'topic^{commit}'], CONFIG['commit'] + '\n')
        self.fake.add('git', ['-C', str(self.root), 'rev-parse', '--verify', '--end-of-options', 'runner^{commit}'], CONFIG['runner_commit'] + '\n')
        self.fake.add('vastai', ['search', 'offers'], json.dumps([OFFER]))
        self.fake.add('vastai', ['show'], '[]')
        self.fake.add('rustc', ['--version'], 'rustc 1.90.0 (test)\n')
        self.fake.add('vastai', ['create', 'instance', '42'], '{"new_contract": 17}')
        self.fake.add('vastai', ['show'], json.dumps([{**test_common.state('running'), 'label': 'minase-spsa-trial'}]))
        self.fake.add('vastai', ['show'], json.dumps([{**test_common.state('running'), 'label': 'minase-spsa-trial'}]))
        self.fake.add('git', ['init', '--bare'])
        self.fake.add('git', [])
        self.fake.add('git', [])
        self.fake.add('ssh', ['-i'])
        self.fake.add('ssh', ['-i'])
        self.fake.add('scp', ['-r'], upload=str(uploaded))
        self.fake.add('ssh', ['-i'])
        with patch.object(Path, 'home', return_value=self.root):
            launch.launch('trial', '42', '1449')
        self.assertEqual(tomllib.loads((uploaded / 'session.toml').read_text()), CONFIG)
        self.assertTrue((uploaded / 'remote.py').is_file())
        self.assertEqual(self.fake.calls()[-1][-1], 'touch /workspace/job/started')
        self.assertEqual((directory / 'instance-id').read_text(), '17\n')
        self.assertFalse(list(directory.glob('.launch.*')))
        self.assertFalse(self.fake.remaining())

    def test_remote_execution_and_failure_codes(self) -> None:
        """事前確認成功時だけ本番へ進み、異常と時間超過の終了コードを保存する。"""
        for scenario, expected in (('production', 0), ('precheck_failure', 4),
                                   ('production_failure', 5), ('too_slow', 3)):
            with self.subTest(scenario=scenario):
                self.fake.responses = []
                job = self.root / scenario
                job.mkdir()
                (job / 'params').write_text('parameter 1\n')
                (job / 'safety.params').write_text('safe\n')
                config = {**CONFIG, 'precheck': True, 'precheck_seed': 1, 'iterations': 1}
                if scenario == 'too_slow':
                    config['iterations'] = 3750
                launch.write_session(job / 'session.toml', config)
                repository = job / 'minase'
                binary = repository / 'target/release/minase'
                tuning = repository / 'target/match-cache' / (CONFIG['commit'] + '-tuning/minase')
                for path in (binary, tuning):
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.symlink_to(self.fake.bin / 'minase')
                self.fake.add('git', ['checkout', '--detach', CONFIG['runner_commit']])
                self.fake.add('git', ['rev-parse', '--verify', '--end-of-options', CONFIG['commit'] + '^{commit}'], CONFIG['commit'] + '\n')
                self.fake.add('cargo', ['build', '--release', '--locked', '--bin', 'minase'])
                self.fake.add('minase', ['spsa', 'params'], 'parameter 1\n')
                self.fake.add('minase', ['dev', 'bench', '--depth', '9', '--repetitions', '3'], 'median: nps=866000\n')
                good = ('summary: applied=1 valid_pairs=8 discarded_pairs=0\n'
                        'engine_failures: illegal_moves=0 crashes=0 timeouts=0 time_forfeits=0 rejected_moves=0\n')
                if scenario != 'too_slow':
                    summary = good.replace('timeouts=0', 'timeouts=1') if scenario == 'precheck_failure' else good
                    self.fake.add('minase', ['spsa', '--run-dir', str(job / 'trial-safety')], summary)
                    if scenario != 'precheck_failure':
                        self.fake.add('minase', ['spsa', '--run-dir', str(job / 'trial')], good,
                                      code=1 if scenario == 'production_failure' else 0)
                with patch.object(remote, 'resources', return_value=(2, 10_000_000_000)):
                    self.assertEqual(remote.remote_main('trial', job), expected)
                self.assertEqual((job / 'logs/exit-status').read_text(), str(expected) + '\n')
                self.assertFalse(self.fake.remaining())

    def test_remote_resume_keeps_manifest_conditions_despite_slow_estimate(self) -> None:
        """再開では見積もりが24時間を超えても、新しい持ち時間を渡さない。"""
        job = self.source
        (job / 'params').write_text('parameter 1\n')
        launch.write_session(job / 'session.toml', CONFIG)
        repository = job / 'minase'
        binary = repository / 'target/release/minase'
        tuning = repository / 'target/match-cache' / (CONFIG['commit'] + '-tuning/minase')
        for path in (binary, tuning):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.symlink_to(self.fake.bin / 'minase')
        (job / '.build-ready').touch()
        self.fake.add('git', ['checkout', '--detach', CONFIG['runner_commit']])
        self.fake.add('git', ['rev-parse', '--verify'], CONFIG['commit'] + '\n')
        self.fake.add('minase', ['dev', 'bench'], 'median: nps=866000\n')
        self.fake.add('minase', ['spsa', '--resume', str(job / 'trial')],
                      'summary: applied=3750 valid_pairs=30000 discarded_pairs=0\n'
                      'engine_failures: illegal_moves=0 crashes=0 timeouts=0 time_forfeits=0 rejected_moves=0\n')
        with patch.object(remote, 'resources', return_value=(2, 10_000_000_000)):
            self.assertEqual(remote.remote_main('trial', job), 0)
        calls = self.fake.calls()
        self.assertEqual(calls[-1], ['minase', 'spsa', '--resume', str(job / 'trial')])
        self.assertFalse(self.fake.remaining())


if __name__ == '__main__':
    unittest.main()
