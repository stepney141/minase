"""実行器の契約を指示書brief-rl-runner.mdとdocs/plans/alphazero-rl.mdから固定する。

対局数、較正、窓の順序、開始の境界は独立した具体例で検証する。外部との境界には
一時ディレクトリをColab機に見立てたRemoteを使い、実ファイルで再開と転送を確かめる。
照合のシェルは外部コマンドだけを小さな実行ファイルへ置き換えて実際に走らせる。
台帳の欠落とチェックポイントの置き換え競合はbrief-rl-runner-fix.mdの契約に従う。
局面数による窓、生成と学習のコミットの分離、一括転送はbrief-runner-window.mdに従う。
"""

from dataclasses import replace
from datetime import UTC, datetime, timedelta
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from minase_zero.__main__ import _parse
from minase_zero.runner import (
    Options, Remote, Runner, append_cycle, can_start, chunk_files, game_counts, parse_balance,
    publish_network, receive, resignation, seed_registration, send, summaries, training_window,
)


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


class FakeRemote:
    """転送と起動だけを置き換え、NAS側の確定処理は実物で検証する。"""

    def __init__(self, root: Path):
        self.root = root
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
        self.corrupt = False
        self.on_start = None
        self.on_download = None

    def exists(self, relative):
        return (self.root / relative).is_file()

    def files(self, relative):
        path = self.root / relative
        return sorted(p.name for p in path.iterdir() if p.is_file()) if path.exists() else []

    def sha256(self, relative):
        self.hashes.append(relative)
        return digest(self.root / relative)

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

    def download(self, relative, target):
        self.downloads.append(relative)
        if self.on_download is not None:
            self.on_download(relative, target)
        write(target, b"corrupt" if self.corrupt else (self.root / relative).read_bytes())

    def usage(self):
        return self.balances.pop(0)

    def machine(self):
        self.machine_calls += 1
        return {"gpu": "NVIDIA L4", "cpu": "Test CPU", "nproc": 12}

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
            root=self.root, session="test-session", generation_commit=GENERATION_COMMIT,
            trainer_commit=TRAINER_COMMIT, first_cycle=1, last_cycle=1,
            deadline=NOW + timedelta(hours=8), cycle_minutes=40, cycle_cu=2,
            target_positions=1000, chunks=2, simulations=32, threads=6, games_per_thread=4,
            window_positions=1000, batch_size=256, lr=0.01, momentum=0.9, weight_decay=0.0001,
            clip_grad_norm=10, value_lambda=0.333, precision="bf16", checkpoint_steps=200,
            selfplay_weights="raw", ema_period_samples=0, ema_scale=0,
            train_seed_base=123, teacher_stats=self.directory / "teacher_stats", poll_seconds=60,
            cycle1_reference_summary=self.reference,
        )
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
        self.assertEqual(game_counts(summaries(self.root, 2), 1000000, 20), (8334, 417))
        self.summary(2, "selfplay-regen", {"games": 20, "training_positions": 3000})
        self.assertEqual(game_counts(summaries(self.root, 2), 1000000, 20), (8000, 400))

    def test_zero_or_negative_reference_is_rejected(self):
        for games, positions in ((0, 100), (10, 0), (-1, 100)):
            with self.subTest(games=games, positions=positions):
                write_json(self.reference, {"games": games, "training_positions": positions})
                with self.assertRaises(ValueError):
                    game_counts([self.reference], 1000, 20)

    def test_resignation_starts_disabled_and_uses_previous_calibration(self):
        self.assertEqual(resignation(self.root, 1), (100, {
            "n": 0, "false_resignations": 0, "upper_bound": None, "enable_resignation": False,
        }))
        for n, errors, expected in ((100, 1, 20), (100, 2, 100), (99, 0, 100), (0, 0, 100)):
            with self.subTest(n=n, errors=errors):
                self.summary(1, "selfplay", {"resign_triggered_playthrough_games": n, "false_resignations": errors})
                percent, calibration = resignation(self.root, 2)
                self.assertEqual(percent, expected)
                self.assertEqual(calibration["n"], n)
                self.assertEqual(calibration["false_resignations"], errors)

    def test_resignation_adds_regenerated_samples(self):
        self.summary(2, "selfplay", {"resign_triggered_playthrough_games": 50, "false_resignations": 1})
        self.summary(2, "selfplay-regen", {"resign_triggered_playthrough_games": 50, "false_resignations": 0})
        percent, calibration = resignation(self.root, 3)
        self.assertEqual(percent, 20)
        self.assertEqual(calibration["n"], 100)
        self.assertAlmostEqual(calibration["upper_bound"], 0.0466, places=4)

    def test_training_window_uses_position_boundary_and_keeps_summary_order(self):
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
                self.assertEqual(training_window(self.root, 3, positions), (cycles, files, third))
        self.assertEqual(training_window(self.root, 1, 1000), ([1], first, first))
        self.assertEqual(training_window(self.root, 2, 100), ([2], second, second))
        self.assertEqual(training_window(self.root, 2, 200), ([2], second, second))

    def test_window_rejects_paths_outside_generation_output(self):
        self.summary(1, "selfplay", {"training_positions": 100, "training_files": ["../../outside.mnsd"]})
        with self.assertRaises(ValueError):
            training_window(self.root, 1, 1)

    def test_window_rejects_invalid_position_counts(self):
        for positions in (-1, 1.5, True):
            with self.subTest(positions=positions):
                self.summary(1, "selfplay", {"training_positions": positions, "training_files": ["chunk-00000.mnsd"]})
                with self.assertRaises(ValueError):
                    training_window(self.root, 1, 100)

    def test_window_does_not_read_cycles_before_the_position_boundary(self):
        self.summary(3, "selfplay", {"training_positions": 100, "training_files": ["chunk-00000.mnsd"]})
        files = ["cycles/cycle-0003/selfplay/chunk-00000.mnsd"]
        self.assertEqual(training_window(self.root, 3, 100), ([3], files, files))

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
        for weights, suffix in (("raw", ""), ("ema", "-ema")):
            with self.subTest(weights=weights):
                source = self.root / f"cycles/cycle-0001/train/model{suffix}.onnx"
                content = f"trained {weights} network".encode()
                write(source, content)
                result = publish_network(self.root, 1, weights)
                target = self.root / f"nets/cycle-0001{suffix}.onnx"
                self.assertEqual(result, digest(source))
                self.assertEqual(target.read_bytes(), content)
                self.assertEqual(target.stat().st_mode & 0o222, 0)
                self.assertEqual(publish_network(self.root, 1, weights), result)
                write(source, b"different network")
                with self.assertRaises(ValueError):
                    publish_network(self.root, 1, weights)
                self.assertEqual(target.read_bytes(), content)


class ResumeTests(RunnerCase):
    def test_incomplete_generation_rejects_a_different_saved_commit_before_launch(self):
        out = "cycles/cycle-0001/selfplay"
        for location in (self.root, self.remote_root):
            with self.subTest(location=location):
                write_json(location / out / "config.json", {
                    "generation_commit": "c" * 40,
                    "network_sha256": digest(self.root / "cycles/cycle-0000/model.onnx"),
                    "nn_backend": "tensorrt", "seed": 1000000000, "games": 10, "chunk_games": 5,
                    "simulations": 32, "playthrough_percent": 100,
                })
                with self.assertRaises(ValueError):
                    self.runner.generation(1, out, "cycles/cycle-0000/model.onnx", 1000000000, 10, 5, 100)
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
        self.assertEqual(command[command.index("train"):], ["train", "--resume", "cycles/cycle-0001/train", "--device", "cuda"])
        self.assertEqual(self.remote.uploads, [])

    def test_nas_checkpoint_is_uploaded_for_resume(self):
        relative = "cycles/cycle-0001/train/checkpoint.pt"
        write(self.root / relative, b"saved checkpoint")
        files = ["cycles/cycle-0001/selfplay/chunk-00000.mnsd"]
        command = self.runner.training_command(1, files, files)
        self.assertEqual(command[command.index("train"):], ["train", "--resume", "cycles/cycle-0001/train", "--device", "cuda"])
        self.assertEqual(self.remote.uploads, [relative])
        self.assertEqual((self.remote_root / relative).read_bytes(), b"saved checkpoint")

    def test_new_training_passes_window_and_every_explicit_setting(self):
        latest = ["cycles/cycle-0003/selfplay/chunk-00000.mnsd",
                  "cycles/cycle-0003/selfplay-regen/chunk-00000.mnsd"]
        files = ["cycles/cycle-0002/selfplay/chunk-00000.mnsd", *latest]
        command = self.runner.training_command(3, files, latest)
        self.assertEqual(command, [
            "uv", "run", "--frozen", "--project", "/content/src/tools/train/zero", "python", "-m", "minase_zero",
            "train", "--init", "cycles/cycle-0002/train/checkpoint.pt", "--data", *files, "--latest-data", *latest,
            "--out-dir", "cycles/cycle-0003/train", "--cycle", "3", "--seed", "126",
            "--batch-size", "256", "--lr", "0.01", "--momentum", "0.9", "--weight-decay", "0.0001",
            "--clip-grad-norm", "10", "--value-lambda", "0.333", "--precision", "bf16",
            "--checkpoint-steps", "200", "--ema-period-samples", "0", "--ema-scale", "0",
            "--device", "cuda", "--trainer-commit", TRAINER_COMMIT,
        ])
        first_files = ["cycles/cycle-0001/selfplay/chunk-00000.mnsd"]
        first = self.runner.training_command(1, first_files, first_files)
        self.assertEqual(first[first.index("--init") + 1], "cycles/cycle-0000/checkpoint.pt")

    def test_completed_cycle_is_skipped_without_remote_operations(self):
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

    def test_recorded_ema_cycle_requires_both_immutable_networks(self):
        append_cycle(self.root / "cycles.jsonl", {"cycle": 1, "ema_network_sha256": "a" * 64})
        write(self.root / "nets/cycle-0001.onnx", b"published raw")
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
    def test_batch_transfer_uses_one_files_from_rsync_and_one_checksum_command(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "NAS with spaces"
            destination = Path(directory) / "colab"
            destination.mkdir()
            files = ["old/chunk-00000.mnsd", "new/chunk-00001.mnsd", "new/chunk-00000.mnsd"]
            for name, size in zip(files, (1000000, 2000000, 500000)):
                write(root / name, b"x" * size)
            write(destination / files[0], (root / files[0]).read_bytes())
            remote = Remote("local")
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
            self.assertEqual(shlex.split(checksum_script), ["sha256sum", "--zero", "--", *files])
            self.assertEqual(commands[2][1], 122.5)
            self.assertEqual(len(manifests), 1)
            self.assertEqual(manifests[0][1], b"new/chunk-00001.mnsd\0new/chunk-00000.mnsd\0")
            self.assertFalse(manifests[0][0].exists())
            for name in files:
                self.assertEqual((destination / name).read_bytes(), (root / name).read_bytes())

    def test_batch_checksum_rejects_missing_duplicate_or_malformed_results(self):
        remote = Remote("local")
        first = "a" * 64 + "  first.mnsd\0"
        second = "b" * 64 + "  second.mnsd\0"
        for output in (first, first + first, first + second[:-1], first + "invalid\0",
                       first + second + "c" * 64 + "  unexpected.mnsd\0"):
            with self.subTest(output=output), patch.object(remote, "shell", return_value=output):
                with self.assertRaises(ValueError):
                    remote.sha256_many(["first.mnsd", "second.mnsd"], 120)

    def test_failed_batch_transfer_removes_its_file_list(self):
        remote = Remote("local")
        manifests = []

        def fail(argv, **kwargs):
            manifests.append(Path(argv[argv.index("--files-from") + 1]))
            self.assertTrue(manifests[0].is_file())
            return subprocess.CompletedProcess(argv, 23, "", "transfer failed")

        with patch("minase_zero.runner.subprocess.run", side_effect=fail), self.assertRaises(RuntimeError):
            remote.upload(Path("/NAS"), ["file.mnsd"], 121)
        self.assertFalse(manifests[0].exists())

    def test_remote_copy_is_independent_of_original_and_can_be_removed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            original = "train output/checkpoint.pt"
            snapshot = "train output/.checkpoint.pt.snapshot"
            write(root / original, b"before replacement")
            remote = Remote("local")

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
            remote = Remote("local")

            def execute(argv, timeout):
                command = shlex.split(argv[-1])
                command[-1] = command[-1].replace("/content/az", str(root / "absent"))
                return subprocess.run(command, cwd=root, capture_output=True, text=True, timeout=timeout)

            with patch.object(remote, "_call", side_effect=execute):
                result = remote.shell("echo first\nprintf wrong > marker", 120)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse((root / "marker").exists())

    def test_ssh_and_rsync_share_control_connection_and_use_timeout(self):
        remote = Remote("l4-one")
        with patch("minase_zero.runner.subprocess.run", return_value=subprocess.CompletedProcess([], 0, "yes\n", "")) as run:
            self.assertTrue(remote.exists("cycles/cycle-0001/train/checkpoint.pt"))
            remote.download("cycles/cycle-0001/train/checkpoint.pt", Path("/tmp/local-checkpoint"))
        ssh_call, rsync_call = run.call_args_list
        ssh = ssh_call.args[0]
        self.assertIn("ProxyCommand=colab ssh --proxy-mode -s l4-one", ssh)
        self.assertIn("ControlMaster=auto", ssh)
        self.assertIn("ControlPath=/tmp/colab-cm-l4-one", ssh)
        self.assertIn("ControlPersist=900", ssh)
        rsync = rsync_call.args[0]
        self.assertEqual(shlex.split(rsync[rsync.index("-e") + 1]), ssh[:ssh.index("root@colab")])
        self.assertTrue(all(call.kwargs["timeout"] == 120 for call in run.call_args_list))

    def test_transient_connections_are_retried_at_most_five_times(self):
        for code, error in ((255, "connection lost"), (1, "HTTP 429 Too Many Requests"),
                            (12, "rsync: connection unexpectedly closed (code 255)")):
            with self.subTest(code=code), patch("minase_zero.runner.time.sleep") as sleep:
                remote = Remote("l4-one")
                with patch("minase_zero.runner.subprocess.run", return_value=subprocess.CompletedProcess([], code, "", error)) as run:
                    with self.assertRaises(RuntimeError):
                        if code == 12:
                            remote.download("model.onnx", Path("/tmp/unused"))
                        else:
                            remote.usage()
                self.assertEqual(run.call_count, 6)
                self.assertEqual([call.args for call in sleep.call_args_list], [(30,)] * 5)

    def test_recovered_connection_returns_output(self):
        results = [subprocess.CompletedProcess([], 255, "", "connection lost"),
                   subprocess.CompletedProcess([], 0, "Current balance: 12.5 compute units\n", "")]
        with patch("minase_zero.runner.time.sleep"), patch("minase_zero.runner.subprocess.run", side_effect=results):
            self.assertEqual(Remote("l4-one").usage(), 12.5)

    def test_command_failure_and_timeout_are_not_retried(self):
        with patch("minase_zero.runner.time.sleep") as sleep:
            with patch("minase_zero.runner.subprocess.run", return_value=subprocess.CompletedProcess([], 2, "", "invalid command")) as run:
                with self.assertRaises(RuntimeError):
                    Remote("l4-one").usage()
                self.assertEqual(run.call_count, 1)
            with patch("minase_zero.runner.subprocess.run", side_effect=subprocess.TimeoutExpired("ssh", 120)) as run:
                with self.assertRaises(RuntimeError):
                    Remote("l4-one").exists("file")
                self.assertEqual(run.call_count, 1)
            sleep.assert_not_called()

    def test_remote_status_detects_live_dead_and_completed_processes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            remote = Remote("local")

            def shell(script, timeout):
                result = subprocess.run(["bash", "-c", script], cwd=root, capture_output=True, text=True, timeout=timeout)
                if result.returncode:
                    raise RuntimeError(result.stderr)
                return result.stdout

            with patch.object(remote, "shell", side_effect=shell):
                self.assertEqual(remote.status("job"), "new")
                write(root / "job.pid", str(os.getpid()).encode())
                self.assertEqual(remote.status("job"), "running")
                write(root / "job.pid", b"2147483647")
                with self.assertRaises(RuntimeError):
                    remote.status("job")
                write(root / "job.exit", b"7\n")
                self.assertEqual(remote.status("job"), 7)

    def test_detached_start_redirects_all_streams_and_records_exit_after_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root / "env.sh", b"# test environment\n")
            remote = Remote("local")
            scripts = []

            def shell(script, timeout):
                scripts.append(script)
                script = script.replace("source /content/src/tools/train/zero/env.sh", "source " + str(root / "env.sh"))
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
            self.assertIn("setsid nohup bash -c", scripts[0])
            self.assertIn("< /dev/null &", scripts[0])
            self.assertIn("echo $! > job.pid", scripts[0])


class CycleTests(RunnerCase):
    def setUp(self):
        super().setUp()
        (self.root / "seed-ledger.jsonl").touch()
        self.compare_failure = False
        self.compare_failure_weights = "raw"
        self.regenerate = False
        self.remote.on_start = self.complete_job
        teacher = """#!/usr/bin/env python3
import json, pathlib, sys
if any(not pathlib.Path(name).is_file() for name in sys.argv[1:]):
    sys.exit(2)
print(json.dumps({'files': sys.argv[1:], 'game_length_plies': {'p50': 100, 'p90': 120, 'p99': 140}}))
"""
        write(self.options.teacher_stats, teacher.encode())
        self.options.teacher_stats.chmod(0o755)
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
        name = 'cpu-b7-ema.json' if failure.read_text() == 'ema' else 'cpu-b7.json'
        sys.exit(1 if args[args.index('--candidate') + 1].endswith(name) else 0)
    sys.exit(0)
out = pathlib.Path(args[args.index('--out') + 1])
out.parent.mkdir(parents=True, exist_ok=True)
out.write_text(json.dumps({'network_sha256': 'test-network', 'positions': []}) + '\\n')
''')
        self.tool.chmod(0o755)

    def complete_job(self, stem, body):
        if stem.endswith("/check"):
            if self.compare_failure:
                write(self.remote_root / "fail-comparison", self.compare_failure_weights.encode())
            script = body.replace("/content/src/target/release/minase", shlex.quote(str(self.tool)))
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
                      "nn_backend": "tensorrt", "seed": int(value("--seed")), "games": count,
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
                       "resign_triggered_playthrough_games": 100, "false_resignations": 0,
                       "resigned_games_excluded": excluded, "resigned_games": 3 if excluded else 0,
                       "win_reason_royal_capture": count, "discarded_max_ply": 0}
            write_json(self.remote_root / stem / "summary.json", summary)
        elif "train" in command:
            cycle = int(stem.split("/")[1].split("-")[1])
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
            if "--ema-period-samples" in command and int(command[command.index("--ema-period-samples") + 1]) > 0:
                write(self.remote_root / stem / "model-ema.onnx", f"ema network {cycle}".encode())
                summary["ema_network_sha256"] = digest(self.remote_root / stem / "model-ema.onnx")
                summary["ema_period_samples"] = int(command[command.index("--ema-period-samples") + 1])
                summary["ema_scale"] = float(command[command.index("--ema-scale") + 1])
            write_json(self.remote_root / stem / "summary.json", summary)
        else:
            raise AssertionError(body)
        return 0

    def test_two_cycles_record_provenance_checks_cost_and_immutable_networks(self):
        self.execute(replace(self.options, last_cycle=2, window_positions=2000))
        rows = [json.loads(line) for line in (self.root / "cycles.jsonl").read_text().splitlines()]
        intervals = [json.loads(line) for line in (self.root / "sessions.jsonl").read_text().splitlines()]
        self.assertEqual([row["cycle"] for row in rows], [1, 2])
        self.assertEqual(self.remote.machine_calls, 1)
        self.assertEqual([row["generation"]["playthrough_percent"] for row in rows], [100, 20])
        self.assertEqual([row["generation"]["seed"] for row in rows], [1000000000, 2000000000])
        self.assertEqual(rows[1]["generation"]["network"], "cycles/cycle-0001/train/model.onnx")
        self.assertEqual(rows[1]["resign_calibration"]["n"], 100)
        self.assertEqual(rows[0]["generation"]["games"], 10)
        self.assertEqual(rows[0]["generation"]["chunk_games"], 5)
        self.assertEqual(len(rows[1]["training_files"]), 8)
        self.assertEqual(len(rows[1]["teacher_stats"]["files"]), 4)
        self.assertEqual(rows[0]["teacher_stats"]["game_length_plies"], {"p50": 100, "p90": 120, "p99": 140})
        self.assertEqual([row["generation_commit"] for row in rows], [GENERATION_COMMIT, GENERATION_COMMIT])
        self.assertEqual([row["trainer_commit"] for row in rows], [TRAINER_COMMIT, TRAINER_COMMIT])
        self.assertEqual([row["window_cycles"] for row in rows], [[1], [1, 2]])
        self.assertEqual([row["window_positions"] for row in rows], [2000, 2000])
        self.assertEqual(rows[0]["training_summary"]["provenance"]["trainer_commit"], TRAINER_COMMIT)
        self.assertEqual(rows[0]["selfplay_summary"]["win_reason_royal_capture"], 10)
        self.assertEqual(rows[0]["machine"], {"gpu": "NVIDIA L4", "cpu": "Test CPU", "nproc": 12})
        self.assertEqual([row["balance_difference"] for row in intervals], [2, 2])
        self.assertEqual(intervals[1]["started_at"], intervals[0]["ended_at"])
        self.assertEqual(intervals[1]["start_balance"], intervals[0]["end_balance"])
        for row in rows:
            self.assertEqual(row["training_summary"], json.loads(
                (self.root / f"cycles/cycle-{row['cycle']:04d}/train/summary.json").read_text()))
            command = shlex.split(next(body for stem, body, _, _ in self.remote.starts
                                       if stem == f"cycles/cycle-{row['cycle']:04d}/train"))
            data = command[command.index("--data") + 1:command.index("--latest-data")]
            latest = command[command.index("--latest-data") + 1:command.index("--out-dir")]
            self.assertEqual(data, row["training_files"])
            self.assertEqual(latest, row["teacher_stats"]["files"])
            self.assertEqual(latest, data[-len(latest):])
            self.assertEqual([(r["backend"], r["batch"], r["exit_code"]) for r in row["checks"]["results"]],
                             [("cpu", 1, 0), ("trt", 1, 0), ("cpu", 7, 0), ("trt", 7, 0), ("cpu", 256, 0), ("trt", 256, 0)])
            self.assertEqual(set(row["elapsed_seconds"]), {"generation", "training", "checking"})
            self.assertGreaterEqual(row["checks"]["elapsed_seconds"], 0)
        first_commands = [json.loads(line) for line in (self.remote_root / "commands.jsonl").read_text().splitlines()][:13]
        fixture = first_commands[0]
        self.assertEqual(fixture[fixture.index("fixture-eval"):], [
            "fixture-eval", "--checkpoint", "cycles/cycle-0001/train/checkpoint.pt", "--onnx", "cycles/cycle-0001/train/model.onnx",
            "--fixture", "/content/src/tools/train/zero/fixtures/nn-positions.jsonl", "--out", "cycles/cycle-0001/check/pytorch.json",
            "--weights", "raw",
        ])
        for command in first_commands[1:]:
            if "--candidate" in command:
                cpu = "/cpu-" in command[command.index("--candidate") + 1]
                self.assertEqual(command[command.index("--policy") + 1], "logits" if cpu else "probabilities")
                self.assertEqual(command[command.index("--policy-tolerance") + 1], "1e-4" if cpu else "1e-2")
                self.assertEqual(command[command.index("--value-tolerance") + 1], "1e-4" if cpu else "5e-2")
            elif command[command.index("--backend") + 1] == "tensorrt":
                self.assertEqual(command[command.index("--cache-dir") + 1], "/content/cache/nn-eval")
        for cycle in (1, 2):
            target = self.root / f"nets/cycle-{cycle:04d}.onnx"
            self.assertEqual(target.read_bytes(), f"network {cycle}".encode())
            self.assertEqual(target.stat().st_mode & 0o222, 0)
        start_count = len(self.remote.starts)
        self.execute(replace(self.options, last_cycle=2))
        self.assertEqual(len(self.remote.starts), start_count)

    def test_raw_selfplay_with_ema_training_records_and_checks_both_networks(self):
        options = replace(self.options, last_cycle=2, ema_period_samples=500_000, ema_scale=8)

        def before_download(relative, target):
            if "/train/" in relative:
                cycle = relative.split("/")[1]
                self.assertFalse((self.root / "cycles" / cycle / "train/summary.json").exists())

        self.remote.on_download = before_download
        self.execute(options)
        rows = [json.loads(line) for line in (self.root / "cycles.jsonl").read_text().splitlines()]
        self.assertEqual(rows[1]["generation"]["network"], "cycles/cycle-0001/train/model.onnx")
        for row in rows:
            cycle = row["cycle"]
            self.assertEqual((row["selfplay_weights"], row["ema_period_samples"], row["ema_scale"]),
                             ("raw", 500_000, 8))
            expected = [(weights, backend, batch, 0) for weights in ("raw", "ema")
                        for batch in (1, 7, 256) for backend in ("cpu", "trt")]
            self.assertEqual([(r["weights"], r["backend"], r["batch"], r["exit_code"])
                              for r in row["checks"]["results"]], expected)
            for weights, suffix, key in (("raw", "", "network_sha256"), ("ema", "-ema", "ema_network_sha256")):
                target = self.root / f"nets/cycle-{cycle:04d}{suffix}.onnx"
                source = self.root / f"cycles/cycle-{cycle:04d}/train/model{suffix}.onnx"
                self.assertEqual(target.read_bytes(), source.read_bytes())
                self.assertEqual(target.stat().st_mode & 0o222, 0)
                self.assertEqual(row[key], digest(target))
                self.assertEqual(row["training_summary"][key], digest(target))
            command = shlex.split(next(body for stem, body, _, _ in self.remote.starts
                                       if stem == f"cycles/cycle-{cycle:04d}/train"))
            self.assertEqual(command[command.index("--ema-period-samples") + 1], "500000")
            self.assertEqual(command[command.index("--ema-scale") + 1], "8")
        commands = [json.loads(line) for line in (self.remote_root / "commands.jsonl").read_text().splitlines()]
        self.assertEqual(len(commands), 52)
        for cycle, group in ((1, commands[:26]), (2, commands[26:])):
            for weights, offset, suffix in (("raw", 0, ""), ("ema", 13, "-ema")):
                fixture = group[offset]
                self.assertEqual(fixture[fixture.index("--weights") + 1], weights)
                self.assertEqual(fixture[fixture.index("--onnx") + 1], f"cycles/cycle-{cycle:04d}/train/model{suffix}.onnx")
                reference = fixture[fixture.index("--out") + 1]
                for command in group[offset + 1:offset + 13]:
                    if "--candidate" in command:
                        self.assertEqual(command[command.index("--reference") + 1], reference)
                        cpu = "/cpu-" in command[command.index("--candidate") + 1]
                        self.assertEqual(command[command.index("--policy") + 1], "logits" if cpu else "probabilities")
                        self.assertEqual(command[command.index("--policy-tolerance") + 1], "1e-4" if cpu else "1e-2")
                        self.assertEqual(command[command.index("--value-tolerance") + 1], "1e-4" if cpu else "5e-2")
                    else:
                        self.assertEqual(command[command.index("--nn-model") + 1],
                                         f"cycles/cycle-{cycle:04d}/train/model{suffix}.onnx")

    def test_selfplay_can_select_previous_cycles_average(self):
        options = replace(self.options, ema_period_samples=500_000, ema_scale=8)
        self.execute(options)
        self.execute(replace(options, first_cycle=2, last_cycle=2, cycle1_reference_summary=None, selfplay_weights="ema"))
        rows = [json.loads(line) for line in (self.root / "cycles.jsonl").read_text().splitlines()]
        row = rows[1]
        network = "cycles/cycle-0001/train/model-ema.onnx"
        self.assertEqual(row["selfplay_weights"], "ema")
        self.assertEqual(row["generation"]["network"], network)
        self.assertEqual(row["generation"]["network_sha256"], digest(self.root / network))
        self.assertEqual(row["generation"]["network_sha256"], rows[0]["ema_network_sha256"])
        command = shlex.split(next(body for stem, body, _, _ in self.remote.starts
                                   if stem == "cycles/cycle-0002/selfplay"))
        self.assertEqual(command[command.index("--nn-model") + 1], network)

    def test_missing_previous_average_stops_without_substituting_raw_weights(self):
        self.runner.run()
        ledger = (self.root / "seed-ledger.jsonl").read_bytes()
        count = len(self.remote.starts)
        with self.assertRaises(FileNotFoundError):
            self.execute(replace(self.options, first_cycle=2, last_cycle=2,
                                 cycle1_reference_summary=None, selfplay_weights="ema"))
        self.assertEqual(len(self.remote.starts), count)
        self.assertEqual((self.root / "seed-ledger.jsonl").read_bytes(), ledger)

    def test_missing_initial_average_stops_before_generation(self):
        with self.assertRaises(FileNotFoundError):
            self.execute(replace(self.options, selfplay_weights="ema"))
        self.assertEqual(self.remote.starts, [])
        self.assertEqual((self.root / "seed-ledger.jsonl").read_bytes(), b"")

    def check_ema_cycle_failure(self, weights):
        self.compare_failure = True
        self.compare_failure_weights = weights
        with self.assertRaises(RuntimeError):
            self.execute(replace(self.options, last_cycle=2, ema_period_samples=500_000, ema_scale=8))
        check = self.root / "cycles/cycle-0001/check"
        self.assertEqual(len(list(check.glob("*.compare.json"))), 12)
        self.assertEqual(len(list(check.glob("*.compare.exit"))), 12)
        for suffix, expected in (("", "1" if weights == "raw" else "0"),
                                 ("-ema", "1" if weights == "ema" else "0")):
            self.assertEqual((check / f"cpu-b7{suffix}.compare.exit").read_text().strip(), expected)
        self.assertFalse((self.root / "cycles.jsonl").exists())
        self.assertFalse((self.root / "nets/cycle-0001.onnx").exists())
        self.assertFalse((self.root / "nets/cycle-0001-ema.onnx").exists())
        self.assertEqual([Path(item[0]).name for item in self.remote.starts], ["selfplay", "train", "check"])

    def test_raw_comparison_failure_stops_an_ema_cycle_after_both_groups(self):
        self.check_ema_cycle_failure("raw")

    def test_ema_comparison_failure_stops_an_ema_cycle_after_both_groups(self):
        self.check_ema_cycle_failure("ema")

    def test_missing_remote_average_does_not_commit_training_summary(self):
        def complete(stem, body):
            code = self.complete_job(stem, body)
            if stem.endswith("/train"):
                (self.remote_root / stem / "model-ema.onnx").unlink()
            return code

        self.remote.on_start = complete
        with self.assertRaises(FileNotFoundError):
            self.execute(replace(self.options, ema_period_samples=500_000, ema_scale=8))
        self.assertFalse((self.root / "cycles/cycle-0001/train/summary.json").exists())
        self.assertFalse((self.root / "cycles.jsonl").exists())
        self.assertEqual([Path(item[0]).name for item in self.remote.starts], ["selfplay", "train"])

    def test_mismatched_average_hash_does_not_commit_training_summary(self):
        def complete(stem, body):
            code = self.complete_job(stem, body)
            if stem.endswith("/train"):
                path = self.remote_root / stem / "summary.json"
                summary = json.loads(path.read_text())
                summary["ema_network_sha256"] = "f" * 64
                write_json(path, summary)
            return code

        self.remote.on_start = complete
        with self.assertRaises(ValueError):
            self.execute(replace(self.options, ema_period_samples=500_000, ema_scale=8))
        self.assertFalse((self.root / "cycles/cycle-0001/train/summary.json").exists())
        self.assertFalse((self.root / "cycles.jsonl").exists())

    def test_false_resignations_generate_replacements_and_append_them_to_window(self):
        self.regenerate = True
        self.runner.run()
        row = json.loads((self.root / "cycles.jsonl").read_text())
        self.assertEqual(row["regeneration"]["seed"], 1500000000)
        self.assertEqual(row["regeneration"]["games"], 3)
        self.assertEqual(row["regeneration"]["chunk_games"], 2)
        self.assertEqual(row["regeneration"]["playthrough_percent"], 100)
        self.assertEqual(row["selfplay_regen_summary"]["games"], 3)
        self.assertEqual(row["training_files"], [
            "cycles/cycle-0001/selfplay/chunk-00000.mnsd", "cycles/cycle-0001/selfplay/chunk-00001.mnsd",
            "cycles/cycle-0001/selfplay-regen/chunk-00000.mnsd", "cycles/cycle-0001/selfplay-regen/chunk-00000.resigned.mnsd",
            "cycles/cycle-0001/selfplay-regen/chunk-00001.mnsd", "cycles/cycle-0001/selfplay-regen/chunk-00001.resigned.mnsd",
        ])
        command = shlex.split(next(body for stem, body, _, _ in self.remote.starts if stem.endswith("/train")))
        self.assertEqual(command[command.index("--latest-data") + 1:command.index("--out-dir")], row["training_files"])
        ledger = [json.loads(line) for line in (self.root / "seed-ledger.jsonl").read_text().splitlines()]
        self.assertEqual([entry["name"] for entry in ledger], ["cycle-0001-gen", "cycle-0001-regen"])

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
            result, _ = self.runner.generation(1, stem, "cycles/cycle-0000/model.onnx", 1000000000, 10, 5, 100)
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
        _, files, latest = training_window(self.root, 1, 1000)
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
        self.assertEqual(command[command.index("train"):], ["train", "--resume", "cycles/cycle-0001/train", "--device", "cuda"])
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
        self.assertEqual(batches, [row["training_files"]])

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
    def arguments(self):
        return ["run-cycles", "--root", str(self.root), "--session", "l4-one",
                "--generation-commit", GENERATION_COMMIT, "--trainer-commit", TRAINER_COMMIT,
                "--first-cycle", "1", "--last-cycle", "3", "--deadline", "2026-10-29T12:00:00+09:00",
                "--cycle-minutes", "40", "--cycle-cu", "2", "--target-positions", "1000000",
                "--chunks", "20", "--simulations", "32", "--threads", "6", "--games-per-thread", "4",
                "--window-positions", "34625240", "--batch-size", "256", "--lr", "0.01", "--momentum", "0.9",
                "--weight-decay", "0.0001", "--clip-grad-norm", "10", "--value-lambda", "0.333",
                "--precision", "bf16", "--checkpoint-steps", "200", "--train-seed-base", "123",
                "--selfplay-weights", "raw", "--ema-period-samples", "500000", "--ema-scale", "8",
                "--teacher-stats", str(self.options.teacher_stats), "--poll-seconds", "60",
                "--cycle1-reference-summary", str(self.reference)]

    def test_all_arguments_other_than_reference_are_required(self):
        args = self.arguments()
        parsed = vars(_parse(args))
        self.assertEqual(parsed.pop("command"), "run-cycles")
        self.assertEqual(Options(**parsed).deadline, datetime(2026, 10, 29, 3, 0, tzinfo=UTC))
        for index in range(1, len(args) - 2, 2):
            with self.subTest(missing=args[index]), self.assertRaises(ValueError):
                _parse(args[:index] + args[index + 2:])

    def test_reference_is_required_exactly_for_first_cycle_one(self):
        with self.assertRaises(ValueError):
            replace(self.options, cycle1_reference_summary=None)
        with self.assertRaises(ValueError):
            replace(self.options, first_cycle=2, last_cycle=3)
        result = replace(self.options, first_cycle=2, last_cycle=3, cycle1_reference_summary=None)
        self.assertIsNone(result.cycle1_reference_summary)

    def test_invalid_inputs_fail_before_remote_work(self):
        for changes in ({"deadline": NOW.replace(tzinfo=None)}, {"generation_commit": "abcd"},
                        {"trainer_commit": "abcd"}, {"session": "x; echo secret"},
                        {"poll_seconds": 0}, {"window_positions": 0}, {"cycle_cu": float("nan")}, {"cycle_minutes": -1},
                        {"weight_decay": -1}, {"value_lambda": 2}, {"train_seed_base": 2**64 - 1},
                        {"selfplay_weights": "invalid"}, {"ema_period_samples": 1}, {"ema_scale": 8},
                        {"ema_period_samples": -1}, {"ema_period_samples": 1.5},
                        {"ema_period_samples": 3, "ema_scale": float("nan")}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                replace(self.options, **changes)


if __name__ == "__main__":
    unittest.main()
