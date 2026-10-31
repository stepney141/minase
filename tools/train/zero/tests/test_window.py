"""学習窓の仕様から、抽出数、分割、保持件数、更新順と再開時の照合を検証する。"""

from dataclasses import fields
import hashlib
import json
import math
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import unittest

import minase_mnsd_reader
import numpy as np
import torch

from minase_zero.data import Dataset, is_training_sample, read_window, sample_training_indices

from .cli import ZERO, environment, init_arguments, require_success, run_cli, train_arguments
from .test_data import single_game, write_small


# shuffle.rsの既知値。seed=11と22の0..79について、検証に入る対局番号である。
VALIDATION_GAMES = {11: {4, 15, 22, 41, 48, 59, 70}, 22: {6, 8, 35, 44, 58, 63, 74}}


def reference_sample(total: int, count: int, seed: int) -> list[int]:
    """docstringの交換式を、全長Tの通常の配列で独立に評価する。"""
    permutation = list(range(total))
    rng = np.random.Generator(np.random.PCG64(seed))
    for index in range(count):
        other = int(rng.integers(index, total, dtype=np.int64))
        permutation[index], permutation[other] = permutation[other], permutation[index]
    return permutation[:count]


def cycle_files(directory: Path) -> list[Path]:
    """過去1ファイルと、最新サイクルの通常生成と再生成に相当する2ファイルを作る。"""
    paths = [directory / name for name in ("older.mnsd", "latest.mnsd", "regenerated.mnsd")]
    for path, seed, games in zip(paths, (11, 22, 22), (range(80), range(40), range(40, 80)), strict=True):
        write_small(path, [single_game(game, game % 2, game % 3, 200 * game, (65_535,)) for game in games],
                    "engine-default")
        content = bytearray(path.read_bytes())
        struct.pack_into("<Q", content, 128, seed)
        path.write_bytes(content)
    return paths


def training_keys() -> list[tuple[int, int]]:
    """既知の検証分割から、ファイル番号と対局番号を抽出前の通し番号順に並べる。"""
    return [(file, game)
            for file, seed, games in ((0, 11, range(80)), (1, 22, range(40)), (2, 22, range(40, 80)))
            for game in games if game not in VALIDATION_GAMES[seed]]


class WindowTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-window-")
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

    def test_same_seed_reproduces_selection_and_other_seed_changes_it(self) -> None:
        data, infos, first = read_window(self.paths, self.paths[1:], 42)
        repeated_data, _, repeated = read_window(self.paths, self.paths[1:], 42)
        _, _, different = read_window(self.paths, self.paths[1:], 43)
        self.assertEqual([info.training_count for info in infos], [73, 37, 36])
        self.assertEqual(first.tolist(), reference_sample(146, 73, 42))
        np.testing.assert_array_equal(first, repeated)
        self.assertNotEqual(set(first), set(different))
        for field in fields(Dataset):
            np.testing.assert_array_equal(getattr(data, field.name), getattr(repeated_data, field.name))

    def test_selected_positions_exclude_validation_and_keep_only_latest_validation(self) -> None:
        data, _, selected = read_window(self.paths, self.paths[1:], 42)
        self.assertEqual(len(selected), 73)
        self.assertEqual(len(set(selected)), 73)
        keys = training_keys()
        expected = [keys[index] for index in sorted(selected)]
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
        data, infos, selected = read_window(self.paths, self.paths[1:], 42)
        self.assertEqual(len(data), 73 + 7)
        self.assertLess(len(data), sum(info.position_count for info in infos))
        offset = 0
        rows = 0
        for index, info in enumerate(infos):
            local = np.sort(selected[(selected >= offset) & (selected < offset + info.training_count)] - offset)
            raw = minase_mnsd_reader.read_selected(str(info.path), local.tolist(), index >= 1)
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
                read_window([str(path) for path in self.paths], latest, 42)
        # パス列を並べ替えれば通し番号も変わるが、新しい末尾に対する抽出は定義できる。
        _, infos, selected = read_window([third, second, first], [first], 42)
        self.assertEqual([info.training_count for info in infos], [36, 37, 73])
        self.assertEqual(len(selected), 73)

    def test_latest_with_no_training_positions_and_invalid_sampling_ranges_raise(self) -> None:
        path = self.paths[-1]
        # seed=20261038の対局番号によらず、価値なしの項目は訓練局面数に含まない。
        write_small(path, [single_game(100, 0, 2, None, ())], "engine-default")
        with self.assertRaises(ValueError):
            read_window(self.paths, [path], 42)
        for total, count, seed in ((1, 0, 0), (1, 2, 0), (1, 1, -1), (1, 1, 1 << 64)):
            with self.subTest(total=total, count=count, seed=seed), self.assertRaises(ValueError):
                sample_training_indices(total, count, seed)


class WindowTrainTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-window-train-")
        cls.addClassCleanup(directory.cleanup)
        cls.root = Path(directory.name)
        cls.paths = cycle_files(cls.root)
        cls.initial, cls.continuous, cls.resumed, cls.partial = (
            cls.root / name for name in ("initial", "continuous", "resumed", "partial")
        )
        require_success(run_cli(init_arguments(cls.initial, 29)))
        require_success(run_cli(cls.arguments(cls.continuous)))
        with (cls.root / "interrupted.log").open("w+") as log:
            process = subprocess.Popen([sys.executable, "-m", "minase_zero", *cls.arguments(cls.resumed)],
                                       cwd=ZERO, env=environment(), stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 120
                while not (cls.resumed / "checkpoint.pt").exists():
                    if process.poll() is not None or time.monotonic() >= deadline:
                        log.seek(0)
                        raise AssertionError("no intermediate checkpoint: " + log.read())
                    time.sleep(0.002)
                process.kill()
                process.wait(timeout=30)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=30)
        interrupted = torch.load(cls.resumed / "checkpoint.pt", weights_only=True)
        if not 0 < interrupted["completed_steps"] < interrupted["total_steps"]:
            raise AssertionError("training was not interrupted between updates")
        shutil.copytree(cls.resumed, cls.partial)
        require_success(run_cli(["train", "--resume", str(cls.resumed), "--device", "cpu"]))
        cls.state = torch.load(cls.continuous / "checkpoint.pt", weights_only=True)
        cls.summary = json.loads((cls.continuous / "summary.json").read_text())

    @classmethod
    def arguments(cls, out: Path) -> list[str]:
        arguments = train_arguments(cls.initial / "checkpoint.pt", out)
        for flag, values in (("--data", [str(path) for path in cls.paths]),
                             ("--latest-data", [str(path) for path in cls.paths[1:]]),
                             ("--batch-size", ["2"]), ("--checkpoint-steps", ["2"])):
            start = arguments.index(flag) + 1
            arguments[start:start + 1] = values
        return arguments

    def test_window_resume_preserves_order_learning_curve_and_onnx_bytes(self) -> None:
        resumed = torch.load(self.resumed / "checkpoint.pt", weights_only=True)
        summary = json.loads((self.resumed / "summary.json").read_text())
        for key in ("data_order_sha256", "sample_sha256", "learning_curve", "positions_seen", "loss_sums"):
            self.assertEqual(self.state[key], resumed[key], key)
        self.assertEqual(self.summary["diagnostics"], summary["diagnostics"])
        self.assertEqual((self.continuous / "model.onnx").read_bytes(), (self.resumed / "model.onnx").read_bytes())

    def test_summary_and_checkpoint_record_sampling_contract_and_global_update_ordinals(self) -> None:
        ordered = reference_sample(146, 73, 42)
        sample_digest = hashlib.sha256(b"".join(struct.pack("<q", index) for index in ordered)).hexdigest()
        expected = {"latest_data": [str(path) for path in self.paths[1:]], "sample_count": 73,
                    "window_training_count": 146, "file_training_counts": [73, 37, 36],
                    "sample_sha256": sample_digest}
        for record in (self.state, self.summary, self.state["config"], self.summary["config"]):
            for key, value in expected.items():
                self.assertEqual(record[key], value, key)
        digest = bytes(32)
        for start in range(0, 73, 2):
            digest = hashlib.sha256(digest + b"".join(struct.pack("<q", index) for index in ordered[start:start + 2])).digest()
        self.assertEqual(self.summary["data_order_sha256"], digest.hex())
        self.assertEqual(self.summary["training_count"], 73)
        self.assertEqual(self.summary["validation_count"], 7)
        self.assertEqual(self.summary["positions_seen"], 73)
        self.assertEqual(self.summary["total_steps"], math.ceil(73 / 2))

    def test_diagnostics_use_sampled_games_and_latest_cycle_only(self) -> None:
        keys = training_keys()
        selected = [keys[index] for index in reference_sample(146, 73, 42)]
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
        changes = {"latest_data": [str(self.paths[-1])], "sample_count": 74,
                   "window_training_count": 147, "file_training_counts": [72, 38, 36],
                   "sample_sha256": "0" * 64, "seed": 43}
        for key, value in changes.items():
            with self.subTest(key=key):
                out = self.root / f"changed-{key}"
                shutil.copytree(self.partial, out)
                state = torch.load(out / "checkpoint.pt", weights_only=True)
                state["config"][key] = value
                torch.save(state, out / "checkpoint.pt")
                result = run_cli(["train", "--resume", str(out), "--device", "cpu"])
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(len(result.stderr.splitlines()), 1)
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
                self.assertEqual(len(result.stderr.splitlines()), 1)
                self.assertIn("latest-data", result.stderr)


if __name__ == "__main__":
    unittest.main()
