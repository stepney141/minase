"""実行器の契約を指示書brief-rl-runner.mdとdocs/plans/archived/alphazero-rl.mdから固定する。

対局数、較正、リプレイバッファの順序、開始の境界は独立した具体例で検証する。外部との境界には
一時ディレクトリを借りた機械に見立てたRemoteを使い、実ファイルで再開と転送を確かめる。
照合のシェルは外部コマンドだけを小さな実行ファイルへ置き換えて実際に走らせる。
台帳の欠落とチェックポイントの置き換え競合はbrief-rl-runner-fix.mdの契約に従う。
局面数によるリプレイバッファ、生成と学習のコミットの分離、一括転送を検証する。
機械の種類と複数GPUの契約はphase3a.mdとdocs/plans/archived/alphazero-multi-gpu.mdに従う。
"""

from contextlib import redirect_stdout
from dataclasses import replace
from datetime import UTC, datetime, timedelta
import hashlib
import io
import json
import os
from pathlib import Path
import runpy
import shlex
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from minase_zero.__main__ import _parse, main
from minase_zero import runner as runner_module
from minase_zero.runner import (
    Options, Remote, RemotePaths, Runner, append_cycle, can_start, chunk_files, game_counts, parse_balance,
    publish_network, receive, resignation, seed_registration, send, summaries, replay_buffer,
    load_vast_common, machine_info,
)


FIXTURE = "/content/src/crates/minase/tests/fixtures/nn-positions.jsonl"
VAST_FIXTURE = "/workspace/src/crates/minase/tests/fixtures/nn-positions.jsonl"
GENERATION_COMMIT = "a" * 40
TRAINER_COMMIT = "b" * 40
NOW = datetime(2026, 10, 29, 1, 0, tzinfo=UTC)


def write(path: Path, content: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(content)


def write_json(path: Path, value: dict) -> None:
    write(path, (json.dumps(value) + "\n").encode())


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def colab_remote(session):
    remote = Remote(session)
    remote.connect("colab", None)
    return remote


class FakeRemote:
    """転送と起動だけを置き換え、NAS側の確定処理は実物で検証する。"""

    def __init__(self, root: Path):
        self.root = root
        self.source_files = {FIXTURE, VAST_FIXTURE}
        self.existence_checks = []
        self.uploads = []
        self.upload_batches = []
        self.hash_batches = []
        self.downloads = []
        self.copies = []
        self.removals = []
        self.hashes = []
        self.starts = []
        self.states = {}
        self.balances = [100.0, 98.0, 98.0, 96.0]
        self.machine_calls = 0
        self.hardware = {"gpu": "NVIDIA L4", "gpu_memory_mib": 23034, "gpu_count": 1,
                         "cpu": "Test CPU", "nproc": 12, "cpus_available": 12}
        self.corrupt = False
        self.on_start = None
        self.on_download = None

    def connect(self, kind, vast_instance):
        self.kind = kind
        self.vast_instance = vast_instance
        self.paths = RemotePaths(kind)

    def exists(self, relative):
        self.existence_checks.append(relative)
        if Path(relative).is_absolute():
            return relative in self.source_files
        return (self.root / relative).is_file()

    def files(self, relative):
        path = self.root / relative
        return sorted(p.name for p in path.iterdir() if p.is_file()) if path.exists() else []

    def sha256(self, relative):
        self.hashes.append(relative)
        return digest(self.root / relative)

    def size(self, relative):
        return (self.root / relative).stat().st_size

    def missing(self, files):
        return [name for name in files if not (self.root / name).is_file()]

    def sha256_many(self, files, timeout):
        self.hash_batches.append((files, timeout))
        return {name: digest(self.root / name) for name in files}

    def copy(self, source, target):
        self.copies.append((source, target))
        shutil.copyfile(self.root / source, self.root / target)

    def remove(self, relative):
        self.removals.append(relative)
        (self.root / relative).unlink(missing_ok=True)

    def upload(self, root, files, timeout):
        self.upload_batches.append((files, timeout))
        for relative in files:
            self.uploads.append(relative)
            write(self.root / relative, b"corrupt" if self.corrupt else (root / relative).read_bytes())

    def download(self, relative, target, timeout):
        self.downloads.append(relative)
        if self.on_download is not None:
            self.on_download(relative, target)
        write(target, b"corrupt" if self.corrupt else (self.root / relative).read_bytes())

    def usage(self):
        if self.kind != "colab":
            raise AssertionError("vast must not query a balance")
        return self.balances.pop(0)

    def machine(self):
        self.machine_calls += 1
        info = {**self.hardware, "kind": self.kind}
        if self.kind == "vast":
            info["vast_instance"] = self.vast_instance
        return info

    def status(self, stem):
        if stem in self.states:
            state = self.states[stem]
            if isinstance(state, list):
                return state.pop(0)
            return state
        if self.exists(stem + ".exit"):
            return int((self.root / (stem + ".exit")).read_text())
        return "new"

    def start(self, stem, body, stdout, stderr):
        self.starts.append((stem, body, stdout, stderr))
        write(self.root / stdout, b"process log\n")
        write(self.root / stderr, b"process log\n")
        if self.on_start is None:
            raise AssertionError("test must define the remote job's outputs")
        code = self.on_start(stem, body)
        write(self.root / (stem + ".exit"), f"{code}\n".encode())


class RunnerCase(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="minase-rl-runner-")
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.root = self.directory / "nas"
        self.root.mkdir()
        self.remote_root = self.directory / "colab"
        self.remote_root.mkdir()
        self.remote = FakeRemote(self.remote_root)
        self.reference = self.directory / "trial.json"
        write_json(self.reference, {"games": 20, "training_positions": 2000})
        self.options = Options(
            root=self.root, machine="colab", vast_instance=None, session="test-session",
            generation_commit=GENERATION_COMMIT,
            cycles_dir=Path("cycles"), base_cycles_dir=Path("cycles"), branch_from=0,
            trainer_commit=TRAINER_COMMIT, first_cycle=1, last_cycle=1,
            deadline=NOW + timedelta(hours=8), cycle_minutes=40, cycle_cu=2,
            start_position="random-opening", generation_seed_offset=0,
            target_positions=1000, chunks=2, simulations=32, threads_per_gpu=6, games_per_thread=4,
            replay_buffer_first_cycle=1,
            replay_buffer_positions=1000, batch_size=256, lr=0.01, momentum=0.9, weight_decay=0.0001,
            clip_grad_norm=10, value_lambda=0.333, precision="bf16", checkpoint_steps=200,
            train_seed_base=123, minase=self.directory / "minase", poll_seconds=60,
            cycle1_reference_summary=self.reference,
        )
        executable = """#!/usr/bin/env python3
import json, pathlib, sys
args = sys.argv[1:]
if args == ['data', 'teacher-stats', '--help']:
    sys.exit(0)
elif (args[:2] == ['data', 'teacher-stats'] and len(args) > 2
      and all(pathlib.Path(name).is_file() for name in args[2:])):
    print(json.dumps({'files': args[2:], 'game_length_plies': {'p50': 100, 'p90': 120, 'p99': 140}}))
else:
    sys.exit(2)
"""
        write(self.options.minase, executable.encode())
        self.options.minase.chmod(0o755)
        self.runner = Runner(self.options, self.remote)
        write(self.root / "cycles/cycle-0000/model.onnx", b"initial network")
        write(self.root / "cycles/cycle-0000/checkpoint.pt", b"initial checkpoint")
        clock = patch("minase_zero.runner.datetime")
        self.clock = clock.start()
        self.addCleanup(clock.stop)
        self.clock.now.return_value = NOW
        self.clock.fromisoformat.side_effect = datetime.fromisoformat
        self.clock.side_effect = datetime

    def summary(self, cycle, kind, data):
        path = self.root / f"cycles/cycle-{cycle:04d}/{kind}/summary.json"
        write_json(path, data)
        return path

    def execute(self, options):
        Runner(options, self.remote).run()


class CalculationTests(RunnerCase):
    def test_cycle_one_uses_trial_ratio_and_rounds_both_counts_up(self):
        write_json(self.reference, {"games": 3, "training_positions": 100})
        self.assertEqual(game_counts([self.reference], 1001, 4), (31, 8))

    def test_previous_cycle_ratio_includes_regenerated_games(self):
        self.summary(2, "selfplay", {"games": 100, "training_positions": 12000})
        self.assertEqual(game_counts(summaries(self.options, 2), 1000000, 20), (8334, 417))
        self.summary(2, "selfplay-regen", {"games": 20, "training_positions": 3000})
        self.assertEqual(game_counts(summaries(self.options, 2), 1000000, 20), (8000, 400))

    def test_zero_or_negative_reference_is_rejected(self):
        for games, positions in ((0, 100), (10, 0), (-1, 100)):
            with self.subTest(games=games, positions=positions):
                write_json(self.reference, {"games": games, "training_positions": positions})
                with self.assertRaises(ValueError):
                    game_counts([self.reference], 1000, 20)

    def test_resignation_starts_disabled_and_uses_previous_calibration(self):
        self.assertEqual(resignation(self.options, 1), (100, {
            "n": 0, "false_resignations": 0, "upper_bound": None, "enable_resignation": False,
        }))
        for n, errors, expected in ((100, 1, 20), (100, 2, 100), (99, 0, 100), (0, 0, 100)):
            with self.subTest(n=n, errors=errors):
                self.summary(1, "selfplay", {"resign_triggered_playthrough_games": n, "false_resignations": errors})
                percent, calibration = resignation(self.options, 2)
                self.assertEqual(percent, expected)
                self.assertEqual(calibration["n"], n)
                self.assertEqual(calibration["false_resignations"], errors)

    def test_resignation_adds_regenerated_samples(self):
        self.summary(2, "selfplay", {"resign_triggered_playthrough_games": 50, "false_resignations": 1})
        self.summary(2, "selfplay-regen", {"resign_triggered_playthrough_games": 50, "false_resignations": 0})
        percent, calibration = resignation(self.options, 3)
        self.assertEqual(percent, 20)
        self.assertEqual(calibration["n"], 100)
        self.assertAlmostEqual(calibration["upper_bound"], 0.0466, places=4)

    def test_replay_buffer_uses_position_boundary_and_keeps_summary_order(self):
        # 3から数えると250、450、550局面。2の生成し直し50局面も含む。
        for cycle, positions in ((1, 100), (2, 150), (3, 250)):
            self.summary(cycle, "selfplay", {
                "training_positions": positions,
                "training_files": ["chunk-00001.mnsd", "chunk-00000.resigned.mnsd"],
            })
        self.summary(2, "selfplay-regen", {"training_positions": 50, "training_files": ["chunk-00000.mnsd"]})
        first = ["cycles/cycle-0001/selfplay/chunk-00001.mnsd",
                 "cycles/cycle-0001/selfplay/chunk-00000.resigned.mnsd"]
        second = ["cycles/cycle-0002/selfplay/chunk-00001.mnsd",
                  "cycles/cycle-0002/selfplay/chunk-00000.resigned.mnsd",
                  "cycles/cycle-0002/selfplay-regen/chunk-00000.mnsd"]
        third = ["cycles/cycle-0003/selfplay/chunk-00001.mnsd",
                 "cycles/cycle-0003/selfplay/chunk-00000.resigned.mnsd"]
        for positions, cycles, files in (
            (1, [3], third), (250, [3], third), (251, [2, 3], second + third),
            (450, [2, 3], second + third), (451, [1, 2, 3], first + second + third),
            (550, [1, 2, 3], first + second + third), (1000, [1, 2, 3], first + second + third),
        ):
            with self.subTest(positions=positions):
                self.assertEqual(replay_buffer(self.options, 3, positions), (cycles, files, third))
        self.assertEqual(replay_buffer(self.options, 1, 1000), ([1], first, first))
        self.assertEqual(replay_buffer(self.options, 2, 100), ([2], second, second))
        self.assertEqual(replay_buffer(self.options, 2, 200), ([2], second, second))

    def test_replay_buffer_rejects_paths_outside_generation_output(self):
        self.summary(1, "selfplay", {"training_positions": 100, "training_files": ["../../outside.mnsd"]})
        with self.assertRaises(ValueError):
            replay_buffer(self.options, 1, 1)

    def test_replay_buffer_rejects_invalid_position_counts(self):
        for positions in (-1, 1.5, True):
            with self.subTest(positions=positions):
                self.summary(1, "selfplay", {"training_positions": positions, "training_files": ["chunk-00000.mnsd"]})
                with self.assertRaises(ValueError):
                    replay_buffer(self.options, 1, 100)

    def test_replay_buffer_does_not_read_cycles_before_the_position_boundary(self):
        self.summary(3, "selfplay", {"training_positions": 100, "training_files": ["chunk-00000.mnsd"]})
        files = ["cycles/cycle-0003/selfplay/chunk-00000.mnsd"]
        self.assertEqual(replay_buffer(self.options, 3, 100), ([3], files, files))

    def test_replay_buffer_does_not_read_cycles_before_replay_buffer_first_cycle(self):
        options = replace(self.options, first_cycle=3, last_cycle=3, replay_buffer_first_cycle=3,
                          cycle1_reference_summary=None)
        self.summary(3, "selfplay", {"training_positions": 100, "training_files": ["chunk-00000.mnsd"]})
        files = ["cycles/cycle-0003/selfplay/chunk-00000.mnsd"]
        self.assertEqual(replay_buffer(options, 3, 1000), ([3], files, files))

    def test_replay_buffer_starts_at_replay_buffer_first_cycle_when_positions_are_insufficient(self):
        options = replace(self.options, first_cycle=4, last_cycle=4, replay_buffer_first_cycle=3,
                          cycle1_reference_summary=None)
        for cycle, positions in ((1, 1000), (2, 1000), (3, 100), (4, 200)):
            self.summary(cycle, "selfplay", {
                "training_positions": positions, "training_files": ["chunk-00000.mnsd"],
            })
        third = ["cycles/cycle-0003/selfplay/chunk-00000.mnsd"]
        fourth = ["cycles/cycle-0004/selfplay/chunk-00000.mnsd"]
        self.assertEqual(replay_buffer(options, 4, 1000), ([3, 4], third + fourth, fourth))

    def test_branch_replay_buffer_and_calibration_use_the_explicit_boundary(self):
        options = replace(self.options, cycles_dir=Path("cycles-branch"), base_cycles_dir=Path("baseline"),
                          branch_from=2, first_cycle=3, last_cycle=4, cycle1_reference_summary=None)
        for directory, cycle, positions, samples in (("baseline", 1, 100, 0), ("baseline", 2, 150, 50),
                                                     ("cycles-branch", 3, 250, 0)):
            write_json(self.root / f"{directory}/cycle-{cycle:04d}/selfplay/summary.json", {
                "games": 10, "training_positions": positions, "training_files": ["chunk-00000.mnsd"],
                "resign_triggered_playthrough_games": samples, "false_resignations": 0,
            })
        write_json(self.root / "baseline/cycle-0002/selfplay-regen/summary.json", {
            "games": 5, "training_positions": 50, "training_files": ["chunk-00001.mnsd"],
            "resign_triggered_playthrough_games": 50, "false_resignations": 0,
        })
        # 反対側にも同じ番号を置き、存在する側を選ぶ処理では通らないようにする。
        for directory, cycle in (("cycles-branch", 2), ("baseline", 3), ("cycles", 2)):
            write_json(self.root / f"{directory}/cycle-{cycle:04d}/selfplay/summary.json", {"wrong": True})
        second = ["baseline/cycle-0002/selfplay/chunk-00000.mnsd",
                  "baseline/cycle-0002/selfplay-regen/chunk-00001.mnsd"]
        third = ["cycles-branch/cycle-0003/selfplay/chunk-00000.mnsd"]
        self.assertEqual(replay_buffer(options, 3, 450), ([2, 3], second + third, third))
        self.assertEqual(replay_buffer(options, 3, 451), (
            [1, 2, 3], ["baseline/cycle-0001/selfplay/chunk-00000.mnsd", *second, *third], third,
        ))
        self.assertEqual(replay_buffer(replace(options, replay_buffer_first_cycle=2), 3, 1000),
                         ([2, 3], second + third, third))
        self.assertEqual(replay_buffer(replace(options, replay_buffer_first_cycle=3), 3, 1000),
                         ([3], third, third))
        self.assertEqual(game_counts(summaries(options, 2), 1000, 2), (75, 38))
        self.assertEqual(resignation(options, 3)[0], 20)
        self.assertEqual(resignation(options, 4)[0], 100)
        (self.root / "baseline/cycle-0002/selfplay/summary.json").unlink()
        with self.assertRaises(FileNotFoundError):
            replay_buffer(options, 3, 450)
        (self.root / "cycles-branch/cycle-0003/selfplay/summary.json").unlink()
        with self.assertRaises(FileNotFoundError):
            replay_buffer(options, 3, 1)

    def test_start_accepts_equal_time_and_balance(self):
        deadline = NOW + timedelta(minutes=40)
        self.assertTrue(can_start(NOW, deadline, 40, 2, 2))
        self.assertFalse(can_start(NOW + timedelta(microseconds=1), deadline, 40, 2, 2))
        self.assertFalse(can_start(NOW, deadline, 40, 1.99, 2))

    def test_usage_requires_one_explicit_numeric_balance(self):
        self.assertEqual(parse_balance("Usage\nCurrent balance: 123.45 compute units\nConsumption: 1.54 CU/h\n"), 123.45)
        self.assertEqual(parse_balance("Current balance: 0 compute units"), 0)
        for invalid in ("", "Current balance: NaN compute units", "Current balance: -1 compute units",
                        "Balance: 12 compute units", "Current balance: 12 credits",
                        "Current balance: 1 compute units\nCurrent balance: 2 compute units"):
            with self.subTest(output=invalid), self.assertRaises(ValueError):
                parse_balance(invalid)


class StorageTests(RunnerCase):
    def test_missing_seed_ledger_is_an_error_and_creates_nothing(self):
        for parent in (self.root, self.root / "absent"):
            with self.subTest(parent=parent):
                ledger = parent / "seed-ledger.jsonl"
                with self.assertRaises(FileNotFoundError):
                    seed_registration(ledger, "cycle-0001-gen", 1000000000, 10)
                self.assertFalse(ledger.exists())
        self.assertFalse((self.root / "absent").exists())

    def test_seed_reuse_preserves_ledger_and_rejects_changed_seed_or_games(self):
        ledger = self.root / "seed-ledger.jsonl"
        ledger.touch()
        first = seed_registration(ledger, "cycle-0001-gen", 1000000000, 10)
        original = ledger.read_bytes()
        self.assertEqual(seed_registration(ledger, "cycle-0001-gen", 1000000000, 10), first)
        self.assertEqual(ledger.read_bytes(), original)
        for seed, games in ((1000000001, 10), (1000000000, 11)):
            with self.subTest(seed=seed, games=games), self.assertRaises(ValueError):
                seed_registration(ledger, "cycle-0001-gen", seed, games)
        self.assertEqual(ledger.read_bytes(), original)

    def test_new_seed_uses_ledger_overlap_and_overflow_checks(self):
        ledger = self.root / "seed-ledger.jsonl"
        ledger.touch()
        seed_registration(ledger, "trial", 1000000000, 10)
        for seed in (1000000009, 2**64 - 1):
            with self.subTest(seed=seed), self.assertRaises(ValueError):
                seed_registration(ledger, "cycle-0001-gen", seed, 2)

    def test_duplicate_cycle_does_not_modify_existing_lines(self):
        path = self.root / "cycles.jsonl"
        append_cycle(path, {"cycle": 1, "result": "accepted"})
        original = path.read_bytes()
        with self.assertRaises(ValueError):
            append_cycle(path, {"cycle": 1, "result": "changed"})
        self.assertEqual(path.read_bytes(), original)
        append_cycle(path, {"cycle": 2})
        self.assertEqual([json.loads(line)["cycle"] for line in path.read_text().splitlines()], [1, 2])

    def test_receive_mismatch_preserves_old_checkpoint(self):
        relative = "cycles/cycle-0001/train/checkpoint.pt"
        write(self.root / relative, b"old checkpoint")
        write(self.remote_root / relative, b"new checkpoint")
        self.remote.corrupt = True
        with self.assertRaises(ValueError):
            receive(self.remote, self.root, relative, True)
        self.assertEqual((self.root / relative).read_bytes(), b"old checkpoint")
        self.assertFalse((self.root / relative).with_name(".checkpoint.pt.tmp").exists())
        self.assertEqual(len(self.remote.copies), 1)
        snapshot = self.remote.copies[0][1]
        self.assertEqual(self.remote.removals, [snapshot])
        self.assertFalse((self.remote_root / snapshot).exists())
        self.assertEqual((self.remote_root / relative).read_bytes(), b"new checkpoint")

    def test_receive_timeout_assumes_one_megabyte_per_second(self):
        for size, timeout in ((300_000_000, 420), (0, 120), (1_250_000, 121.25)):
            with self.subTest(size=size):
                relative = f"train/checkpoint-{size}.pt"
                write(self.remote_root / relative, b"checkpoint" if size else b"")
                with patch.object(self.remote, "size", return_value=size) as remote_size, \
                        patch.object(self.remote, "download", wraps=self.remote.download) as download:
                    self.assertTrue(receive(self.remote, self.root, relative, False))
                self.assertEqual(download.call_args.args[2:], (timeout,))
                remote_size.assert_called_once_with(relative)

    def test_receive_replacement_timeout_uses_snapshot_size(self):
        relative = "train/checkpoint.pt"
        write(self.remote_root / relative, b"a" * 1_000_000)
        copy = self.remote.copy

        def copy_then_replace(source, target):
            copy(source, target)
            write(self.remote_root / relative, b"b" * 2_000_000)

        with patch.object(self.remote, "copy", side_effect=copy_then_replace), \
                patch.object(self.remote, "size", wraps=self.remote.size) as remote_size, \
                patch.object(self.remote, "download", wraps=self.remote.download) as download:
            self.assertTrue(receive(self.remote, self.root, relative, True))
        snapshot = self.remote.copies[0][1]
        remote_size.assert_called_once_with(snapshot)
        self.assertEqual(download.call_args.args[2:], (121,))
        self.assertEqual((self.root / relative).read_bytes(), b"a" * 1_000_000)
        self.assertFalse((self.remote_root / snapshot).exists())

    def test_invalid_remote_size_does_not_commit_a_received_file(self):
        relative = "train/checkpoint.pt"
        write(self.remote_root / relative, b"checkpoint")
        remote = colab_remote("local")
        for replace in (False, True):
            with self.subTest(replace=replace):
                with patch.object(remote, "shell", return_value="not-an-integer\n"), \
                        patch.object(self.remote, "size", side_effect=remote.size), \
                        self.assertRaises(ValueError):
                    receive(self.remote, self.root, relative, replace)
                self.assertFalse((self.root / relative).exists())
                self.assertFalse((self.root / relative).with_name(".checkpoint.pt.tmp").exists())
                self.assertEqual(self.remote.downloads, [])
                self.assertTrue(all(not (self.remote_root / name).exists() for _, name in self.remote.copies))

    def test_checkpoint_is_replaced_only_after_download_and_checksum(self):
        relative = "cycles/cycle-0001/train/checkpoint.pt"
        write(self.root / relative, b"old checkpoint")
        write(self.remote_root / relative, b"new checkpoint")

        def inspect_download(name, target):
            self.assertEqual((self.root / relative).read_bytes(), b"old checkpoint")
            self.assertEqual(target.name, ".checkpoint.pt.tmp")

        self.remote.on_download = inspect_download
        self.assertTrue(receive(self.remote, self.root, relative, True))
        self.assertEqual((self.root / relative).read_bytes(), b"new checkpoint")
        self.assertFalse(receive(self.remote, self.root, relative, True))
        self.assertEqual(len(self.remote.downloads), 1)
        self.assertEqual(len(self.remote.copies), 2)
        self.assertEqual(self.remote.removals, [snapshot for _, snapshot in self.remote.copies])
        self.assertTrue(all(not (self.remote_root / snapshot).exists() for _, snapshot in self.remote.copies))

    def test_checkpoint_uses_one_copied_version_when_original_is_replaced(self):
        relative = "cycles/cycle-0001/train/checkpoint.pt"
        write(self.root / relative, b"old NAS checkpoint")
        write(self.remote_root / relative, b"checkpoint before replacement")
        remote_hash = self.remote.sha256

        def hash_then_replace(name):
            checksum = remote_hash(name)
            replacement = (self.remote_root / relative).with_name("checkpoint.pt.tmp")
            write(replacement, b"checkpoint after replacement")
            replacement.replace(self.remote_root / relative)
            return checksum

        with patch.object(self.remote, "sha256", side_effect=hash_then_replace):
            self.assertTrue(receive(self.remote, self.root, relative, True))
        self.assertEqual((self.root / relative).read_bytes(), b"checkpoint before replacement")
        self.assertEqual((self.remote_root / relative).read_bytes(), b"checkpoint after replacement")
        self.assertEqual(len(self.remote.copies), 1)
        source, snapshot = self.remote.copies[0]
        self.assertEqual(source, relative)
        self.assertNotEqual(snapshot, relative)
        self.assertEqual(Path(snapshot).parent, Path(relative).parent)
        self.assertEqual(self.remote.hashes, [snapshot])
        self.assertEqual(self.remote.downloads, [snapshot])
        self.assertEqual(self.remote.removals, [snapshot])
        self.assertFalse((self.remote_root / snapshot).exists())

    def test_download_failure_removes_snapshot_and_preserves_nas_checkpoint(self):
        relative = "cycles/cycle-0001/train/checkpoint.pt"
        write(self.root / relative, b"old checkpoint")
        write(self.remote_root / relative, b"new checkpoint")

        def fail_download(name, target):
            write(target, b"partial transfer")
            raise OSError("download failed")

        self.remote.on_download = fail_download
        with self.assertRaises(OSError):
            receive(self.remote, self.root, relative, True)
        self.assertEqual((self.root / relative).read_bytes(), b"old checkpoint")
        self.assertFalse((self.root / relative).with_name(".checkpoint.pt.tmp").exists())
        self.assertEqual(len(self.remote.copies), 1)
        snapshot = self.remote.copies[0][1]
        self.assertEqual(self.remote.removals, [snapshot])
        self.assertFalse((self.remote_root / snapshot).exists())

    def test_immutable_file_is_received_directly_without_a_snapshot(self):
        relative = "cycles/cycle-0001/train/model.onnx"
        write(self.remote_root / relative, b"immutable network")
        self.assertTrue(receive(self.remote, self.root, relative, False))
        self.assertEqual((self.root / relative).read_bytes(), b"immutable network")
        self.assertEqual(self.remote.hashes, [relative])
        self.assertEqual(self.remote.downloads, [relative])
        self.assertEqual(self.remote.copies, [])
        self.assertEqual(self.remote.removals, [])

    def test_immutable_file_is_never_overwritten(self):
        write(self.root / "file.json", b"original")
        write(self.remote_root / "file.json", b"changed")
        with self.assertRaises(ValueError):
            receive(self.remote, self.root, "file.json", False)
        self.assertEqual((self.root / "file.json").read_bytes(), b"original")
        self.assertEqual(self.remote.downloads, [])

    def test_upload_checks_remote_hash_and_existing_remote_content(self):
        write(self.root / "model.onnx", b"network")
        self.remote.corrupt = True
        with self.assertRaises(ValueError):
            send(self.remote, self.root, ["model.onnx"])
        self.remote.corrupt = False
        with self.assertRaises(ValueError):
            send(self.remote, self.root, ["model.onnx"])
        self.assertEqual(self.remote.uploads, ["model.onnx"])

    def test_batch_upload_only_sends_missing_files_and_checks_every_file_once(self):
        files = ["old/chunk-00000.mnsd", "new/chunk-00000.mnsd", "new/chunk-00001.mnsd"]
        for name, size in zip(files, (1000000, 2000000, 500000)):
            write(self.root / name, b"x" * size)
        write(self.remote_root / files[0], (self.root / files[0]).read_bytes())
        with patch.object(self.remote, "exists", side_effect=AssertionError("per-file exists")), \
                patch.object(self.remote, "sha256", side_effect=AssertionError("per-file hash")):
            send(self.remote, self.root, files)
        self.assertEqual(self.remote.upload_batches, [(files[1:], 122.5)])
        self.assertEqual(self.remote.hash_batches, [(files, 122.5)])
        self.assertTrue(all((self.remote_root / name).read_bytes() == (self.root / name).read_bytes() for name in files))
        send(self.remote, self.root, files)
        self.assertEqual(len(self.remote.upload_batches), 1)
        self.assertEqual(self.remote.hash_batches[-1], (files, 120))

    def test_batch_rejects_a_mismatch_in_any_file(self):
        files = ["old/chunk-00000.mnsd", "new/chunk-00000.mnsd", "new/chunk-00001.mnsd"]
        for name in files:
            write(self.root / name, b"expected")
            write(self.remote_root / name, b"expected")
        for name in files:
            with self.subTest(name=name):
                write(self.remote_root / name, b"corrupt")
                with self.assertRaises(ValueError):
                    send(self.remote, self.root, files)
                write(self.remote_root / name, b"expected")
        self.assertEqual(self.remote.uploads, [])
        self.assertEqual(len(self.remote.hash_batches), 3)

    def test_generation_restore_sends_only_config_and_committed_triples(self):
        out = "cycles/cycle-0001/selfplay"
        names = ["config.json", "chunk-00000.mnsd", "chunk-00000.resigned.mnsd", "chunk-00000.json"]
        for name in [*names, "chunk-00001.mnsd", "chunk-00001.json.tmp", ".zero_selfplay.lock"]:
            write(self.root / out / name, name.encode())
        self.runner.restore_generation(out)
        self.assertEqual(self.remote.uploads, [f"{out}/{name}" for name in names])
        self.assertEqual(len(self.remote.upload_batches), 1)
        self.assertEqual(len(self.remote.hash_batches), 1)
        self.remote.uploads.clear()
        self.runner.restore_generation(out)
        self.assertEqual(self.remote.uploads, [])

    def test_chunk_without_both_data_files_is_rejected(self):
        with self.assertRaises(ValueError):
            chunk_files(["chunk-00000.json", "chunk-00000.mnsd"])
        self.assertEqual(chunk_files(["chunk-00000.mnsd", "chunk-00000.json.tmp"]), [])

    def test_publish_network_is_read_only_and_rejects_different_existing_bytes(self):
        source = self.root / "cycles/cycle-0001/train/model.onnx"
        content = b"trained network"
        write(source, content)
        result = publish_network(self.options, 1)
        target = self.root / "nets/cycle-0001.onnx"
        self.assertEqual(result, digest(source))
        self.assertEqual(target.read_bytes(), content)
        self.assertEqual(target.stat().st_mode & 0o222, 0)
        self.assertEqual(publish_network(self.options, 1), result)
        write(source, b"different network")
        with self.assertRaises(ValueError):
            publish_network(self.options, 1)
        self.assertEqual(target.read_bytes(), content)


class GenerationTests(RunnerCase):
    def test_generation_rejects_a_different_start_position(self):
        path = self.root / self.out / "config.json"
        saved = json.loads(path.read_text())
        saved["start_position"] = "shuffled-setup"
        write_json(path, saved)
        with self.assertRaisesRegex(ValueError, "generation settings differ"):
            self.generation()

    def setUp(self):
        super().setUp()
        self.out = "cycles/cycle-0001/selfplay"
        self.summary_path = self.root / self.out / "summary.json"
        write_json(self.root / self.out / "config.json", {
            "generation_commit": GENERATION_COMMIT,
            "network_sha256": digest(self.root / "cycles/cycle-0000/model.onnx"),
            "start_position": "random-opening", "nn_backend": "tensorrt", "seed": 1000000000, "games": 11, "chunk_games": 5,
            "simulations": 32, "playthrough_percent": 100,
        })
        self.saved_summary = {"target_positions": 1000, "chunks": 2, "games": 10}
        write_json(self.summary_path, self.saved_summary)
        for chunk in range(3):
            for suffix in (".mnsd", ".resigned.mnsd", ".json"):
                write(self.root / self.out / f"chunk-{chunk:05d}{suffix}", b"committed chunk")
        for suffix in (".out", ".err"):
            write(self.root / (self.out + suffix), b"generation log")

    def generation(self):
        return self.runner.generation(1, self.out, "cycles/cycle-0000/model.onnx",
                                      1000000000, 11, 5, 100, 1000)

    def test_generation_accepts_and_preserves_chunks_after_the_target_prefix(self):
        before = {path: path.read_bytes() for path in (self.root / self.out).iterdir()}
        summary, _ = self.generation()
        self.assertEqual(summary, self.saved_summary)
        for path, content in before.items():
            self.assertEqual(path.read_bytes(), content)
        self.assertEqual(self.remote.starts, [])

    def test_generation_does_not_require_chunks_after_the_target_prefix(self):
        for suffix in (".mnsd", ".resigned.mnsd", ".json"):
            (self.root / self.out / f"chunk-00002{suffix}").unlink()
        summary, _ = self.generation()
        self.assertEqual(summary, self.saved_summary)

    def test_generation_rejects_target_mismatch_invalid_chunks_and_games_above_the_cap(self):
        # 上限より多い塊のファイルが揃っていても、集計の塊数は上限を超えられない。
        for suffix in (".mnsd", ".resigned.mnsd", ".json"):
            write(self.root / self.out / f"chunk-00003{suffix}", b"extra chunk")
        for field, invalid in [
            ("target_positions", 999), ("target_positions", 1000.0), ("target_positions", True),
            ("chunks", 0), ("chunks", -1), ("chunks", 4), ("chunks", 2.0),
            ("chunks", True), ("chunks", None), ("chunks", "2"),
            ("games", 12), ("games", 0), ("games", -1), ("games", 10.0), ("games", True),
        ]:
            with self.subTest(field=field, invalid=invalid):
                write_json(self.summary_path, {**self.saved_summary, field: invalid})
                with self.assertRaises(ValueError):
                    self.generation()

    def test_generation_requires_all_three_files_for_every_included_chunk(self):
        for chunk in range(2):
            for suffix in (".mnsd", ".resigned.mnsd", ".json"):
                with self.subTest(chunk=chunk, suffix=suffix):
                    path = self.root / self.out / f"chunk-{chunk:05d}{suffix}"
                    contents = path.read_bytes()
                    path.unlink()
                    with self.assertRaises(ValueError):
                        self.generation()
                    write(path, contents)


class ResumeTests(RunnerCase):
    def test_training_uses_four_processes_and_floors_replay_threads_for_new_and_resumed_cycles(self):
        self.remote.hardware.update(gpu_count=4, cpus_available=18)
        options = replace(self.options, machine="vast", vast_instance=12345, session=None, cycle_cu=None,
                          batch_size=1024)
        runner = Runner(options, self.remote)
        files = ["cycles/cycle-0001/selfplay/chunk-00000.mnsd"]
        prefix = ["uv", "run", "--frozen", "--project", "/workspace/src/tools/train/zero", "torchrun",
                  "--standalone", "--nproc-per-node", "4", "-m", "minase_zero", "train"]
        command = runner.training_command(1, files, files)
        self.assertEqual(command[:len(prefix)], prefix)
        self.assertEqual(command[command.index("--batch-size") + 1], "1024")
        self.assertEqual(command[command.index("--micro-batch-size") + 1], "256")
        self.assertEqual(command[command.index("--replay-threads") + 1], "4")
        checkpoint = "cycles/cycle-0001/train/checkpoint.pt"
        write(self.root / checkpoint, b"saved checkpoint")
        for location in ("nas", "remote"):
            with self.subTest(location=location):
                command = runner.training_command(1, files, files)
                self.assertEqual(command, [*prefix, "--resume", "cycles/cycle-0001/train",
                                           "--device", "cuda", "--replay-threads", "4"])
                self.assertNotIn("--micro-batch-size", command)
        self.assertEqual(self.remote.uploads, [checkpoint])

    def test_training_requires_one_cpu_per_gpu_before_upload_or_launch(self):
        self.remote.hardware.update(gpu_count=4, cpus_available=3)
        for checkpoint in (None, self.root, self.remote_root):
            with self.subTest(checkpoint=checkpoint):
                if checkpoint is not None:
                    write(checkpoint / "cycles/cycle-0001/train/checkpoint.pt", b"checkpoint")
                with self.assertRaisesRegex(ValueError, "CPU"):
                    self.runner.training(1, ["unavailable.mnsd"], ["unavailable.mnsd"])
                self.assertEqual(self.remote.starts, [])
                self.assertEqual(self.remote.uploads, [])
                self.assertFalse((self.root / "cycles/cycle-0001/train-data.json").exists())

    def test_new_training_manifest_preserves_order_and_identical_bytes(self):
        files = ["old/a.mnsd", "new/局面 b.mnsd"]
        command = self.runner.training_command(1, files, files[-1:])
        path = self.root / "cycles/cycle-0001/train-data.json"
        self.assertEqual(json.loads(path.read_text(encoding="utf-8")), files)
        self.assertEqual(command[command.index("--data-list") + 1], "cycles/cycle-0001/train-data.json")
        self.assertNotIn("--data", command)
        os.utime(path, ns=(1_000_000_000, 1_000_000_000))
        before = path.read_bytes(), path.stat().st_mtime_ns
        self.runner.training_command(1, files, files[-1:])
        self.assertEqual((path.read_bytes(), path.stat().st_mtime_ns), before)
        self.assertFalse((self.root / "cycles/cycle-0001/train").exists())

    def test_different_training_manifest_stops_before_upload_or_launch(self):
        path = self.root / "cycles/cycle-0001/train-data.json"
        for before in (b'["different.mnsd"]\n', b'[ "expected.mnsd" ]\n'):
            with self.subTest(contents=before):
                write(path, before)
                with self.assertRaisesRegex(ValueError, "train-data.json"):
                    self.runner.training(1, ["expected.mnsd"], ["expected.mnsd"])
                self.assertEqual(path.read_bytes(), before)
                self.assertEqual(self.remote.starts, [])
                self.assertEqual(self.remote.uploads, [])

    def test_incomplete_generation_rejects_a_different_saved_commit_before_launch(self):
        out = "cycles/cycle-0001/selfplay"
        for location in (self.root, self.remote_root):
            with self.subTest(location=location):
                write_json(location / out / "config.json", {
                    "generation_commit": "c" * 40,
                    "network_sha256": digest(self.root / "cycles/cycle-0000/model.onnx"),
                    "start_position": "random-opening", "nn_backend": "tensorrt", "seed": 1000000000, "games": 10, "chunk_games": 5,
                    "simulations": 32, "playthrough_percent": 100,
                })
                with self.assertRaises(ValueError):
                    self.runner.generation(1, out, "cycles/cycle-0000/model.onnx", 1000000000, 10, 5, 100, 1000)
                self.assertEqual(self.remote.starts, [])
                self.assertEqual(self.remote.uploads, [])
                self.assertFalse((self.root / out / "summary.json").exists())
                (self.root / out / "config.json").unlink()

    def test_remote_checkpoint_takes_precedence_over_nas_checkpoint(self):
        relative = "cycles/cycle-0001/train/checkpoint.pt"
        write(self.remote_root / relative, b"new checkpoint")
        write(self.root / relative, b"older checkpoint")
        files = ["cycles/cycle-0001/selfplay/chunk-00000.mnsd"]
        command = self.runner.training_command(1, files, files)
        self.assertEqual(command[command.index("train"):], ["train", "--resume", "cycles/cycle-0001/train", "--device", "cuda", "--replay-threads", "12"])
        self.assertEqual(self.remote.uploads, [])

    def test_nas_checkpoint_is_uploaded_for_resume(self):
        relative = "cycles/cycle-0001/train/checkpoint.pt"
        write(self.root / relative, b"saved checkpoint")
        files = ["cycles/cycle-0001/selfplay/chunk-00000.mnsd"]
        command = self.runner.training_command(1, files, files)
        self.assertEqual(command[command.index("train"):], ["train", "--resume", "cycles/cycle-0001/train", "--device", "cuda", "--replay-threads", "12"])
        self.assertEqual(self.remote.uploads, [relative])
        self.assertEqual((self.remote_root / relative).read_bytes(), b"saved checkpoint")

    def test_branch_training_uses_only_its_checkpoint_and_the_explicit_initial_cycle(self):
        options = replace(self.options, cycles_dir=Path("cycles-branch"), branch_from=2,
                          first_cycle=3, last_cycle=4, cycle1_reference_summary=None)
        runner = Runner(options, self.remote)
        write(self.remote_root / "cycles/cycle-0003/train/checkpoint.pt", b"mainline checkpoint")
        write(self.root / "cycles/cycle-0003/train/checkpoint.pt", b"mainline checkpoint")
        for cycle, initial in ((3, "cycles/cycle-0002/train/checkpoint.pt"),
                               (4, "cycles-branch/cycle-0003/train/checkpoint.pt")):
            files = [f"cycles-branch/cycle-{cycle:04d}/selfplay/chunk-00000.mnsd"]
            command = runner.training_command(cycle, files, files)
            self.assertEqual(command[command.index("--init") + 1], initial)
            self.assertEqual(command[command.index("--out-dir") + 1], f"cycles-branch/cycle-{cycle:04d}/train")
        checkpoint = "cycles-branch/cycle-0003/train/checkpoint.pt"
        write(self.root / checkpoint, b"branch checkpoint")
        for location in ("nas", "remote"):
            with self.subTest(location=location):
                command = runner.training_command(3, files, files)
                self.assertEqual(command[command.index("train"):],
                                 ["train", "--resume", "cycles-branch/cycle-0003/train", "--device", "cuda", "--replay-threads", "12"])
                self.assertEqual((self.remote_root / checkpoint).read_bytes(), b"branch checkpoint")
        self.assertEqual(self.remote.uploads, [checkpoint])

    def test_new_training_passes_replay_buffer_and_every_explicit_setting(self):
        latest = ["cycles/cycle-0003/selfplay/chunk-00000.mnsd",
                  "cycles/cycle-0003/selfplay-regen/chunk-00000.mnsd"]
        files = ["cycles/cycle-0002/selfplay/chunk-00000.mnsd", *latest]
        command = self.runner.training_command(3, files, latest)
        self.assertEqual(command, [
            "uv", "run", "--frozen", "--project", "/content/src/tools/train/zero",
            "torchrun", "--standalone", "--nproc-per-node", "1", "-m", "minase_zero", "train", "--init", "cycles/cycle-0002/train/checkpoint.pt",
            "--data-list", "cycles/cycle-0003/train-data.json", "--latest-data", *latest,
            "--out-dir", "cycles/cycle-0003/train", "--cycle", "3", "--seed", "126",
            "--batch-size", "256", "--lr", "0.01", "--momentum", "0.9", "--weight-decay", "0.0001",
            "--clip-grad-norm", "10", "--value-lambda", "0.333", "--precision", "bf16",
            "--checkpoint-steps", "200",
            "--micro-batch-size", "256", "--device", "cuda", "--replay-threads", "12",
            "--trainer-commit", TRAINER_COMMIT,
        ])
        first_files = ["cycles/cycle-0001/selfplay/chunk-00000.mnsd"]
        first = self.runner.training_command(1, first_files, first_files)
        self.assertEqual(first[first.index("--init") + 1], "cycles/cycle-0000/checkpoint.pt")

    def test_completed_cycle_is_skipped_without_remote_jobs(self):
        append_cycle(self.root / "cycles.jsonl", {"cycle": 1})
        write(self.root / "nets/cycle-0001.onnx", b"published")
        self.runner.run()
        self.assertEqual(self.remote.machine_calls, 0)
        self.assertEqual(len(self.remote.balances), 4)
        self.assertEqual(self.remote.starts, [])
        self.assertFalse((self.root / "seed-ledger.jsonl").exists())

    def test_record_without_network_is_an_error(self):
        append_cycle(self.root / "cycles.jsonl", {"cycle": 1})
        with self.assertRaises(ValueError):
            self.runner.run()
        self.assertEqual(self.remote.starts, [])

    def test_duplicate_rows_are_rejected_before_remote_work(self):
        write(self.root / "cycles.jsonl", b'{"cycle":1}\n{"cycle":1}\n')
        write(self.root / "nets/cycle-0001.onnx", b"network")
        with self.assertRaises(ValueError):
            self.runner.run()
        self.assertEqual(self.remote.machine_calls, 0)

    def test_partial_final_registration_is_reported(self):
        append_cycle(self.root / "sessions.jsonl", {"cycle": 1})
        with self.assertRaises(ValueError):
            self.runner.run()
        self.assertEqual(self.remote.machine_calls, 0)

    def test_insufficient_deadline_or_balance_does_not_start_cycle(self):
        self.execute(replace(self.options, deadline=NOW + timedelta(minutes=39)))
        self.assertEqual(len(self.remote.balances), 4)
        self.remote.balances = [1.99]
        self.runner.run()
        self.assertEqual(self.remote.starts, [])
        self.assertEqual(self.remote.machine_calls, 0)
        self.assertFalse((self.root / "seed-ledger.jsonl").exists())


class RemoteTests(unittest.TestCase):
    def test_vast_helpers_load_from_the_repository_and_require_the_file(self):
        repository = Path(__file__).resolve().parents[4]
        self.assertEqual(Path(load_vast_common().__file__), repository / "scripts/vast/common.py")
        with tempfile.TemporaryDirectory() as directory:
            absent = Path(directory) / "tools/train/zero/minase_zero/runner.py"
            with patch.object(runner_module, "__file__", str(absent)), self.assertRaises(FileNotFoundError):
                load_vast_common()

    def test_vast_ssh_and_rsync_share_one_resolved_address_and_connection(self):
        common = load_vast_common()
        for host, rsync_host in (("192.0.2.10", "192.0.2.10"), ("2001:db8::10", "[2001:db8::10]")):
            with self.subTest(host=host), patch.object(common, "ssh_address", return_value=(host, 22222)) as address, \
                    patch("minase_zero.runner.load_vast_common", return_value=common), \
                    patch("minase_zero.runner.subprocess.run",
                          return_value=subprocess.CompletedProcess([], 0, "yes\n", "")) as run:
                remote = Remote(None)
                remote.connect("vast", 12345)
                self.assertTrue(remote.exists("model.onnx"))
                remote.upload(Path("/NAS"), ["checkpoint.pt", "data.mnsd"], 121)
                remote.download("model.onnx", Path("/tmp/model.onnx"), 420)
                self.assertTrue(remote.exists("checkpoint.pt"))
                address.assert_called_once_with(12345, 600)
                ssh = run.call_args_list[0].args[0]
                self.assertEqual(ssh[ssh.index("-i") + 1], str(Path.home() / ".ssh/id_vastai"))
                self.assertEqual(ssh[ssh.index("-p") + 1], "22222")
                self.assertEqual(ssh[-2], f"root@{host}")
                for option in ("BatchMode=yes", "StrictHostKeyChecking=accept-new", "ConnectTimeout=20",
                               "ServerAliveInterval=30", "ControlMaster=auto",
                               "ControlPath=/tmp/vast-cm-12345", "ControlPersist=900"):
                    self.assertIn(option, ssh)
                self.assertFalse(any("colab" in arg for arg in ssh))
                for call in run.call_args_list[1:3]:
                    argv = call.args[0]
                    self.assertEqual(argv[0], "rsync")
                    self.assertEqual(shlex.split(argv[argv.index("-e") + 1]), ssh[:-2])
                self.assertEqual(run.call_args_list[1].args[0][-1], f"root@{rsync_host}:/workspace/az/")
                self.assertEqual(run.call_args_list[2].args[0][-2], f"root@{rsync_host}:/workspace/az/model.onnx")

    def test_vast_retries_connections_without_resolving_the_address_again(self):
        common = load_vast_common()
        for code, detail in ((255, "connection lost"), (1, "HTTP 429 Too Many Requests")):
            with self.subTest(code=code), patch.object(common, "ssh_address", return_value=("192.0.2.10", 22222)) as address, \
                    patch("minase_zero.runner.load_vast_common", return_value=common), \
                    patch("minase_zero.runner.time.sleep") as sleep, \
                    patch("minase_zero.runner.subprocess.run",
                          return_value=subprocess.CompletedProcess([], code, "", detail)) as run:
                remote = Remote(None)
                remote.connect("vast", 12345)
                with self.assertRaises(RuntimeError):
                    remote.exists("model.onnx")
                self.assertEqual(run.call_count, 6)
                self.assertEqual([call.args for call in sleep.call_args_list], [(30,)] * 5)
                address.assert_called_once_with(12345, 600)

    def test_detached_jobs_use_each_machines_work_source_and_cache(self):
        common = load_vast_common()
        for kind, base, env, session, instance in (
                ("colab", "/content", "/content/src/tools/train/zero/env.sh", "test", None),
                ("vast", "/workspace", "/opt/minase-zero/env.sh", None, 12345)):
            with self.subTest(kind=kind), tempfile.TemporaryDirectory() as directory, \
                    patch.object(common, "ssh_address", return_value=("192.0.2.10", 22222)), \
                    patch("minase_zero.runner.load_vast_common", return_value=common):
                root = Path(directory)
                write(root / "env.sh", b"export TEST_ENV_LOADED=yes\n")
                remote = Remote(session)
                remote.connect(kind, instance)

                def execute(argv, timeout, input_text):
                    command = shlex.split(argv[-1])
                    script = command[-1]
                    self.assertTrue(script.startswith(f"set -e\ncd {base}/az\n"))
                    self.assertIn(f"source {env}\n", script)
                    script = script.replace(f"cd {base}/az\n", f"cd {shlex.quote(str(root))}\n")
                    script = script.replace(f"source {env}",
                                            "source " + shlex.quote(str(root / "env.sh")))
                    script = 'setsid() { "$@"; }\n' + script.replace("< /dev/null &\n", "< /dev/null\n")
                    return subprocess.run(["bash", "-c", script], capture_output=True, text=True,
                                          timeout=timeout, check=True).stdout

                with patch.object(remote, "_call", side_effect=execute):
                    remote.start("job", 'printf "%s\\n" "$TEST_ENV_LOADED" "$XDG_CACHE_HOME" "$PYTHONPATH"',
                                 "job.out", "job.err")
                self.assertEqual((root / "job.out").read_text().splitlines(),
                                 ["yes", base + "/cache", base + "/src/tools/train/zero"])
                self.assertEqual((root / "job.exit").read_text(), "0\n")

    def test_large_missing_list_uses_stdin_below_linux_argument_limit(self):
        files = [f"cycles/{number:05d}/" + "局面" * 20 + ".mnsd" for number in range(2000)]
        self.assertGreater(len("".join(files).encode("utf-8")), 131072)

        def run(argv, **kwargs):
            self.assertTrue(all(len(arg.encode("utf-8")) < 131072 for arg in argv))
            self.assertEqual(json.loads(kwargs["input"]), files)
            return subprocess.CompletedProcess(argv, 0, json.dumps(files[::2]), "")

        with patch("minase_zero.runner.subprocess.run", side_effect=run):
            self.assertEqual(colab_remote("test").missing(files), files[::2])

    def test_large_sha256_list_uses_nul_stdin_below_linux_argument_limit(self):
        files = [f"cycles/{number:05d}/" + "局面" * 20 + " '\n.mnsd" for number in range(2000)]
        self.assertGreater(len("".join(files).encode("utf-8")), 131072)
        expected = {name: hashlib.sha256(name.encode("utf-8")).hexdigest() for name in files}

        def run(argv, **kwargs):
            self.assertTrue(all(len(arg.encode("utf-8")) < 131072 for arg in argv))
            self.assertEqual(kwargs["input"], "\0".join(files) + "\0")
            output = "".join(f"{checksum}  {name}\0" for name, checksum in expected.items())
            return subprocess.CompletedProcess(argv, 0, output, "")

        with patch("minase_zero.runner.subprocess.run", side_effect=run):
            self.assertEqual(colab_remote("test").sha256_many(files, 120), expected)

    def test_batch_transfer_uses_one_files_from_rsync_and_one_checksum_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "NAS with spaces"
            destination = Path(directory) / "colab"
            destination.mkdir()
            files = ["old/chunk-00000.mnsd", "new/chunk-00001.mnsd", "new/chunk-00000.mnsd"]
            for name, size in zip(files, (1000000, 2000000, 500000)):
                write(root / name, b"x" * size)
            write(destination / files[0], (root / files[0]).read_bytes())
            remote = colab_remote("local")
            commands = []
            manifests = []
            original_run = subprocess.run

            def execute(argv, **kwargs):
                commands.append((argv, kwargs["timeout"]))
                if argv[0] == "rsync":
                    manifest = Path(argv[argv.index("--files-from") + 1])
                    manifests.append((manifest, manifest.read_bytes()))
                    self.assertIn("--from0", argv)
                    self.assertEqual(shlex.split(argv[argv.index("-e") + 1]), remote.ssh)
                    self.assertEqual(argv[-2:], [str(root) + "/", "root@colab:/content/az/"])
                    # 同じrsyncの引数で、転送先だけをローカルのディレクトリへ置き換える。
                    transport = argv.index("-e")
                    local = argv[:transport] + argv[transport + 2:-1] + [str(destination) + "/"]
                    return original_run(local, **kwargs)
                self.assertEqual(argv[0], "ssh")
                command = shlex.split(argv[-1])
                command[-1] = command[-1].replace("cd /content/az\n", "cd " + shlex.quote(str(destination)) + "\n")
                return original_run(command, **kwargs)

            with patch("minase_zero.runner.subprocess.run", side_effect=execute):
                send(remote, root, files)
            self.assertEqual(len(commands), 3)
            self.assertEqual(commands[0][1], 120)
            self.assertEqual(commands[1][0][0], "rsync")
            self.assertEqual(commands[1][1], 122.5)
            checksum_script = shlex.split(commands[2][0][-1])[-1].splitlines()[-1]
            self.assertEqual(shlex.split(checksum_script), ["xargs", "-0", "-r", "sha256sum", "--zero", "--"])
            self.assertEqual(commands[2][1], 122.5)
            self.assertEqual(len(manifests), 1)
            self.assertEqual(manifests[0][1], b"new/chunk-00001.mnsd\0new/chunk-00000.mnsd\0")
            self.assertFalse(manifests[0][0].exists())
            for name in files:
                self.assertEqual((destination / name).read_bytes(), (root / name).read_bytes())

    def test_batch_checksum_rejects_missing_duplicate_or_malformed_results(self):
        remote = colab_remote("local")
        first = "a" * 64 + "  first.mnsd\0"
        second = "b" * 64 + "  second.mnsd\0"
        for output in (first, first + first, first + second[:-1], first + "invalid\0",
                       first + second + "c" * 64 + "  unexpected.mnsd\0"):
            with self.subTest(output=output), patch.object(remote, "shell", return_value=output):
                with self.assertRaises(ValueError):
                    remote.sha256_many(["first.mnsd", "second.mnsd"], 120)

    def test_failed_batch_transfer_removes_its_file_list(self):
        remote = colab_remote("local")
        manifests = []

        def fail(argv, **kwargs):
            manifests.append(Path(argv[argv.index("--files-from") + 1]))
            self.assertTrue(manifests[0].is_file())
            return subprocess.CompletedProcess(argv, 23, "", "transfer failed")

        with patch("minase_zero.runner.subprocess.run", side_effect=fail), self.assertRaises(RuntimeError):
            remote.upload(Path("/NAS"), ["file.mnsd"], 121)
        self.assertFalse(manifests[0].exists())

    def test_remote_size_reads_decimal_bytes_with_stat(self):
        remote = colab_remote("local")
        relative = "-train output/checkpoint 'one'.pt"
        for output, expected in (("0\n", 0), ("300000000\n", 300_000_000)):
            with self.subTest(output=output), patch.object(remote, "shell", return_value=output) as shell:
                self.assertEqual(remote.size(relative), expected)
                shell.assert_called_once()
                self.assertEqual(shlex.split(shell.call_args.args[0]), ["stat", "-c", "%s", "--", relative])
                self.assertEqual(shell.call_args.args[1], 120)

    def test_remote_size_rejects_invalid_responses(self):
        remote = colab_remote("local")
        for output in ("", "\n", "-1\n", "+1\n", "1.5\n", "1_000\n", "0x10\n", "１２\n",
                       "not-an-integer\n", "12 bytes\n", "12\n34\n"):
            with self.subTest(output=output), patch.object(remote, "shell", return_value=output), \
                    self.assertRaises(ValueError):
                remote.size("checkpoint.pt")

    def test_download_passes_the_supplied_timeout_to_rsync(self):
        remote = colab_remote("local")
        for timeout in (120, 420, 121.25):
            with self.subTest(timeout=timeout), patch.object(remote, "_call", return_value="") as call:
                remote.download("checkpoint.pt", Path("/tmp/local-checkpoint"), timeout)
                call.assert_called_once()
                self.assertEqual(call.call_args.args[0][0], "rsync")
                self.assertEqual(call.call_args.args[1], timeout)

    def test_remote_copy_is_independent_of_original_and_can_be_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = "train output/checkpoint.pt"
            snapshot = "train output/.checkpoint.pt.snapshot"
            write(root / original, b"before replacement")
            remote = colab_remote("local")

            def shell(script, timeout):
                return subprocess.run(["bash", "-c", script], cwd=root, capture_output=True,
                                      text=True, check=True, timeout=timeout).stdout

            with patch.object(remote, "shell", side_effect=shell):
                remote.copy(original, snapshot)
                replacement = root / "train output/checkpoint.pt.tmp"
                write(replacement, b"after replacement")
                replacement.replace(root / original)
                self.assertEqual((root / snapshot).read_bytes(), b"before replacement")
                remote.remove(snapshot)
            self.assertFalse((root / snapshot).exists())
            self.assertEqual((root / original).read_bytes(), b"after replacement")

    def test_missing_remote_working_directory_stops_before_commands(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            remote = colab_remote("local")

            def execute(argv, timeout, input_text):
                command = shlex.split(argv[-1])
                command[-1] = command[-1].replace("/content/az", str(root / "absent"))
                return subprocess.run(command, input=input_text, cwd=root, capture_output=True, text=True, timeout=timeout)

            with patch.object(remote, "_call", side_effect=execute):
                result = remote.shell("echo first\nprintf wrong > marker", 120)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((root / "marker").exists())

    def test_ssh_and_rsync_share_control_connection_and_use_timeout(self):
        remote = colab_remote("l4-one")
        with patch("minase_zero.runner.subprocess.run", return_value=subprocess.CompletedProcess([], 0, "yes\n", "")) as run:
            self.assertTrue(remote.exists("cycles/cycle-0001/train/checkpoint.pt"))
            remote.download("cycles/cycle-0001/train/checkpoint.pt", Path("/tmp/local-checkpoint"), 420)
        ssh_call, rsync_call = run.call_args_list
        ssh = ssh_call.args[0]
        self.assertIn("ProxyCommand=colab ssh --proxy-mode -s l4-one", ssh)
        self.assertIn("ControlMaster=auto", ssh)
        self.assertIn("ControlPath=/tmp/colab-cm-l4-one", ssh)
        self.assertIn("ControlPersist=900", ssh)
        rsync = rsync_call.args[0]
        self.assertEqual(shlex.split(rsync[rsync.index("-e") + 1]), ssh[:ssh.index("root@colab")])
        self.assertEqual(ssh_call.kwargs["timeout"], 120)
        self.assertEqual(rsync_call.kwargs["timeout"], 420)

    def test_transient_connections_are_retried_at_most_five_times(self):
        for code, error in ((255, "connection lost"), (1, "HTTP 429 Too Many Requests"),
                            (12, "rsync: connection unexpectedly closed (code 255)")):
            with self.subTest(code=code), patch("minase_zero.runner.time.sleep") as sleep:
                remote = colab_remote("l4-one")
                with patch("minase_zero.runner.subprocess.run", return_value=subprocess.CompletedProcess([], code, "", error)) as run:
                    with self.assertRaises(RuntimeError):
                        if code == 12:
                            remote.download("model.onnx", Path("/tmp/unused"), 420)
                        else:
                            remote.usage()
                self.assertEqual(run.call_count, 6)
                self.assertEqual([call.args for call in sleep.call_args_list], [(30,)] * 5)

    def test_recovered_connection_returns_output(self):
        results = [subprocess.CompletedProcess([], 255, "", "connection lost"),
                   subprocess.CompletedProcess([], 0, "Current balance: 12.5 compute units\n", "")]
        with patch("minase_zero.runner.time.sleep"), patch("minase_zero.runner.subprocess.run", side_effect=results):
            self.assertEqual(colab_remote("l4-one").usage(), 12.5)

    def test_command_failure_and_timeout_are_not_retried(self):
        with patch("minase_zero.runner.time.sleep") as sleep:
            with patch("minase_zero.runner.subprocess.run", return_value=subprocess.CompletedProcess([], 2, "", "invalid command")) as run:
                with self.assertRaises(RuntimeError):
                    colab_remote("l4-one").usage()
                self.assertEqual(run.call_count, 1)
            with patch("minase_zero.runner.subprocess.run", side_effect=subprocess.TimeoutExpired("ssh", 120)) as run:
                with self.assertRaises(RuntimeError):
                    colab_remote("l4-one").exists("file")
                self.assertEqual(run.call_count, 1)
            sleep.assert_not_called()

    def test_remote_status_detects_live_dead_and_completed_processes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            remote = colab_remote("local")

            def shell(script, timeout):
                result = subprocess.run(["bash", "-c", script], cwd=root, capture_output=True, text=True, timeout=timeout)
                if result.returncode:
                    raise RuntimeError(result.stderr)
                return result.stdout

            with patch.object(remote, "shell", side_effect=shell):
                for stem in ("job", "experiments/branch's run/cycle-0001/train"):
                    with self.subTest(stem=stem):
                        self.assertEqual(remote.status(stem), "new")
                        write(root / (stem + ".pid"), str(os.getpid()).encode())
                        self.assertEqual(remote.status(stem), "running")
                        write(root / (stem + ".pid"), b"2147483647")
                        with self.assertRaisesRegex(RuntimeError, "process disappeared without an exit code"):
                            remote.status(stem)
                        write(root / (stem + ".exit"), b"7\n")
                        self.assertEqual(remote.status(stem), 7)

    def test_detached_start_redirects_all_streams_and_records_exit_after_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root / "env.sh", b"# test environment\n")
            remote = colab_remote("local")
            scripts = []

            def shell(script, timeout):
                scripts.append(script)
                script = script.replace("source " + remote.paths.env, "source " + str(root / "env.sh"))
                # setsidだけを同期実行するシェル関数にし、実際のジョブと入出力の契約を調べる。
                script = "setsid() { \"$@\"; }\n" + script.replace("< /dev/null &\n", "< /dev/null\n")
                result = subprocess.run(["bash", "-c", script], cwd=root, capture_output=True, text=True, timeout=5)
                if result.returncode:
                    raise RuntimeError(result.stderr)
                return result.stdout

            with patch.object(remote, "shell", side_effect=shell):
                remote.start("job", "echo output; echo problem >&2; exit 7", "job.out", "job.err")
                self.assertEqual(remote.status("job"), 7)
                remote.start("job", "echo must-not-run", "job.out", "job.err")
            self.assertEqual((root / "job.out").read_text(), "output\n")
            self.assertEqual((root / "job.err").read_text(), "problem\n")
            self.assertIn("source /content/src/tools/train/zero/env.sh\n", scripts[0])
            self.assertIn("setsid nohup bash -c", scripts[0])
            self.assertIn("< /dev/null &", scripts[0])
            self.assertIn("echo $! > job.pid", scripts[0])


class MachineTests(unittest.TestCase):
    """GPUの同一性とcgroupの境界値はphase3a.mdの照会規則から定める。"""

    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="minase-machine-")
        self.addCleanup(temporary.cleanup)
        self.cgroup = Path(temporary.name)
        self.gpus = "NVIDIA RTX 4090, 24564\n"
        self.cpuset = 8
        self.cpuinfo = "processor : 0\nmodel name : Test CPU\n"
        self.nproc = 32
        read_text = Path.read_text

        def read(path, *args, **kwargs):
            if path == Path("/proc/cpuinfo"):
                return self.cpuinfo
            return read_text(path, *args, **kwargs)

        def output(argv, **kwargs):
            self.assertTrue(kwargs["text"])
            if argv == ["nproc"]:
                return f"{self.nproc}\n"
            self.assertEqual(argv, ["nvidia-smi", "--query-gpu=name,memory.total", "--format=csv,noheader,nounits"])
            return self.gpus

        for target, replacement in (("pathlib.Path.read_text", read),
                                     ("subprocess.check_output", output),
                                     ("os.sched_getaffinity", lambda pid: set(range(self.cpuset)))):
            mock = patch(target, replacement)
            mock.start()
            self.addCleanup(mock.stop)

    def test_gpu_count_accepts_one_or_four_identical_rows(self):
        write(self.cgroup / "cpu.max", b"max 100000\n")
        for count in (1, 4):
            with self.subTest(count=count):
                self.gpus = "NVIDIA RTX 4090, 24564\n" * count
                self.assertEqual(machine_info(str(self.cgroup)), {
                    "gpu": "NVIDIA RTX 4090", "gpu_memory_mib": 24564, "gpu_count": count,
                    "cpu": "Test CPU", "nproc": 32, "cpus_available": 8,
                })

    def test_gpu_query_rejects_empty_mixed_and_incomplete_rows(self):
        write(self.cgroup / "cpu.max", b"max 100000\n")
        for output in ("", " \n", "NVIDIA RTX 4090, 24564\nNVIDIA RTX 4080, 24564\n",
                       "NVIDIA RTX 4090, 24564\nNVIDIA RTX 4090, 24563\n",
                       "NVIDIA RTX 4090\n", "NVIDIA RTX 4090, N/A\n", ", 24564\n",
                       "NVIDIA RTX 4090, 0\n"):
            with self.subTest(output=output), self.assertRaises(ValueError):
                self.gpus = output
                machine_info(str(self.cgroup))

    def test_cgroup_v2_uses_the_smaller_of_cpuset_and_floored_quota(self):
        for allocation, cpuset, expected in (("max 100000", 8, 8), ("400000 100000", 8, 4),
                                             ("350000 100000", 8, 3), ("400000 100000", 2, 2),
                                             ("50000 100000", 8, 0)):
            with self.subTest(allocation=allocation, cpuset=cpuset):
                write(self.cgroup / "cpu.max", allocation.encode())
                self.cpuset = cpuset
                self.assertEqual(machine_info(str(self.cgroup))["cpus_available"], expected)

    def test_machine_query_requires_cpu_name_nproc_and_affinity(self):
        write(self.cgroup / "cpu.max", b"max 100000\n")
        for cpuinfo, nproc, cpuset in (("", 32, 8), ("model name : \n", 32, 8),
                                       ("model name : Test CPU\n", 0, 8),
                                       ("model name : Test CPU\n", "N/A", 8),
                                       ("model name : Test CPU\n", 32, 0)):
            with self.subTest(cpuinfo=cpuinfo, nproc=nproc, cpuset=cpuset), self.assertRaises(ValueError):
                self.cpuinfo, self.nproc, self.cpuset = cpuinfo, nproc, cpuset
                machine_info(str(self.cgroup))

    def test_cgroup_v1_uses_unlimited_or_floored_quota(self):
        write(self.cgroup / "cpu/cpu.cfs_period_us", b"100000\n")
        for quota, cpuset, expected in ((-1, 8, 8), (400000, 8, 4), (350000, 8, 3), (400000, 2, 2)):
            with self.subTest(quota=quota, cpuset=cpuset):
                write(self.cgroup / "cpu/cpu.cfs_quota_us", str(quota).encode())
                self.cpuset = cpuset
                self.assertEqual(machine_info(str(self.cgroup))["cpus_available"], expected)

    def test_cgroup_missing_or_invalid_allocation_is_an_error(self):
        with self.assertRaises(ValueError):
            machine_info(str(self.cgroup))
        for allocation in ("", "max", "max 0", "max -1", "0 100000", "-1 100000", "invalid 100000"):
            with self.subTest(allocation=allocation), self.assertRaises(ValueError):
                write(self.cgroup / "cpu.max", allocation.encode())
                machine_info(str(self.cgroup))
        (self.cgroup / "cpu.max").unlink()
        write(self.cgroup / "cpu/cpu.cfs_quota_us", b"-1\n")
        with self.assertRaises(FileNotFoundError):
            machine_info(str(self.cgroup))
        for quota, period in ((-1, 0), (0, 100000), (-2, 100000)):
            with self.subTest(quota=quota, period=period), self.assertRaises(ValueError):
                write(self.cgroup / "cpu/cpu.cfs_quota_us", str(quota).encode())
                write(self.cgroup / "cpu/cpu.cfs_period_us", str(period).encode())
                machine_info(str(self.cgroup))

    def test_remote_machine_executes_self_contained_probe_and_adds_machine_identity(self):
        write(self.cgroup / "cpu.max", b"350000 100000\n")
        common = load_vast_common()

        def shell(command, timeout):
            argv = shlex.split(command)
            self.assertEqual(argv[:2], ["python3", "-c"])
            self.assertEqual(timeout, 120)
            output = io.StringIO()
            script = argv[2].replace("'/sys/fs/cgroup'", repr(str(self.cgroup)))
            with redirect_stdout(output):
                exec(script, {})
            return output.getvalue()

        for kind, session, instance in (("colab", "test", None), ("vast", None, 12345)):
            with self.subTest(kind=kind), patch.object(common, "ssh_address", return_value=("192.0.2.10", 22222)), \
                    patch("minase_zero.runner.load_vast_common", return_value=common):
                remote = Remote(session)
                remote.connect(kind, instance)
                with patch.object(remote, "shell", side_effect=shell):
                    info = remote.machine()
                expected = {"gpu": "NVIDIA RTX 4090", "gpu_memory_mib": 24564, "gpu_count": 1,
                            "cpu": "Test CPU", "nproc": 32, "cpus_available": 3, "kind": kind}
                if kind == "vast":
                    expected["vast_instance"] = 12345
                self.assertEqual(info, expected)


class CycleTests(RunnerCase):
    def test_vast_cycles_record_machine_and_devices_without_balance_queries(self):
        self.remote.hardware.update(gpu_count=4, cpus_available=18, nproc=32)
        self.regenerate = True
        options = replace(self.options, machine="vast", vast_instance=12345, session=None, cycle_cu=None,
                          threads_per_gpu=4, batch_size=1024, last_cycle=2)
        common = load_vast_common()
        # 接続先の確定は実物を通し、転送とジョブは一時ディレクトリ内で実行する。
        with patch.object(common, "ssh_address", return_value=("192.0.2.10", 22222)) as address, \
                patch("minase_zero.runner.load_vast_common", return_value=common), \
                patch.object(self.remote, "connect", side_effect=lambda kind, instance: Remote.connect(self.remote, kind, instance)), \
                patch.object(self.remote, "usage", side_effect=AssertionError("vast must not query a balance")):
            self.execute(options)
        address.assert_called_once_with(12345, 600)
        self.assertEqual(self.remote.machine_calls, 1)
        self.assertEqual(self.remote.existence_checks[0], VAST_FIXTURE)
        rows = [json.loads(line) for line in (self.root / "cycles.jsonl").read_text().splitlines()]
        intervals = [json.loads(line) for line in (self.root / "sessions.jsonl").read_text().splitlines()]
        self.assertEqual([row["cycle"] for row in rows], [1, 2])
        for row, interval in zip(rows, intervals):
            self.assertEqual(row["machine"], {
                "gpu": "NVIDIA L4", "gpu_memory_mib": 23034, "gpu_count": 4,
                "cpu": "Test CPU", "nproc": 32, "cpus_available": 18,
                "kind": "vast", "vast_instance": 12345,
            })
            self.assertEqual(row["interval"], interval)
            self.assertEqual(interval["machine_kind"], "vast")
            self.assertEqual(interval["vast_instance"], 12345)
            for absent in ("session", "start_balance", "end_balance", "balance_difference", "balance_increased"):
                self.assertNotIn(absent, interval)
            for field in ("generation", "regeneration"):
                self.assertEqual(row[field]["threads"], 16)
                self.assertEqual(row[field]["threads_per_gpu"], 4)
                self.assertEqual(row[field]["devices"], 4)
        self.assertEqual(intervals[1]["started_at"], intervals[0]["ended_at"])
        for stem, body, _, _ in self.remote.starts:
            if stem.endswith("/check"):
                continue
            command = shlex.split(body)
            if "zero-selfplay" in command:
                self.assertEqual(command[0], "/workspace/src/target/release/minase")
                self.assertEqual(command[command.index("--threads") + 1], "16")
                self.assertEqual(command[command.index("--devices") + 1], "4")
            else:
                self.assertEqual(command[:13], [
                    "uv", "run", "--frozen", "--project", "/workspace/src/tools/train/zero", "torchrun",
                    "--standalone", "--nproc-per-node", "4", "-m", "minase_zero", "train", "--init",
                ])
                self.assertEqual(command[command.index("--replay-threads") + 1], "4")
                self.assertEqual(command[command.index("--micro-batch-size") + 1], "256")
        commands = [json.loads(line) for line in (self.remote_root / "commands.jsonl").read_text().splitlines()]
        self.assertEqual(len(commands), 26)
        for command in commands:
            if "fixture-eval" in command:
                self.assertEqual(command[:7], ["run", "--frozen", "--project", "/workspace/src/tools/train/zero",
                                               "python", "-m", "minase_zero"])
                self.assertEqual(command[command.index("--fixture") + 1], VAST_FIXTURE)
            elif "--candidate" in command:
                self.assertEqual(command[:6], ["run", "--frozen", "--project", "/workspace/src/tools/train/zero",
                                               "python", "/workspace/src/tools/train/zero/nn_compare.py"])
            else:
                self.assertEqual(command, ["--protocol", "usi", "--search", "mcts"])
        self.assert_usi_commands("/workspace/cache", (1, 2))

    def test_vast_deadline_rejects_overrun_and_accepts_equality_without_querying_balance(self):
        options = replace(self.options, machine="vast", vast_instance=12345, session=None, cycle_cu=None,
                          deadline=NOW + timedelta(minutes=40, microseconds=-1))
        with patch.object(self.remote, "usage", side_effect=AssertionError("vast must not query a balance")):
            self.execute(options)
            self.assertEqual(self.remote.starts, [])
            self.assertEqual(self.remote.machine_calls, 0)
            self.assertFalse((self.root / "cycles.jsonl").exists())
            self.execute(replace(options, deadline=NOW + timedelta(minutes=40)))
        self.assertEqual(len(self.remote.starts), 3)
        self.assertEqual(json.loads((self.root / "cycles.jsonl").read_text())["cycle"], 1)

    def test_generation_rejects_threads_above_available_cpus_and_accepts_equality(self):
        options = replace(self.options, threads_per_gpu=4, batch_size=1024)
        self.remote.hardware.update(gpu_count=4, cpus_available=15)
        with self.assertRaisesRegex(ValueError, "threads.*CPU"):
            self.execute(options)
        self.assertEqual(self.remote.starts, [])
        self.assertEqual(self.remote.uploads, [])
        self.remote.hardware["cpus_available"] = 16
        self.execute(options)
        command = shlex.split(self.remote.starts[0][1])
        self.assertEqual(command[command.index("--threads") + 1], "16")
        self.assertEqual(command[command.index("--devices") + 1], "4")

    def test_vast_resumes_colab_checkpoint_without_repeating_completed_generation(self):
        complete = self.remote.on_start

        def interrupt_training(stem, body):
            if stem.endswith("/train"):
                write(self.remote_root / stem / "checkpoint.pt", b"partial checkpoint")
                return 9
            return complete(stem, body)

        self.remote.on_start = interrupt_training
        with self.assertRaises(RuntimeError):
            self.runner.run()
        ledger = (self.root / "seed-ledger.jsonl").read_bytes()
        shutil.rmtree(self.remote_root)
        self.remote_root.mkdir()
        self.remote = FakeRemote(self.remote_root)
        self.remote.hardware.update(gpu_count=4, cpus_available=18)
        self.remote.on_start = self.complete_job
        # 生成済みなので、新しい機械で生成する場合のCPU上限は検査しない。
        options = replace(self.options, machine="vast", vast_instance=12345, session=None, cycle_cu=None,
                          threads_per_gpu=10)
        self.execute(options)
        self.assertEqual([Path(item[0]).name for item in self.remote.starts], ["train", "check"])
        self.assertEqual(shlex.split(self.remote.starts[0][1]), [
            "uv", "run", "--frozen", "--project", "/workspace/src/tools/train/zero", "torchrun",
            "--standalone", "--nproc-per-node", "4", "-m", "minase_zero", "train",
            "--resume", "cycles/cycle-0001/train", "--device", "cuda", "--replay-threads", "4",
        ])
        self.assertEqual((self.root / "seed-ledger.jsonl").read_bytes(), ledger)
        self.assertEqual(json.loads((self.root / "cycles.jsonl").read_text())["machine"]["vast_instance"], 12345)

    def test_start_position_reaches_generation_and_regeneration_and_cycle_record(self):
        self.regenerate = True
        self.execute(replace(self.options, start_position="shuffled-setup"))
        row = json.loads((self.root / "cycles.jsonl").read_text())
        self.assertEqual(row["generation_seed_offset"], 0)
        self.assertEqual(row["generation"]["seed"], 1_000_000_000)
        self.assertEqual(row["regeneration"]["seed"], 1_500_000_000)
        commands = [shlex.split(body) for stem, body, _, _ in self.remote.starts if "/selfplay" in stem]
        self.assertEqual(len(commands), 2)
        self.assertEqual([command[command.index("--start-position") + 1] for command in commands],
                         ["shuffled-setup", "shuffled-setup"])
        for field in ("generation", "regeneration"):
            self.assertEqual(row[field]["start_position"], "shuffled-setup")

    def test_generation_seed_interval_overflow_leaves_ledger_unchanged(self):
        write_json(self.reference, {"games": 1, "training_positions": 1})
        ledger = self.root / "seed-ledger.jsonl"
        before = ledger.read_bytes()
        options = replace(self.options, generation_seed_offset=300_000_000, target_positions=200_000_001)
        with self.assertRaisesRegex(ValueError, "generation seed range"):
            self.execute(options)
        self.assertEqual(ledger.read_bytes(), before)
        self.assertEqual(self.remote.starts, [])

    def test_generation_seed_interval_accepts_exact_upper_boundary(self):
        write_json(self.reference, {"games": 1, "training_positions": 1})
        options = replace(self.options, generation_seed_offset=300_000_000, target_positions=200_000_000)
        with patch.object(Runner, "generation", side_effect=RuntimeError("generation reached")):
            with self.assertRaisesRegex(RuntimeError, "generation reached"):
                self.execute(options)
        row = json.loads((self.root / "seed-ledger.jsonl").read_text())
        self.assertEqual((row["seed"], row["games"]), (1_300_000_000, 200_000_000))

    def test_new_training_sends_manifest_with_replay_buffer(self):
        self.runner.run()
        row = json.loads((self.root / "cycles.jsonl").read_text())
        manifest = "cycles/cycle-0001/train-data.json"
        for root in (self.root, self.remote_root):
            self.assertEqual(json.loads((root / manifest).read_text()), row["training_files"])
        self.assertIn(manifest, self.remote.uploads)
        batches = [files for files, _ in self.remote.hash_batches if manifest in files]
        self.assertEqual(batches, [[*row["training_files"], manifest]])
        command = shlex.split(next(body for stem, body, _, _ in self.remote.starts if stem.endswith("/train")))
        self.assertEqual(command[command.index("--data-list") + 1], manifest)
        self.assertNotIn("--data", command)

    def setUp(self):
        super().setUp()
        (self.root / "seed-ledger.jsonl").touch()
        self.compare_failure = False
        self.regenerate = False
        self.remote.on_start = self.complete_job
        self.tool = self.directory / "fake-tool"
        write(self.tool, b'''#!/usr/bin/env python3
import json, pathlib, sys
args = sys.argv[1:]
with open('commands.jsonl', 'a') as output:
    output.write(json.dumps(args) + '\\n')
if any(arg.endswith('/nn_compare.py') for arg in args):
    print(json.dumps({'positions': 288, 'max_value_abs_diff': 0.00001}))
    failure = pathlib.Path('fail-comparison')
    if failure.exists():
        name = 'cpu-b7.json'
        sys.exit(1 if args[args.index('--candidate') + 1].endswith(name) else 0)
    sys.exit(0)
engine = args == ['--protocol', 'usi', '--search', 'mcts']
if engine:
    commands = sys.stdin.read().splitlines()
    with open('usi-input.jsonl', 'a') as output:
        output.write(json.dumps(commands) + '\\n')
    paths = [line.removeprefix('nn-eval out ') for line in commands if line.startswith('nn-eval out ')]
    assert len(paths) == 1
    out = pathlib.Path(paths[0])
else:
    out = pathlib.Path(args[args.index('--out') + 1])
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps({'network_sha256': 'test-network', 'positions': []}) + '\\n')
if engine:
    if not pathlib.Path('missing-end').exists():
        print('info string end')
    if pathlib.Path('fail-engine').exists():
        sys.exit(9)
''')
        self.tool.chmod(0o755)

    def assert_usi_commands(self, cache, cycles):
        actual = [json.loads(line) for line in (self.remote_root / "usi-input.jsonl").read_text().splitlines()]
        expected = []
        for cycle in cycles:
            path = f"cycles/cycle-{cycle:04d}"
            for batch in (1, 7, 256):
                for backend, label in (("cpu", "cpu"), ("tensorrt", "trt")):
                    commands = ["usi", "setoption name RuleSet value JCSA",
                                f"setoption name NnModel value {path}/train/model.onnx",
                                f"setoption name NnBackend value {backend}",
                                f"setoption name NnBatchSize value {batch}"]
                    if backend == "tensorrt":
                        commands.append(f"setoption name NnCacheDir value {cache}/nn-eval")
                    expected.append([*commands, "isready", f"nn-eval out {path}/check/{label}-b{batch}.json", "quit"])
        self.assertEqual(actual, expected)

    def test_missing_diagnostic_end_fails_check_and_blocks_next_cycle(self):
        write(self.remote_root / "missing-end", b"")
        with self.assertRaisesRegex(RuntimeError, "network check exited 1"):
            self.execute(replace(self.options, last_cycle=2))
        self.assertFalse((self.root / "cycles.jsonl").exists())
        self.assertFalse((self.root / "nets/cycle-0001.onnx").exists())
        self.assertEqual([Path(item[0]).name for item in self.remote.starts], ["selfplay", "train", "check"])
        self.assertNotIn("info string end", (self.root / "cycles/cycle-0001/check/cpu-b1.usi.log").read_text())

    def test_engine_nonzero_exit_fails_check_even_with_end(self):
        write(self.remote_root / "fail-engine", b"")
        with self.assertRaisesRegex(RuntimeError, "network check exited 1"):
            self.execute(replace(self.options, last_cycle=2))
        self.assertFalse((self.root / "cycles.jsonl").exists())
        self.assertFalse((self.root / "nets/cycle-0001.onnx").exists())
        self.assertEqual([Path(item[0]).name for item in self.remote.starts], ["selfplay", "train", "check"])
        self.assertEqual((self.root / "cycles/cycle-0001/check/cpu-b1.usi.log").read_text(), "info string end\n")

    def test_check_rejects_newlines_in_paths_before_starting(self):
        for newline in ("\n", "\r"):
            with self.subTest(newline=newline):
                with patch.object(self.runner.paths, "cache", f"/cache{newline}quit"):
                    with self.assertRaisesRegex(ValueError, "USI paths must not contain a newline"):
                        self.runner.check_script(1)
                with patch.object(Options, "cycle_path", return_value=f"cycles{newline}quit"):
                    with self.assertRaisesRegex(ValueError, "USI paths must not contain a newline"):
                        self.runner.check_script(1)
        self.assertEqual(self.remote.starts, [])

    def test_check_quotes_shell_characters_and_preserves_spaces_in_usi_paths(self):
        cache = "/cache with spaces/'$`literal`"
        with patch.object(self.runner.paths, "cache", cache):
            body = self.runner.check_script(1)
            self.assertEqual(self.complete_job("cycles/cycle-0001/check", body), 0)
        self.assert_usi_commands(cache, (1,))

    def complete_job(self, stem, body):
        if stem.endswith("/check"):
            if self.compare_failure:
                write(self.remote_root / "fail-comparison", b"")
            script = body.replace(self.remote.paths.engine, shlex.quote(str(self.tool)))
            script = script.replace("uv run", shlex.quote(str(self.tool)) + " run")
            result = subprocess.run(["bash", "-eu", "-o", "pipefail", "-c", script], cwd=self.remote_root,
                                    capture_output=True, text=True, timeout=20)
            self.assertIn(result.returncode, (0, 1), result.stderr)
            return result.returncode
        command = shlex.split(body)
        if "zero-selfplay" in command:
            value = lambda flag: command[command.index(flag) + 1]
            count, chunk = int(value("--games")), int(value("--chunk-games"))
            config = {"generation_commit": value("--generation-commit"),
                      "network_sha256": digest(self.remote_root / value("--nn-model")),
                      "start_position": value("--start-position"), "nn_backend": "tensorrt", "seed": int(value("--seed")), "games": count,
                      "chunk_games": chunk, "simulations": int(value("--simulations")),
                      "playthrough_percent": int(value("--playthrough-percent"))}
            write_json(self.remote_root / stem / "config.json", config)
            excluded = self.regenerate and stem.endswith("/selfplay")
            files = []
            for index in range((count + chunk - 1) // chunk):
                name = f"chunk-{index:05d}"
                write(self.remote_root / stem / (name + ".mnsd"), b"teacher data")
                write(self.remote_root / stem / (name + ".resigned.mnsd"), b"resigned data")
                write_json(self.remote_root / stem / (name + ".json"), {"chunk": index})
                files.append(name + ".mnsd")
                if not excluded:
                    files.append(name + ".resigned.mnsd")
            summary = {"games": count, "training_positions": count * 100, "training_files": files,
                       "target_positions": int(value("--target-positions")), "chunks": (count + chunk - 1) // chunk,
                       "resign_triggered_playthrough_games": 100, "false_resignations": 0,
                       "resigned_games_excluded": excluded, "resigned_games": 3 if excluded else 0,
                       "resigned_positions": 300 if excluded else 0,
                       "win_reason_royal_capture": count, "discarded_max_ply": 0}
            write_json(self.remote_root / stem / "summary.json", summary)
        elif "train" in command:
            cycle = int(Path(stem).parent.name.removeprefix("cycle-"))
            write(self.remote_root / stem / "checkpoint.pt", f"checkpoint {cycle}".encode())
            write(self.remote_root / stem / "model.onnx", f"network {cycle}".encode())
            summary = {
                "cycle": cycle, "network_sha256": digest(self.remote_root / stem / "model.onnx"),
                "provenance": {"trainer_commit": TRAINER_COMMIT, "environment_commit": TRAINER_COMMIT},
                "diagnostics": {"validation": {"value_result": 0.7},
                                "latest_training_sample": {"value_result": 0.6}},
                "sample_count": 900, "sample_sha256": "d" * 64, "completed_steps": 4,
            }
            if "--latest-data" in command:
                summary["latest_data"] = command[command.index("--latest-data") + 1:command.index("--out-dir")]
                summary["provenance"]["trainer_commit"] = command[command.index("--trainer-commit") + 1]
            write_json(self.remote_root / stem / "summary.json", summary)
        else:
            raise AssertionError(body)
        return 0

    def test_unsupported_teacher_stats_stops_before_any_remote_command(self):
        write(self.options.minase, b"#!/bin/sh\nexit 2\n")
        with self.assertRaises(RuntimeError):
            self.runner.run()
        self.assertEqual(self.remote.existence_checks, [])
        self.assertEqual(self.remote.machine_calls, 0)
        self.assertEqual(len(self.remote.balances), 4)
        self.assertEqual(self.remote.starts, [])
        self.assertFalse((self.remote_root / "commands.jsonl").exists())
        self.assertFalse((self.root / "cycles.jsonl").exists())

    def test_missing_remote_fixture_stops_before_cycle_work(self):
        self.remote.source_files.remove(FIXTURE)
        with self.assertRaises(RuntimeError) as error:
            self.runner.run()
        self.assertIn(FIXTURE, str(error.exception))
        self.assertIn(TRAINER_COMMIT, str(error.exception))
        self.assertEqual(self.remote.existence_checks, [FIXTURE])
        self.assertEqual(self.remote.machine_calls, 0)
        self.assertEqual(len(self.remote.balances), 4)
        self.assertEqual(self.remote.starts, [])
        self.assertFalse((self.remote_root / "commands.jsonl").exists())
        self.assertFalse((self.root / "cycles.jsonl").exists())

    def test_two_cycles_record_provenance_checks_cost_and_immutable_networks(self):
        def before_download(relative, target):
            if "/train/" in relative:
                cycle = relative.split("/")[1]
                self.assertFalse((self.root / "cycles" / cycle / "train/summary.json").exists())

        self.remote.on_download = before_download
        self.execute(replace(self.options, last_cycle=2, replay_buffer_positions=2000))
        rows = [json.loads(line) for line in (self.root / "cycles.jsonl").read_text().splitlines()]
        intervals = [json.loads(line) for line in (self.root / "sessions.jsonl").read_text().splitlines()]
        self.assertEqual([row["cycle"] for row in rows], [1, 2])
        self.assertEqual(self.remote.machine_calls, 1)
        self.assertEqual([row["generation"]["playthrough_percent"] for row in rows], [100, 20])
        self.assertEqual([row["generation"]["seed"] for row in rows], [1000000000, 2000000000])
        self.assertEqual([row["generation"]["network"] for row in rows],
                         ["cycles/cycle-0000/model.onnx", "cycles/cycle-0001/train/model.onnx"])
        self.assertEqual(rows[1]["resign_calibration"]["n"], 100)
        self.assertEqual(rows[0]["generation"]["games"], 10)
        self.assertEqual(rows[0]["generation"]["chunk_games"], 5)
        self.assertEqual([row["generation"]["target_positions"] for row in rows], [1000, 1000])
        self.assertEqual([row["generation"]["threads"] for row in rows], [6, 6])
        self.assertEqual([row["generation"]["threads_per_gpu"] for row in rows], [6, 6])
        self.assertEqual([row["generation"]["devices"] for row in rows], [1, 1])
        commands = [shlex.split(body) for stem, body, _, _ in self.remote.starts if stem.endswith("/selfplay")]
        self.assertEqual([command[command.index("--threads") + 1] for command in commands], ["6", "6"])
        self.assertEqual([command[command.index("--devices") + 1] for command in commands], ["1", "1"])
        self.assertEqual([command[command.index("--target-positions") + 1] for command in commands], ["1000", "1000"])
        self.assertEqual([command[command.index("--nn-model") + 1] for command in commands],
                         [row["generation"]["network"] for row in rows])
        self.assertEqual(len(rows[1]["training_files"]), 8)
        self.assertEqual(len(rows[1]["teacher_stats"]["files"]), 4)
        self.assertEqual(rows[0]["teacher_stats"]["game_length_plies"], {"p50": 100, "p90": 120, "p99": 140})
        self.assertEqual([row["generation_commit"] for row in rows], [GENERATION_COMMIT, GENERATION_COMMIT])
        self.assertEqual([row["trainer_commit"] for row in rows], [TRAINER_COMMIT, TRAINER_COMMIT])
        self.assertEqual([row["replay_buffer_cycles"] for row in rows], [[1], [1, 2]])
        self.assertEqual([row["replay_buffer_first_cycle"] for row in rows], [1, 1])
        self.assertEqual([row["replay_buffer_positions"] for row in rows], [2000, 2000])
        self.assertEqual(rows[0]["training_summary"]["provenance"]["trainer_commit"], TRAINER_COMMIT)
        self.assertEqual(rows[0]["selfplay_summary"]["win_reason_royal_capture"], 10)
        self.assertEqual(rows[0]["machine"], {
            "gpu": "NVIDIA L4", "gpu_memory_mib": 23034, "gpu_count": 1,
            "cpu": "Test CPU", "nproc": 12, "cpus_available": 12, "kind": "colab",
        })
        self.assertEqual([row["machine_kind"] for row in intervals], ["colab", "colab"])
        self.assertEqual([row["session"] for row in intervals], ["test-session", "test-session"])
        self.assertEqual([row["balance_difference"] for row in intervals], [2, 2])
        self.assertEqual(intervals[1]["started_at"], intervals[0]["ended_at"])
        self.assertEqual(intervals[1]["start_balance"], intervals[0]["end_balance"])
        for record in [*rows, *intervals]:
            self.assertEqual(record["lr"], 0.01)
        for row in rows:
            self.assertEqual(row["training_summary"], json.loads(
                (self.root / f"cycles/cycle-{row['cycle']:04d}/train/summary.json").read_text()))
            command = shlex.split(next(body for stem, body, _, _ in self.remote.starts
                                       if stem == f"cycles/cycle-{row['cycle']:04d}/train"))
            manifest = command[command.index("--data-list") + 1]
            data = json.loads((self.remote_root / manifest).read_text())
            self.assertNotIn("--data", command)
            latest = command[command.index("--latest-data") + 1:command.index("--out-dir")]
            self.assertEqual(data, row["training_files"])
            self.assertEqual(latest, row["teacher_stats"]["files"])
            self.assertEqual(latest, data[-len(latest):])
            self.assertEqual([(r["backend"], r["batch"], r["exit_code"]) for r in row["checks"]["results"]],
                             [("cpu", 1, 0), ("trt", 1, 0), ("cpu", 7, 0), ("trt", 7, 0), ("cpu", 256, 0), ("trt", 256, 0)])
            self.assertEqual(set(row["elapsed_seconds"]), {"generation", "training", "checking"})
            self.assertGreaterEqual(row["checks"]["elapsed_seconds"], 0)
        check_commands = [json.loads(line) for line in (self.remote_root / "commands.jsonl").read_text().splitlines()]
        self.assertEqual(len(check_commands), 26)
        first_commands = check_commands[:13]
        fixture = first_commands[0]
        self.assertEqual(fixture[fixture.index("fixture-eval"):], [
            "fixture-eval", "--checkpoint", "cycles/cycle-0001/train/checkpoint.pt", "--onnx", "cycles/cycle-0001/train/model.onnx",
            "--fixture", FIXTURE, "--out", "cycles/cycle-0001/check/pytorch.json",
        ])
        for command in first_commands[1:]:
            if "--candidate" in command:
                cpu = "/cpu-" in command[command.index("--candidate") + 1]
                self.assertEqual(command[command.index("--policy") + 1], "fp16-logits" if cpu else "probabilities")
                self.assertEqual(command[command.index("--policy-tolerance") + 1], "1e-4" if cpu else "1e-2")
                self.assertEqual(command[command.index("--value-tolerance") + 1], "1e-4" if cpu else "5e-2")
            else:
                self.assertEqual(command, ["--protocol", "usi", "--search", "mcts"])
        self.assert_usi_commands("/content/cache", (1, 2))
        for cycle, row in enumerate(rows, 1):
            target = self.root / f"nets/cycle-{cycle:04d}.onnx"
            self.assertEqual(target.read_bytes(), f"network {cycle}".encode())
            self.assertEqual(target.stat().st_mode & 0o222, 0)
            self.assertEqual(row["network_sha256"], digest(target))
            self.assertEqual(row["training_summary"]["network_sha256"], digest(target))
        start_count = len(self.remote.starts)
        self.execute(replace(self.options, last_cycle=2))
        self.assertEqual(len(self.remote.starts), start_count)

    def test_cycles_record_configured_replay_buffer_first_cycle(self):
        self.execute(replace(self.options, last_cycle=2))
        self.remote.balances = [96, 94]
        self.execute(replace(self.options, first_cycle=3, last_cycle=3, replay_buffer_first_cycle=2,
                             cycle1_reference_summary=None))
        rows = [json.loads(line) for line in (self.root / "cycles.jsonl").read_text().splitlines()]
        self.assertEqual([row["replay_buffer_first_cycle"] for row in rows], [1, 1, 2])
        self.assertEqual([row["replay_buffer_cycles"] for row in rows], [[1], [2], [3]])

    def test_branch_cycles_keep_outputs_records_seeds_and_networks_separate(self):
        self.execute(replace(self.options, last_cycle=2))
        unchanged = {path: path.read_bytes() for path in self.root.rglob("*")
                     if path.is_file() and path.name != "seed-ledger.jsonl"}
        self.remote.balances = [100, 98, 98, 96]
        before = len(self.remote.starts)
        options = replace(self.options, cycles_dir=Path("cycles-branch"), generation_seed_offset=300_000_000, branch_from=1,
                          first_cycle=2, last_cycle=3, cycle1_reference_summary=None,
                          replay_buffer_positions=2500)
        self.execute(options)
        rows = [json.loads(line) for line in (self.root / "cycles-branch.jsonl").read_text().splitlines()]
        intervals = [json.loads(line) for line in (self.root / "cycles-branch-sessions.jsonl").read_text().splitlines()]
        self.assertEqual([row["cycle"] for row in rows], [2, 3])
        self.assertEqual([row["generation"]["network"] for row in rows], [
            "cycles/cycle-0001/train/model.onnx", "cycles-branch/cycle-0002/train/model.onnx",
        ])
        self.assertEqual([row["generation"]["seed"] for row in rows], [2_300_000_000, 3_300_000_000])
        self.assertEqual([row["generation_seed_offset"] for row in rows], [300_000_000, 300_000_000])
        self.assertEqual([row["generation"]["start_position"] for row in rows], ["random-opening", "random-opening"])
        self.assertEqual([row["replay_buffer_cycles"] for row in rows], [[1, 2], [1, 2, 3]])
        for row in [*rows, *intervals]:
            self.assertEqual(row["cycles_dir"], "cycles-branch")
            self.assertEqual(row["base_cycles_dir"], "cycles")
            self.assertEqual(row["branch_from"], 1)
        expected_files = [f"{directory}/cycle-{cycle:04d}/selfplay/chunk-{chunk:05d}{suffix}.mnsd"
                          for cycle, directory in ((1, "cycles"), (2, "cycles-branch"), (3, "cycles-branch"))
                          for chunk in (0, 1) for suffix in ("", ".resigned")]
        self.assertEqual(rows[1]["training_files"], expected_files)
        self.assertEqual(rows[1]["teacher_stats"]["files"], expected_files[-4:])
        starts = self.remote.starts[before:]
        self.assertEqual([stem for stem, _, _, _ in starts], [
            f"cycles-branch/cycle-{cycle:04d}/{stage}" for cycle in (2, 3)
            for stage in ("selfplay", "train", "check")
        ])
        for cycle, previous in ((2, "cycles/cycle-0001"), (3, "cycles-branch/cycle-0002")):
            command = shlex.split(next(body for stem, body, _, _ in starts
                                      if stem == f"cycles-branch/cycle-{cycle:04d}/train"))
            self.assertEqual(command[command.index("--init") + 1], previous + "/train/checkpoint.pt")
            target = self.root / f"nets/cycles-branch/cycle-{cycle:04d}.onnx"
            self.assertEqual(target.read_bytes(), f"network {cycle}".encode())
            self.assertEqual(target.stat().st_mode & 0o222, 0)
            for stage in ("selfplay/summary.json", "train/summary.json", "check/cpu-b1.compare.json"):
                self.assertTrue((self.root / f"cycles-branch/cycle-{cycle:04d}" / stage).is_file())
        ledger = [json.loads(line) for line in (self.root / "seed-ledger.jsonl").read_text().splitlines()]
        self.assertEqual([(row["name"], row["seed"]) for row in ledger], [
            ("cycle-0001-gen", 1_000_000_000), ("cycle-0002-gen", 2_000_000_000),
            ("cycles-branch-0002-gen", 2_300_000_000), ("cycles-branch-0003-gen", 3_300_000_000),
        ])
        for path, content in unchanged.items():
            self.assertEqual(path.read_bytes(), content, str(path))
        count = len(self.remote.starts)
        self.execute(options)
        self.assertEqual(len(self.remote.starts), count)
        (self.root / "nets/cycles-branch/cycle-0002.onnx").unlink()
        with self.assertRaises(ValueError):
            self.execute(options)
        # 本線の同番号のネットや台帳では、別系列の未確定状態を補完しない。
        (self.root / "cycles-branch.jsonl").write_bytes(b"")
        with self.assertRaises(ValueError):
            self.execute(options)
        self.assertEqual(len(self.remote.starts), count)

    def test_branch_regeneration_uses_a_distinct_seed_and_ledger_name(self):
        self.regenerate = True
        seed_registration(self.root / "seed-ledger.jsonl", "cycle-0001-gen", 1_000_000_000, 10)
        seed_registration(self.root / "seed-ledger.jsonl", "cycle-0001-regen", 1_500_000_000, 3)
        seed_registration(self.root / "seed-ledger.jsonl", "cycle-0002-gen", 2_000_000_000, 10)
        self.execute(replace(self.options, cycles_dir=Path("cycles-branch"), generation_seed_offset=300_000_000))
        row = json.loads((self.root / "cycles-branch.jsonl").read_text())
        self.assertEqual(row["generation"]["network"], "cycles/cycle-0000/model.onnx")
        self.assertEqual(row["generation"]["seed"], 1_300_000_000)
        self.assertEqual(row["regeneration"]["seed"], 1_800_000_000)
        self.assertEqual(row["generation_seed_offset"], 300_000_000)
        self.assertTrue(all(name.startswith("cycles-branch/cycle-0001/") for name in row["training_files"]))
        self.assertTrue(any("/selfplay-regen/" in name for name in row["training_files"]))
        ledger = [json.loads(line) for line in (self.root / "seed-ledger.jsonl").read_text().splitlines()]
        self.assertEqual([(item["name"], item["seed"]) for item in ledger[-2:]], [
            ("cycles-branch-0001-gen", 1_300_000_000), ("cycles-branch-0001-regen", 1_800_000_000),
        ])

    def test_nested_relative_directory_is_used_by_all_stages(self):
        directory = "experiments/branch's-run"
        shutil.copytree(self.root / "cycles", self.root / "base cycles")
        self.execute(replace(self.options, cycles_dir=Path(directory), base_cycles_dir=Path("base cycles")))
        row = json.loads((self.root / f"{directory}.jsonl").read_text())
        self.assertEqual(row["cycle"], 1)
        self.assertEqual(row["generation"]["network"], "base cycles/cycle-0000/model.onnx")
        self.assertTrue((self.root / f"{directory}-sessions.jsonl").is_file())
        self.assertTrue((self.root / f"nets/{directory}/cycle-0001.onnx").is_file())
        self.assertEqual([stem for stem, _, _, _ in self.remote.starts], [
            f"{directory}/cycle-0001/{stage}" for stage in ("selfplay", "train", "check")
        ])

    def test_missing_previous_network_stops_before_generation(self):
        self.runner.run()
        ledger = (self.root / "seed-ledger.jsonl").read_bytes()
        count = len(self.remote.starts)
        (self.root / "cycles/cycle-0001/train/model.onnx").unlink()
        with self.assertRaises(FileNotFoundError):
            self.execute(replace(self.options, first_cycle=2, last_cycle=2,
                                 cycle1_reference_summary=None))
        self.assertEqual(len(self.remote.starts), count)
        self.assertEqual((self.root / "seed-ledger.jsonl").read_bytes(), ledger)

    def test_missing_initial_network_stops_before_generation(self):
        (self.root / "cycles/cycle-0000/model.onnx").unlink()
        with self.assertRaises(FileNotFoundError):
            self.runner.run()
        self.assertEqual(self.remote.starts, [])
        self.assertEqual((self.root / "seed-ledger.jsonl").read_bytes(), b"")

    def test_missing_remote_network_does_not_commit_training_summary(self):
        def complete(stem, body):
            code = self.complete_job(stem, body)
            if stem.endswith("/train"):
                (self.remote_root / stem / "model.onnx").unlink()
            return code

        self.remote.on_start = complete
        with self.assertRaises(FileNotFoundError):
            self.runner.run()
        self.assertFalse((self.root / "cycles/cycle-0001/train/summary.json").exists())
        self.assertFalse((self.root / "cycles.jsonl").exists())
        self.assertEqual([Path(item[0]).name for item in self.remote.starts], ["selfplay", "train"])

    def test_mismatched_network_hash_does_not_commit_training_summary(self):
        def complete(stem, body):
            code = self.complete_job(stem, body)
            if stem.endswith("/train"):
                path = self.remote_root / stem / "summary.json"
                summary = json.loads(path.read_text())
                summary["network_sha256"] = "f" * 64
                write_json(path, summary)
            return code

        self.remote.on_start = complete
        with self.assertRaises(ValueError):
            self.runner.run()
        self.assertFalse((self.root / "cycles/cycle-0001/train/summary.json").exists())
        self.assertFalse((self.root / "cycles.jsonl").exists())

    def test_false_resignations_generate_replacements_and_append_them_to_replay_buffer(self):
        self.regenerate = True
        self.runner.run()
        row = json.loads((self.root / "cycles.jsonl").read_text())
        self.assertEqual(row["regeneration"]["seed"], 1500000000)
        self.assertEqual(row["regeneration"]["games"], 3)
        self.assertEqual(row["regeneration"]["chunk_games"], 2)
        self.assertEqual(row["regeneration"]["playthrough_percent"], 100)
        self.assertEqual(row["regeneration"]["target_positions"], 300)
        self.assertEqual(row["selfplay_regen_summary"]["target_positions"], 300)
        self.assertEqual(row["selfplay_regen_summary"]["games"], 3)
        generation_commands = [shlex.split(body) for stem, body, _, _ in self.remote.starts
                               if Path(stem).name in ("selfplay", "selfplay-regen")]
        self.assertEqual([command[command.index("--target-positions") + 1] for command in generation_commands],
                         ["1000", "300"])
        self.assertEqual([command[command.index("--games") + 1] for command in generation_commands], ["10", "3"])
        self.assertEqual(row["training_files"], [
            "cycles/cycle-0001/selfplay/chunk-00000.mnsd", "cycles/cycle-0001/selfplay/chunk-00001.mnsd",
            "cycles/cycle-0001/selfplay-regen/chunk-00000.mnsd", "cycles/cycle-0001/selfplay-regen/chunk-00000.resigned.mnsd",
            "cycles/cycle-0001/selfplay-regen/chunk-00001.mnsd", "cycles/cycle-0001/selfplay-regen/chunk-00001.resigned.mnsd",
        ])
        command = shlex.split(next(body for stem, body, _, _ in self.remote.starts if stem.endswith("/train")))
        self.assertEqual(command[command.index("--latest-data") + 1:command.index("--out-dir")], row["training_files"])
        ledger = [json.loads(line) for line in (self.root / "seed-ledger.jsonl").read_text().splitlines()]
        self.assertEqual([entry["name"] for entry in ledger], ["cycle-0001-gen", "cycle-0001-regen"])

    def test_regeneration_rejects_missing_or_nonpositive_integer_resigned_positions(self):
        self.regenerate = True
        out = "cycles/cycle-0001/selfplay"
        summary, _ = self.runner.generation(1, out, "cycles/cycle-0000/model.onnx",
                                            1000000000, 10, 5, 100, 1000)
        missing = object()
        invalid_values = (0, -1, 300.0, True, "300", None, missing)
        self.remote.balances = [100.0] * len(invalid_values)
        for invalid in invalid_values:
            with self.subTest(invalid=invalid):
                changed = {**summary, "resigned_positions": invalid}
                if invalid is missing:
                    del changed["resigned_positions"]
                write_json(self.root / out / "summary.json", changed)
                with self.assertRaises(ValueError):
                    self.runner.run()
                self.assertEqual([Path(stem).name for stem, _, _, _ in self.remote.starts], ["selfplay"])
        ledger = [json.loads(line) for line in (self.root / "seed-ledger.jsonl").read_text().splitlines()]
        self.assertEqual([entry["name"] for entry in ledger], ["cycle-0001-gen"])

    def test_comparison_failure_receives_all_outputs_and_blocks_next_cycle(self):
        self.compare_failure = True
        with self.assertRaises(RuntimeError):
            self.execute(replace(self.options, last_cycle=2))
        check = self.root / "cycles/cycle-0001/check"
        self.assertEqual(len(list(check.glob("*.compare.json"))), 6)
        self.assertEqual(len(list(check.glob("*.compare.exit"))), 6)
        self.assertEqual((check / "cpu-b7.compare.exit").read_text().strip(), "1")
        self.assertTrue((check / "trt-b256.json").is_file())
        self.assertTrue((check / "elapsed-seconds").is_file())
        self.assertFalse((self.root / "nets/cycle-0001.onnx").exists())
        self.assertFalse((self.root / "cycles.jsonl").exists())
        self.assertEqual([Path(item[0]).name for item in self.remote.starts], ["selfplay", "train", "check"])

    def test_balance_increase_is_recorded_without_assuming_credits_or_purchase(self):
        self.remote.balances = [100.0, 110.0]
        self.runner.run()
        interval = json.loads((self.root / "sessions.jsonl").read_text())
        self.assertEqual(interval["balance_difference"], -10)
        self.assertTrue(interval["balance_increased"])
        self.assertNotIn("purchased", interval)
        self.assertNotIn("granted", interval)

    def test_live_generation_is_monitored_without_relaunch(self):
        self.runner.run()
        # 生成器が完了ファイルを書いた直後、実行器にはまだ未受信という状態を作る。
        shutil.rmtree(self.root / "cycles/cycle-0001")
        stem = "cycles/cycle-0001/selfplay"
        self.remote.states[stem] = ["running", "running", 0]
        before = len(self.remote.starts)
        with patch("minase_zero.runner.time.sleep") as sleep:
            result, _ = self.runner.generation(1, stem, "cycles/cycle-0000/model.onnx", 1000000000, 10, 5, 100, 1000)
        self.assertEqual(result["games"], 10)
        self.assertEqual(len(self.remote.starts), before)
        sleep.assert_called_once_with(60)
        downloaded = [name for name in self.remote.downloads if name.startswith(stem + "/")]
        self.assertLess(downloaded.index(stem + "/chunk-00000.resigned.mnsd"), downloaded.index(stem + "/chunk-00000.json"))

    def test_failed_generation_keeps_chunks_and_stops_before_training(self):
        complete = self.remote.on_start

        def fail_after_output(stem, body):
            complete(stem, body)
            return 3

        self.remote.on_start = fail_after_output
        with self.assertRaises(RuntimeError):
            self.runner.run()
        self.assertTrue((self.root / "cycles/cycle-0001/selfplay/chunk-00000.json").is_file())
        self.assertFalse((self.root / "cycles/cycle-0001/selfplay/summary.json").exists())
        self.assertEqual(len(self.remote.starts), 1)

    def test_checkpoint_is_polled_while_attaching_to_running_training(self):
        self.runner.run()
        out = "cycles/cycle-0001/train"
        shutil.rmtree(self.root / out)
        (self.root / (out + ".log")).unlink()
        self.remote.states[out] = ["running", "running", 0]
        before = len(self.remote.starts)
        _, files, latest = replay_buffer(self.options, 1, 1000)
        with patch("minase_zero.runner.time.sleep") as sleep:
            self.runner.training(1, files, latest)
        self.assertEqual((self.root / out / "checkpoint.pt").read_bytes(), b"checkpoint 1")
        self.assertEqual(len(self.remote.starts), before)
        sleep.assert_called_once_with(60)

    def test_new_machine_resumes_nas_checkpoint_without_repeating_completed_generation(self):
        complete = self.remote.on_start

        def interrupt_training(stem, body):
            if stem.endswith("/train"):
                write(self.remote_root / stem / "checkpoint.pt", b"partial checkpoint")
                return 9
            return complete(stem, body)

        self.remote.on_start = interrupt_training
        with self.assertRaises(RuntimeError):
            self.runner.run()
        ledger = (self.root / "seed-ledger.jsonl").read_bytes()
        self.assertEqual((self.root / "cycles/cycle-0001/train/checkpoint.pt").read_bytes(), b"partial checkpoint")
        shutil.rmtree(self.remote_root)
        self.remote_root.mkdir()
        self.remote = FakeRemote(self.remote_root)
        self.remote.on_start = self.complete_job
        self.clock.now.return_value = NOW + timedelta(hours=2)
        self.execute(self.options)
        self.assertEqual([Path(item[0]).name for item in self.remote.starts], ["train", "check"])
        command = shlex.split(self.remote.starts[0][1])
        self.assertEqual(command[command.index("train"):], ["train", "--resume", "cycles/cycle-0001/train", "--device", "cuda", "--replay-threads", "12"])
        self.assertIn("cycles/cycle-0001/train/checkpoint.pt", self.remote.uploads)
        self.assertEqual((self.root / "seed-ledger.jsonl").read_bytes(), ledger)
        row = json.loads((self.root / "cycles.jsonl").read_text())
        self.assertEqual(row["elapsed_seconds"]["training"], 7200)

    def test_completed_generation_records_its_commit_and_accepts_a_new_trainer(self):
        complete = self.remote.on_start
        self.remote.on_start = lambda stem, body: 9 if stem.endswith("/train") else complete(stem, body)
        with self.assertRaises(RuntimeError):
            self.runner.run()
        config = (self.root / "cycles/cycle-0001/selfplay/config.json").read_bytes()
        shutil.rmtree(self.remote_root)
        self.remote_root.mkdir()
        self.remote = FakeRemote(self.remote_root)
        self.remote.on_start = self.complete_job
        self.execute(replace(self.options, generation_commit="c" * 40, trainer_commit="e" * 40))
        self.assertEqual([Path(stem).name for stem, _, _, _ in self.remote.starts], ["train", "check"])
        self.assertEqual((self.root / "cycles/cycle-0001/selfplay/config.json").read_bytes(), config)
        row = json.loads((self.root / "cycles.jsonl").read_text())
        self.assertEqual(row["generation_commit"], GENERATION_COMMIT)
        self.assertEqual(row["generation"]["generation_commit"], GENERATION_COMMIT)
        self.assertEqual(row["trainer_commit"], "e" * 40)
        command = shlex.split(self.remote.starts[0][1])
        self.assertEqual(command[command.index("--trainer-commit") + 1], "e" * 40)
        batches = [files for files, _ in self.remote.upload_batches if any(name.endswith(".mnsd") for name in files)]
        self.assertEqual(batches, [[*row["training_files"], "cycles/cycle-0001/train-data.json"]])

    def test_new_generation_checks_its_commit_before_accepting_the_summary(self):
        complete = self.remote.on_start

        def different_commit(stem, body):
            code = complete(stem, body)
            path = self.remote_root / stem / "config.json"
            config = json.loads(path.read_text())
            config["generation_commit"] = "c" * 40
            write_json(path, config)
            return code

        self.remote.on_start = different_commit
        with self.assertRaises(ValueError):
            self.runner.run()
        self.assertFalse((self.root / "cycles/cycle-0001/selfplay/summary.json").exists())
        self.assertEqual([Path(stem).name for stem, _, _, _ in self.remote.starts], ["selfplay"])

    def test_new_machine_restores_generation_config_and_committed_chunks(self):
        complete = self.remote.on_start

        def interrupt_generation(stem, body):
            complete(stem, body)
            return 9

        self.remote.on_start = interrupt_generation
        with self.assertRaises(RuntimeError):
            self.runner.run()
        ledger = (self.root / "seed-ledger.jsonl").read_bytes()
        shutil.rmtree(self.remote_root)
        self.remote_root.mkdir()
        self.remote = FakeRemote(self.remote_root)
        self.remote.on_start = self.complete_job
        self.execute(self.options)
        out = "cycles/cycle-0001/selfplay"
        restored = [name for name in self.remote.uploads if name.startswith(out + "/")]
        self.assertEqual(restored, [f"{out}/{name}" for name in (
            "config.json", "chunk-00000.mnsd", "chunk-00000.resigned.mnsd", "chunk-00000.json",
            "chunk-00001.mnsd", "chunk-00001.resigned.mnsd", "chunk-00001.json",
        )])
        self.assertEqual((self.root / "seed-ledger.jsonl").read_bytes(), ledger)
        self.assertTrue((self.root / "nets/cycle-0001.onnx").exists())

    def test_elapsed_seconds_start_at_launch_after_input_uploads(self):
        upload = self.remote.upload
        complete = self.remote.on_start

        def timed_upload(root, files, timeout):
            upload(root, files, timeout)
            self.clock.now.return_value += timedelta(seconds=10)

        def timed_job(stem, body):
            code = complete(stem, body)
            self.clock.now.return_value += timedelta(seconds=30)
            return code

        self.remote.upload = timed_upload
        self.remote.on_start = timed_job
        self.runner.run()
        row = json.loads((self.root / "cycles.jsonl").read_text())
        self.assertEqual(row["elapsed_seconds"], {"generation": 30, "training": 30, "checking": 30})
        self.assertEqual(row["interval"]["started_at"], NOW.isoformat())
        self.assertGreater(datetime.fromisoformat(row["interval"]["ended_at"]), NOW + timedelta(seconds=90))


class ArgumentTests(RunnerCase):
    def test_vast_launcher_preflight_validates_without_constructing_runner_or_remote(self):
        path = Path(__file__).resolve().parents[4] / "docs/measurements/alphazero-multi-gpu/launch-vast.py"
        script = runpy.run_path(str(path))["VALIDATE_RUNNER"]
        with (patch("sys.argv", ["-c", *self.vast_arguments()]),
              patch("minase_zero.runner.Options", wraps=Options) as options,
              patch("minase_zero.runner.Runner") as runner,
              patch("minase_zero.runner.Remote") as remote,
              patch("minase_zero.__main__.Runner") as cli_runner,
              patch("minase_zero.__main__.Remote") as cli_remote):
            exec(compile(script, "<runner-preflight>", "exec"), {})
        options.assert_called_once()
        self.assertEqual(options.call_args.kwargs["machine"], "vast")
        for constructor in (runner, remote, cli_runner, cli_remote):
            constructor.assert_not_called()

    def test_vast_launcher_preflight_rejects_parser_errors_and_invalid_options(self):
        path = Path(__file__).resolve().parents[4] / "docs/measurements/alphazero-multi-gpu/launch-vast.py"
        script = runpy.run_path(str(path))["VALIDATE_RUNNER"]
        args = self.vast_arguments()
        invalid_options = args.copy()
        invalid_options[invalid_options.index("--batch-size") + 1] = "0"
        for invalid in (args + ["--unknown"], invalid_options, args + ["--help"]):
            with (self.subTest(arguments=invalid), patch("sys.argv", ["-c", *invalid]),
                  redirect_stdout(io.StringIO()), self.assertRaises(ValueError)):
                exec(compile(script, "<runner-preflight>", "exec"), {})

    def vast_arguments(self):
        args = self.arguments()
        args[args.index("--machine") + 1] = "vast"
        for flag in ("--session", "--cycle-cu"):
            index = args.index(flag)
            del args[index:index + 2]
        return [*args, "--vast-instance", "12345"]

    def test_vast_arguments_require_instance_deadline_duration_and_threads_per_gpu(self):
        args = self.vast_arguments()
        parsed = vars(_parse(args))
        parsed.pop("command")
        options = Options(**parsed)
        self.assertEqual(options.machine, "vast")
        self.assertEqual(options.vast_instance, 12345)
        self.assertIsNone(options.session)
        self.assertIsNone(options.cycle_cu)
        for flag in ("--machine", "--vast-instance", "--deadline", "--cycle-minutes", "--threads-per-gpu"):
            index = args.index(flag)
            with self.subTest(missing=flag), self.assertRaises(ValueError):
                parsed = vars(_parse(args[:index] + args[index + 2:]))
                parsed.pop("command")
                Options(**parsed)

    def test_invalid_machine_argument_combinations_fail_before_external_commands(self):
        colab = self.arguments()
        vast = self.vast_arguments()
        cases = [colab + ["--vast-instance", "12345"], vast + ["--cycle-cu", "2"],
                 vast + ["--session", "test"], colab + ["--threads", "4"]]
        for base, flag in ((vast, "--vast-instance"), (colab, "--cycle-cu"), (colab, "--session")):
            index = base.index(flag)
            cases.append(base[:index] + base[index + 2:])
        for base, flag, value in ((vast, "--vast-instance", "0"), (vast, "--vast-instance", "-1"),
                                  (vast, "--vast-instance", "1.5"), (colab, "--threads-per-gpu", "0"),
                                  (colab, "--threads-per-gpu", "-1"), (colab, "--threads-per-gpu", "1.5"),
                                  (colab, "--machine", "unknown")):
            changed = list(base)
            changed[changed.index(flag) + 1] = value
            cases.append(changed)
        for args in cases:
            with self.subTest(args=args), patch("sys.argv", ["minase_zero", *args]), \
                    patch("sys.stderr", new_callable=io.StringIO), \
                    patch("minase_zero.runner.subprocess.run") as run, \
                    patch("minase_zero.runner.load_vast_common") as common:
                self.assertEqual(main(), 1)
                run.assert_not_called()
                common.assert_not_called()

    def test_start_position_and_seed_offset_reject_invalid_options_and_cli_values(self):
        for offset in (-1, 500_000_000, 1_000_000_000, 1.5, True, "0"):
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                replace(self.options, generation_seed_offset=offset)
        for offset in (0, 300_000_000, 499_999_999):
            self.assertEqual(replace(self.options, generation_seed_offset=offset).generation_seed_offset, offset)
        with self.assertRaises(ValueError):
            replace(self.options, start_position="unknown")
        for option, value in (("--start-position", "unknown"), ("--generation-seed-offset", "-1"),
                              ("--generation-seed-offset", "500000000"), ("--generation-seed-offset", "1.5")):
            arguments = self.arguments()
            arguments[arguments.index(option) + 1] = value
            with self.subTest(option=option, value=value), self.assertRaises(ValueError):
                parsed = vars(_parse(arguments))
                parsed.pop("command")
                Options(**parsed)

    def arguments(self):
        return ["run-cycles", "--root", str(self.root), "--machine", "colab", "--session", "l4-one",
                "--cycles-dir", "cycles", "--base-cycles-dir", "cycles", "--branch-from", "0",
                "--start-position", "random-opening", "--generation-seed-offset", "0",
                "--generation-commit", GENERATION_COMMIT, "--trainer-commit", TRAINER_COMMIT,
                "--first-cycle", "1", "--last-cycle", "3", "--deadline", "2026-10-29T12:00:00+09:00",
                "--cycle-minutes", "40", "--cycle-cu", "2", "--target-positions", "1000000",
                "--chunks", "20", "--simulations", "32", "--threads-per-gpu", "6", "--games-per-thread", "4",
                "--replay-buffer-first-cycle", "1",
                "--replay-buffer-positions", "34625240", "--batch-size", "256", "--lr", "0.01",
                "--momentum", "0.9",
                "--weight-decay", "0.0001", "--clip-grad-norm", "10", "--value-lambda", "0.333",
                "--precision", "bf16", "--checkpoint-steps", "200", "--train-seed-base", "123",
                "--minase", str(self.options.minase), "--poll-seconds", "60",
                "--cycle1-reference-summary", str(self.reference)]

    def test_colab_arguments_other_than_reference_are_required(self):
        args = self.arguments()
        parsed = vars(_parse(args))
        self.assertEqual(parsed.pop("command"), "run-cycles")
        self.assertEqual(parsed["replay_buffer_first_cycle"], 1)
        self.assertEqual(Options(**parsed).deadline, datetime(2026, 10, 29, 3, 0, tzinfo=UTC))
        for index in range(1, len(args) - 2, 2):
            with self.subTest(missing=args[index]), self.assertRaises(ValueError):
                parsed = vars(_parse(args[:index] + args[index + 2:]))
                parsed.pop("command")
                Options(**parsed)

    def test_reference_is_required_exactly_for_first_cycle_one(self):
        with self.assertRaises(ValueError):
            replace(self.options, cycle1_reference_summary=None)
        with self.assertRaises(ValueError):
            replace(self.options, first_cycle=2, last_cycle=3)
        result = replace(self.options, first_cycle=2, last_cycle=3, cycle1_reference_summary=None)
        self.assertIsNone(result.cycle1_reference_summary)

    def test_replay_buffer_first_cycle_bounds_apply_to_mainline_and_branches(self):
        for directory, branch_from in (("cycles", 0), ("cycles-branch", 2)):
            options = replace(self.options, cycles_dir=Path(directory), branch_from=branch_from,
                              first_cycle=4, last_cycle=4, cycle1_reference_summary=None)
            for first in (1, 2, 3, 4):
                with self.subTest(directory=directory, first=first):
                    self.assertEqual(replace(options, replay_buffer_first_cycle=first).replay_buffer_first_cycle, first)
            for first in (-1, 0, 5, True, 1.5):
                with self.subTest(directory=directory, first=first), self.assertRaises(ValueError):
                    replace(options, replay_buffer_first_cycle=first)

    def test_invalid_inputs_fail_before_remote_work(self):
        for changes in ({"deadline": NOW.replace(tzinfo=None)}, {"generation_commit": "abcd"},
                        {"trainer_commit": "abcd"}, {"session": "x; echo secret"},
                        {"poll_seconds": 0}, {"replay_buffer_positions": 0}, {"cycle_cu": float("nan")}, {"cycle_minutes": -1},
                        {"weight_decay": -1}, {"value_lambda": 2}, {"train_seed_base": 2**64 - 1},
                        {"cycles_dir": Path("/absolute")},
                        {"cycles_dir": Path("cycles branch")},
                        {"base_cycles_dir": Path("../outside")}, {"cycles_dir": Path(".")},
                        {"branch_from": -1}, {"branch_from": 1}, {"branch_from": True}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                replace(self.options, **changes)


if __name__ == "__main__":
    unittest.main()
