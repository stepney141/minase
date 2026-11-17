"""リプレイバッファの仕様から、抽出数、分割、保持件数、更新順と再開時の照合を検証する。"""

from dataclasses import fields
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import time
import unittest
from unittest.mock import patch

import minase_mnsd_reader
import numpy as np
import torch

from minase_zero.data import Dataset, is_training_sample, read_replay_buffer, sample_training_indices

from tests.cli import (ZERO, command, environment, init_arguments, kill_training,
                  require_success, run_cli, set_training_data, train_arguments)
from tests.test_data import single_game, write_small


# shuffle.rsの既知値。seed=11と22の0..79について、検証に入る対局番号である。
VALIDATION_GAMES = {11: {4, 15, 22, 41, 48, 59, 70}, 22: {6, 8, 35, 44, 58, 63, 74}}


def reference_sample(total: int, count: int, seed: int) -> list[int]:
    """docstringの交換式を、全長Tの通常の配列で独立に評価する。"""
    rng = np.random.Generator(np.random.PCG64(seed))
    if count > total:
        raise ValueError("sample count must not exceed total")
    permutation = list(range(total))
    for index in range(count):
        other = int(rng.integers(index, total, dtype=np.int64))
        permutation[index], permutation[other] = permutation[other], permutation[index]
    return permutation[:count]


def cycle_files(directory: Path) -> list[Path]:
    """過去サイクルの1ファイルと、最新サイクルの2つの塊を作る。"""
    paths = [directory / name for name in ("older.mnsd", "latest-chunk-00000.mnsd", "latest-chunk-00001.mnsd")]
    for path, seed, games in zip(paths, (11, 22, 22), (range(80), range(40), range(40, 80)), strict=True):
        write_small(path, [single_game(game, game % 2, game % 3, 200 * game, (65_535,)) for game in games],
                    "lishogi")
        content = bytearray(path.read_bytes())
        struct.pack_into("<Q", content, 128, seed)
        path.write_bytes(content)
    return paths


def training_keys() -> list[tuple[int, int]]:
    """既知の検証分割から、ファイル番号と対局番号を抽出前の通し番号順に並べる。"""
    return [(file, game)
            for file, seed, games in ((0, 11, range(80)), (1, 22, range(40)), (2, 22, range(40, 80)))
            for game in games if game not in VALIDATION_GAMES[seed]]


class ReplayBufferTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-replay-buffer-")
        self.addCleanup(directory.cleanup)
        self.paths = cycle_files(Path(directory.name))

    def test_sparse_sampling_matches_the_exchange_contract_and_preserves_random_state(self) -> None:
        state = np.random.get_state()
        for total in (1, 2, 7, 31):
            for count in (1, total // 2 + 1, total):
                for seed in (0, 42, (1 << 64) - 1):
                    with self.subTest(total=total, count=count, seed=seed):
                        selected = sample_training_indices(total, count, seed)
                        self.assertEqual(selected.dtype, np.int64)
                        self.assertEqual(selected.tolist(), reference_sample(total, count, seed))
                        self.assertEqual(len(np.unique(selected)), count)
                        self.assertTrue(np.all((0 <= selected) & (selected < total)))
        after = np.random.get_state()
        self.assertEqual(state[0], after[0])
        np.testing.assert_array_equal(state[1], after[1])
        self.assertEqual(state[2:], after[2:])

    def test_sampling_rejects_count_above_total(self) -> None:
        for total in (1, 2, 7, 31):
            for count in (total + 1, 2 * total, 4 * total + 1):
                with self.subTest(total=total, count=count):
                    with self.assertRaisesRegex(ValueError, "sample count must not exceed total"):
                        sample_training_indices(total, count, 42)

    def test_sampling_preserves_fixed_sequences(self) -> None:
        # count <= totalの列は変更しない契約に従い、修正前に取得した小さな例を固定する。
        cases = ((1, 1, 0, [0]),
                 (7, 4, 0, [5, 4, 1, 2]),
                 (7, 7, 42, [0, 5, 1, 4, 2, 6, 3]),
                 (7, 4, (1 << 64) - 1, [3, 5, 6, 2]))
        for total, count, seed, expected in cases:
            with self.subTest(total=total, count=count, seed=seed):
                self.assertEqual(sample_training_indices(total, count, seed).tolist(), expected)

    def test_same_seed_reproduces_selection_and_other_seed_changes_it(self) -> None:
        data, infos, first, _ = read_replay_buffer(self.paths, self.paths[1:], 42,
            samples="latest", rank=0, world_size=1, batch_size=1, micro_batch_size=1, replay_threads=1,
        )
        repeated_data, _, repeated, _ = read_replay_buffer(self.paths, self.paths[1:], 42,
            samples="latest", rank=0, world_size=1, batch_size=1, micro_batch_size=1, replay_threads=1,
        )
        _, _, different, _ = read_replay_buffer(self.paths, self.paths[1:], 43,
            samples="latest", rank=0, world_size=1, batch_size=1, micro_batch_size=1, replay_threads=1,
        )
        self.assertEqual([info.training_count for info in infos], [73, 37, 36])
        self.assertEqual(first.tolist(), reference_sample(146, 73, 42))
        np.testing.assert_array_equal(first, repeated)
        self.assertNotEqual(first.tolist(), different.tolist())
        for field in fields(Dataset):
            np.testing.assert_array_equal(getattr(data, field.name), getattr(repeated_data, field.name))

    def test_selected_positions_exclude_validation_and_keep_only_latest_validation(self) -> None:
        data, _, selected, _ = read_replay_buffer(self.paths, self.paths[1:], 42,
            samples="latest", rank=0, world_size=1, batch_size=1, micro_batch_size=1, replay_threads=1,
        )
        self.assertEqual(len(selected), 73)
        self.assertEqual(len(set(selected)), 73)
        keys = training_keys()
        expected = [keys[index] for index in sorted(set(selected))]
        actual = list(zip(data.file_indices[~data.validation].tolist(), data.games[~data.validation].tolist()))
        self.assertEqual(actual, expected)
        self.assertTrue(np.all(data.plies == 23))
        actual_validation = set(zip(data.file_indices[data.validation].tolist(), data.games[data.validation].tolist()))
        self.assertEqual(actual_validation, {(1 if game < 40 else 2, game) for game in VALIDATION_GAMES[22]})
        latest_sample = data.training_sample & (data.file_indices >= 1)
        self.assertTrue(np.any(latest_sample))
        self.assertTrue(np.all(is_training_sample(22, data.games[latest_sample])))
        self.assertFalse(np.any(data.training_sample & data.validation))

    def test_extension_and_python_keep_only_sampled_rows_and_latest_validation(self) -> None:
        data, infos, selected, _ = read_replay_buffer(self.paths, self.paths[1:], 42,
            samples="latest", rank=0, world_size=1, batch_size=1, micro_batch_size=1, replay_threads=1,
        )
        self.assertEqual(len(data), 73 + 7)
        self.assertLess(len(data), sum(info.position_count for info in infos))
        offset = 0
        rows = 0
        for index, info in enumerate(infos):
            local = np.unique(selected[(selected >= offset) & (selected < offset + info.training_count)] - offset)
            raw = minase_mnsd_reader.read_selected(str(info.path), local.tolist(), [], index >= 1)
            kept = len(local) + (info.validation_count if index >= 1 else 0)
            self.assertEqual(len(raw["games"]), kept * 4)
            self.assertEqual(len(raw["boards"]), kept * 144)
            self.assertEqual(len(raw["policy_offsets"]), (kept + 1) * 8)
            self.assertEqual(len(raw["legal_offsets"]), (kept + 1) * 8)
            rows += kept
            offset += info.training_count
        self.assertEqual(rows, len(data))

    def test_latest_paths_must_match_the_suffix_as_strings_in_order(self) -> None:
        first, second, third = self.paths
        alias = str(third.parent) + "/./" + third.name
        for latest in ([], [first], [third, second], [second], [alias], [first, second, third, third]):
            with self.subTest(latest=latest), self.assertRaises(ValueError):
                read_replay_buffer([str(path) for path in self.paths], latest, 42,
                    samples="latest", rank=0, world_size=1, batch_size=1, micro_batch_size=1, replay_threads=1,
                )
        # パス列を並べ替えれば通し番号も変わるが、新しい末尾に対する抽出は定義できる。
        _, infos, selected, _ = read_replay_buffer([third, second, first], [first], 42,
            samples="latest", rank=0, world_size=1, batch_size=1, micro_batch_size=1, replay_threads=1,
        )
        self.assertEqual([info.training_count for info in infos], [36, 37, 73])
        self.assertEqual(len(selected), 73)

    def test_latest_with_no_training_positions_and_invalid_sampling_ranges_raise(self) -> None:
        path = self.paths[-1]
        # seed=20261038の対局番号によらず、価値なしの項目は訓練局面数に含まない。
        write_small(path, [single_game(100, 0, 2, None, ())], "lishogi")
        with self.assertRaises(ValueError):
            read_replay_buffer(self.paths, [path], 42,
                samples="latest", rank=0, world_size=1, batch_size=1, micro_batch_size=1, replay_threads=1,
            )
        for total, count, seed in ((1, 0, 0), (0, 2, 0), (1, 1, -1), (1, 1, 1 << 64)):
            with self.subTest(total=total, count=count, seed=seed), self.assertRaises(ValueError):
                sample_training_indices(total, count, seed)

    def test_ranks_keep_assigned_occurrences_and_rank_zero_diagnostics_without_the_tail(self):
        keys = training_keys()
        # 最新が1ファイルならL=36、2ファイルならL=73である。
        # B=32の倍数だけ使い、末尾の4局面または9局面を捨てる。
        for latest, count in ((self.paths[-1:], 36), (self.paths[1:], 73)):
            selected = reference_sample(146, count, 42)
            used = selected[:count // 32 * 32]
            diagnostics = {index for index in used
                           if is_training_sample(11 if keys[index][0] == 0 else 22,
                                                 np.array([keys[index][1]], dtype=np.uint32))[0]}
            for n in (1, 2, 4):
                for rank in range(n):
                    with self.subTest(count=count, n=n, rank=rank):
                        expected = {ordinal for position, ordinal in enumerate(used) if (position // 8) % n == rank}
                        if rank == 0:
                            expected |= diagnostics
                        data, infos, actual, retained = read_replay_buffer(
                            self.paths, latest, 42, samples="latest", rank=rank, world_size=n,
                            batch_size=32, micro_batch_size=8, replay_threads=2,
                        )
                        self.assertEqual(actual.tolist(), selected)
                        self.assertEqual(retained.tolist(), sorted(expected))
                        self.assertEqual(list(zip(data.file_indices[~data.validation], data.games[~data.validation])),
                                         [keys[index] for index in sorted(expected)])
                        validation_count = sum(info.validation_count for info in infos[-len(latest):]) if rank == 0 else 0
                        self.assertEqual(np.count_nonzero(data.validation), validation_count)
                        if rank == 0:
                            diagnostic_rows = list(zip(data.file_indices[data.training_sample], data.games[data.training_sample]))
                            self.assertEqual(diagnostic_rows, [keys[index] for index in sorted(diagnostics)])

    def test_insufficient_sample_count_fails_before_any_replay(self):
        with patch("minase_zero.data._read_selected", side_effect=AssertionError("replay must not start")):
            with self.assertRaisesRegex(ValueError, "sample count must be at least batch_size"):
                read_replay_buffer(self.paths, self.paths[1:], 42, samples="latest", rank=0, world_size=1,
                                   batch_size=74, micro_batch_size=1, replay_threads=2)

    def test_latest_size_equal_to_batch_size_makes_one_full_update(self):
        # 最新2ファイルは37+36=73局面なので、B=73なら全抽出局面を1更新で使う。
        data, _, selected, retained = read_replay_buffer(
            self.paths, self.paths[1:], 42, samples="latest", rank=0, world_size=1,
            batch_size=73, micro_batch_size=73, replay_threads=2,
        )
        self.assertEqual(len(selected), 73)
        self.assertEqual(len(set(selected)), 73)
        self.assertEqual(retained.tolist(), sorted(selected))
        self.assertEqual(np.count_nonzero(~data.validation), 73)

    def test_integer_samples_selects_requested_count_without_replacement_and_drops_tail(self):
        # 最新は73局面、全体は146局面。1、Lより大きい数、Tを境界として確かめる。
        for count, batch_size in ((1, 1), (101, 4), (146, 2)):
            with self.subTest(count=count):
                data, _, selected, retained = read_replay_buffer(
                    self.paths, self.paths[1:], 42, samples=count, rank=0, world_size=1,
                    batch_size=batch_size, micro_batch_size=1, replay_threads=2,
                )
                self.assertEqual(len(selected), count)
                self.assertEqual(len(set(selected)), count)
                self.assertTrue(np.all((0 <= selected) & (selected < 146)))
                self.assertEqual(selected.tolist(), reference_sample(146, count, 42))
                used_count = batch_size * (count // batch_size)
                self.assertEqual(retained.tolist(), sorted(selected[:used_count]))
                self.assertEqual(np.count_nonzero(~data.validation), used_count)

    def test_integer_samples_outside_training_bounds_fails_before_replay(self):
        for count, message in ((147, "must not exceed.*146"), (1, "at least batch_size")):
            with (self.subTest(count=count),
                  patch("minase_zero.data._read_selected", side_effect=AssertionError("replay must not start")),
                  self.assertRaisesRegex(ValueError, message)):
                read_replay_buffer(self.paths, self.paths[1:], 42, samples=count, rank=0, world_size=1,
                                   batch_size=2, micro_batch_size=1, replay_threads=2)


class ReplayBufferTrainTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-replay-buffer-train-")
        cls.addClassCleanup(directory.cleanup)
        cls.root = Path(directory.name)
        cls.paths = cycle_files(cls.root)
        cls.initial, cls.continuous, cls.resumed, cls.partial = (
            cls.root / name for name in ("initial", "continuous", "resumed", "partial")
        )
        require_success(run_cli(init_arguments(cls.initial, 29)))
        require_success(run_cli(cls.arguments(cls.continuous)))
        with (cls.root / "interrupted.log").open("w+") as log:
            process = subprocess.Popen(command(cls.arguments(cls.resumed)),
                                       cwd=ZERO, env=environment(), stdout=log, stderr=log, start_new_session=True)
            try:
                deadline = time.monotonic() + 120
                while not (cls.resumed / "checkpoint.pt").exists():
                    if process.poll() is not None or time.monotonic() >= deadline:
                        log.seek(0)
                        raise AssertionError("no intermediate checkpoint: " + log.read())
                    time.sleep(0.002)
                kill_training(process)
            finally:
                if process.poll() is None:
                    kill_training(process)
        interrupted = torch.load(cls.resumed / "checkpoint.pt", weights_only=True)
        if not 0 < interrupted["completed_steps"] < interrupted["total_steps"]:
            raise AssertionError("training was not interrupted between updates")
        shutil.copytree(cls.resumed, cls.partial)
        require_success(run_cli(["train", "--resume", str(cls.resumed), "--device", "cpu", "--replay-threads", "1", "--lr", "0.01"]))
        cls.state = torch.load(cls.continuous / "checkpoint.pt", weights_only=True)
        cls.summary = json.loads((cls.continuous / "summary.json").read_text())

    @classmethod
    def arguments(cls, out: Path) -> list[str]:
        arguments = train_arguments(cls.initial / "checkpoint.pt", out)
        set_training_data(arguments, [str(path) for path in cls.paths])
        for flag, values in (("--latest-data", [str(path) for path in cls.paths[1:]]),
                             ("--batch-size", ["2"]), ("--micro-batch-size", ["2"]), ("--checkpoint-steps", ["2"])):
            start = arguments.index(flag) + 1
            arguments[start:start + 1] = values
        return arguments

    def test_replay_buffer_resume_preserves_order_learning_curve_and_onnx_bytes(self) -> None:
        resumed = torch.load(self.resumed / "checkpoint.pt", weights_only=True)
        summary = json.loads((self.resumed / "summary.json").read_text())
        for key in ("data_order_sha256", "sample_sha256", "learning_curve", "positions_seen", "loss_sums", "optimizer_steps", "optimizer_steps_start"):
            self.assertEqual(self.state[key], resumed[key], key)
        self.assertEqual(self.summary["diagnostics"], summary["diagnostics"])
        self.assertEqual((self.continuous / "model.onnx").read_bytes(), (self.resumed / "model.onnx").read_bytes())

    def test_summary_and_checkpoint_record_sampling_contract_and_global_update_ordinals(self) -> None:
        ordered = reference_sample(146, 73, 42)
        sample_digest = hashlib.sha256(b"".join(struct.pack("<q", index) for index in ordered)).hexdigest()
        expected = {"latest_data": [str(path) for path in self.paths[1:]], "samples": "latest", "sample_count": 73,
                    "replay_buffer_training_count": 146, "file_training_counts": [73, 37, 36],
                    "sample_sha256": sample_digest}
        for record in (self.state, self.summary, self.state["config"], self.summary["config"]):
            for key, value in expected.items():
                self.assertEqual(record[key], value, key)
        digest = bytes(32)
        for start in range(0, 72, 2):
            digest = hashlib.sha256(digest + b"".join(struct.pack("<q", index) for index in ordered[start:start + 2])).digest()
        self.assertEqual(self.summary["data_order_sha256"], digest.hex())
        self.assertEqual(self.summary["training_count"], 72)
        self.assertEqual(self.summary["validation_count"], 7)
        self.assertEqual(self.summary["positions_seen"], 72)
        self.assertEqual(self.summary["total_steps"], 36)

    def test_larger_replay_buffer_trains_one_latest_cycle_worth_without_duplicates(self) -> None:
        # 最新を対局0〜38の36訓練局面とし、診断用の対局32も含める。
        # リプレイバッファは73+36+36=145局面であり、最新と同数の36局面を重複なく選ぶ。
        latest = self.root / "distinct-latest.mnsd"
        write_small(latest, [single_game(game, game % 2, game % 3, 200 * game, (65_535,))
                             for game in range(39)], "lishogi")
        content = bytearray(latest.read_bytes())
        struct.pack_into("<Q", content, 128, 22)
        latest.write_bytes(content)
        out = self.root / "distinct-sequence"
        arguments = self.arguments(out)
        set_training_data(arguments, [str(self.paths[0]), str(self.paths[2]), str(latest)])
        first = arguments.index("--latest-data") + 1
        arguments[first:first + 2] = [str(latest)]
        arguments[arguments.index("--batch-size") + 1] = "36"
        # seed=43では、最新ファイルの診断用対局32を含む36局面が選ばれる。
        arguments[arguments.index("--seed") + 1] = "43"
        require_success(run_cli(arguments))
        summary = json.loads((out / "summary.json").read_text())
        ordered = reference_sample(145, 36, 43)
        encoded = b"".join(struct.pack("<q", index) for index in ordered)
        self.assertEqual(len(set(ordered)), 36)
        self.assertEqual(summary["sample_count"], 36)
        self.assertEqual(summary["training_count"], 36)
        self.assertEqual(summary["total_steps"], 1)
        self.assertEqual(summary["positions_seen"], 36)
        self.assertEqual(summary["optimizer_steps"], 1)
        self.assertEqual(summary["last_lr"], 0.01 / 250)
        self.assertEqual(summary["sample_sha256"], hashlib.sha256(encoded).hexdigest())
        self.assertEqual(summary["data_order_sha256"], hashlib.sha256(bytes(32) + encoded).hexdigest())

    def test_diagnostics_use_sampled_games_and_latest_cycle_only(self) -> None:
        keys = training_keys()
        selected = [keys[index] for index in sorted(reference_sample(146, 73, 42)[:72])]
        samples = [(file, game) for file, game in selected
                   if is_training_sample(11 if file == 0 else 22, np.array([game], dtype=np.uint32))[0]]
        for name, positions in (("training_sample", samples),
                                ("latest_training_sample", [(file, game) for file, game in samples if file >= 1]),
                                ("validation", [(1 if game < 40 else 2, game) for game in VALIDATION_GAMES[22]])):
            diagnostic = self.summary["diagnostics"][name]
            expected = [{"seed": 11 if file == 0 else 22, "game": game} for file, game in sorted(positions)]
            self.assertEqual(diagnostic["games"], expected)
            self.assertEqual(diagnostic["position_count"], len(positions))
            self.assertEqual(diagnostic["game_count"], len(positions))
            for metric in ("policy", "value", "value_result", "legal_probability_mass",
                           "log_legal_probability_mass", "policy_agreement"):
                self.assertTrue(math.isfinite(diagnostic[metric]), metric)

    def test_resume_rejects_changed_sampling_conditions(self) -> None:
        changes = {"latest_data": [str(self.paths[-1])], "samples": 101, "sample_count": 74,
                   "replay_buffer_training_count": 147, "file_training_counts": [72, 38, 36],
                   "sample_sha256": "0" * 64, "seed": 43}
        for key, value in changes.items():
            with self.subTest(key=key):
                out = self.root / f"changed-{key}"
                shutil.copytree(self.partial, out)
                state = torch.load(out / "checkpoint.pt", weights_only=True)
                state["config"][key] = value
                torch.save(state, out / "checkpoint.pt")
                result = run_cli(["train", "--resume", str(out), "--device", "cpu", "--replay-threads", "1", "--lr", "0.01"])
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(sum(line.startswith("minase_zero:") for line in result.stderr.splitlines()), 1)
                self.assertFalse((out / "summary.json").exists())

    def test_integer_samples_resume_preserves_selection_and_finishes(self):
        continuous, resumed = (self.root / name for name in ("integer-continuous", "integer-resumed"))

        def arguments(out):
            args = self.arguments(out)
            args[args.index("--samples") + 1] = "101"
            return args

        require_success(run_cli(arguments(continuous)))
        with (self.root / "integer-interrupted.log").open("w+") as log:
            process = subprocess.Popen(command(arguments(resumed)), cwd=ZERO, env=environment(),
                                       stdout=log, stderr=log, start_new_session=True)
            try:
                deadline = time.monotonic() + 120
                while not (resumed / "checkpoint.pt").exists():
                    if process.poll() is not None or time.monotonic() >= deadline:
                        log.seek(0)
                        self.fail("no intermediate checkpoint: " + log.read())
                    time.sleep(0.002)
                kill_training(process)
            finally:
                if process.poll() is None:
                    kill_training(process)
        interrupted = torch.load(resumed / "checkpoint.pt", weights_only=True)
        self.assertGreater(interrupted["completed_steps"], 0)
        self.assertLess(interrupted["completed_steps"], interrupted["total_steps"])
        self.assertFalse((resumed / "summary.json").exists())
        require_success(run_cli(["train", "--resume", str(resumed), "--device", "cpu",
                                 "--replay-threads", "1", "--lr", "0.01"]))
        ordered = reference_sample(146, 101, 42)
        digest = hashlib.sha256(b"".join(struct.pack("<q", index) for index in ordered)).hexdigest()
        self.assertEqual(interrupted["sample_sha256"], digest)
        summaries = []
        for out in (continuous, resumed):
            state = torch.load(out / "checkpoint.pt", weights_only=True)
            summary = json.loads((out / "summary.json").read_text())
            summaries.append(summary)
            for record in (state, summary):
                self.assertEqual(record["config"]["samples"], 101)
                self.assertEqual(record["sample_count"], 101)
                self.assertEqual(record["sample_sha256"], digest)
                self.assertEqual(record["positions_seen"], 100)
                self.assertEqual(record["completed_steps"], 50)
                self.assertEqual(record["total_steps"], 50)
        for key in ("data_order_sha256", "learning_curve", "diagnostics"):
            self.assertEqual(summaries[0][key], summaries[1][key])
        self.assertEqual((continuous / "model.onnx").read_bytes(), (resumed / "model.onnx").read_bytes())

    def test_checkpoint_without_samples_can_initialize_but_cannot_resume(self):
        # 完了済みの訓練チェックポイントから記録だけを除き、次サイクルの初期値にする。
        initial = self.root / "without-samples.pt"
        state = torch.load(self.continuous / "checkpoint.pt", weights_only=True)
        del state["config"]["samples"]
        del state["samples"]
        torch.save(state, initial)
        out = self.root / "init-without-samples"
        args = self.arguments(out)
        args[args.index("--init") + 1] = str(initial)
        args[args.index("--samples") + 1] = "146"
        args[args.index("--batch-size") + 1] = "146"
        require_success(run_cli(args))
        summary = json.loads((out / "summary.json").read_text())
        self.assertEqual(summary["config"]["samples"], 146)
        self.assertEqual(summary["sample_count"], 146)
        self.assertEqual(summary["optimizer_steps_start"], state["optimizer_steps"])
        out = self.root / "resume-without-samples"
        shutil.copytree(self.partial, out)
        state = torch.load(out / "checkpoint.pt", weights_only=True)
        del state["config"]["samples"]
        del state["samples"]
        torch.save(state, out / "checkpoint.pt")
        result = run_cli(["train", "--resume", str(out), "--device", "cpu", "--replay-threads", "1", "--lr", "0.01"])
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("resume requires samples in checkpoint config", result.stderr)
        self.assertFalse((out / "summary.json").exists())

    def test_cli_requires_latest_data_and_rejects_non_suffix_order(self) -> None:
        missing = self.arguments(self.root / "missing-latest")
        start = missing.index("--latest-data")
        del missing[start:start + 3]
        reversed_latest = self.arguments(self.root / "reversed-latest")
        start = reversed_latest.index("--latest-data") + 1
        reversed_latest[start:start + 2] = reversed(reversed_latest[start:start + 2])
        for arguments in (missing, reversed_latest):
            with self.subTest(arguments=arguments):
                result = run_cli(arguments)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(sum(line.startswith("minase_zero:") for line in result.stderr.splitlines()), 1)
                self.assertIn("latest-data", result.stderr)


if __name__ == "__main__":
    unittest.main()
