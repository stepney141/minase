"""複数GPUの設計書の比較規則を、損失の定義、解析例、および実ファイルで検証する。"""

from contextlib import redirect_stderr
import hashlib
import importlib.util
import io
import json
import math
from pathlib import Path
import shutil
import struct
import sys
import tempfile
import unittest

import numpy as np
import torch

from minase_zero.data import read_file
from minase_zero.encoding import policy_mask
from minase_zero.train import _load_checkpoint, _model, compute_losses, diagnose
from tests.cli import ZERO, init_arguments, require_success, run_cli, run_command
from tests.test_data import FIXTURE, single_game, write_small


SCRIPT = ZERO.parents[2] / "docs" / "measurements" / "alphazero-multi-gpu" / "compare-validation.py"
SPEC = importlib.util.spec_from_file_location("compare_validation", SCRIPT)
compare = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(compare)


class PositionLossTests(unittest.TestCase):
    def test_position_means_match_trainer_and_cross_entropy_definition(self) -> None:
        policy_logits = torch.tensor([
            [math.log(2), 1000, 0, 0, -1000],
            [0, 1000, 0, math.log(2), 1000],
            [0, 1000, 0, 0, 1000],
        ], dtype=torch.float32)
        value_logits = torch.tensor([0, math.log(3), -math.log(3)], dtype=torch.float32)
        rows = torch.tensor([0, 0, 0, 1, 2, 2])
        labels = torch.tensor([0, 2, 3, 2, 0, 3])
        probabilities = torch.tensor([32768, 16384, 16383, 65535, 0, 65535])
        results = torch.tensor([1, 0.5, 0], dtype=torch.float32)
        values = torch.tensor([0, 32768, 65535])
        mask = torch.tensor([True, False, True, True, False])
        # 定数マスク内の確率は、順に(1/2,1/4,1/4)、(1/4,1/4,1/2)、(1/3,1/3,1/3)。
        expected_policy = torch.tensor([
            (32768 * math.log(2) + 32767 * math.log(4)) / 65535,
            math.log(4), math.log(3),
        ], dtype=torch.float32)
        for value_lambda in (0.0, 0.333, 1.0):
            with self.subTest(value_lambda=value_lambda):
                args = (policy_logits, value_logits, rows, labels, probabilities,
                        results, values, value_lambda, mask)
                policy, value = compare.per_position_losses(*args)
                trainer_policy, trainer_value, _ = compute_losses(*args)
                torch.testing.assert_close(policy.mean(), trainer_policy)
                torch.testing.assert_close(value.mean(), trainer_value)
                torch.testing.assert_close(policy, expected_policy)
                targets = [(1 - value_lambda) * result + value_lambda * root / 65535
                           for result, root in ((1, 0), (0.5, 32768), (0, 65535))]
                expected_value = torch.tensor([
                    -target * math.log(probability) - (1 - target) * math.log(1 - probability)
                    for target, probability in zip(targets, (0.5, 0.75, 0.25), strict=True)
                ], dtype=torch.float32)
                torch.testing.assert_close(value, expected_value)

    def test_policy_target_outside_constant_mask_is_rejected(self) -> None:
        for label in (-1, 1, 3):
            with self.subTest(label=label), self.assertRaises(ValueError):
                compare.per_position_losses(
                    torch.zeros(1, 3), torch.zeros(1), torch.tensor([0]), torch.tensor([label]),
                    torch.tensor([65535]), torch.tensor([1.0]), torch.tensor([32768]), 0.333,
                    torch.tensor([True, False, True]),
                )


class BootstrapTests(unittest.TestCase):
    def setUp(self) -> None:
        self.reference = np.array([[10, 2], [12, 6]], dtype=np.float64)
        self.candidate = np.array([[12, 1], [9, 9]], dtype=np.float64)
        self.counts = np.array([1, 3], dtype=np.int64)

    def test_position_weighted_paired_estimate_and_seed_reproducibility(self) -> None:
        first = compare.paired_game_bootstrap(self.reference, self.candidate, self.counts, 2000, 42)
        second = compare.paired_game_bootstrap(self.reference, self.candidate, self.counts, 2000, 42)
        self.assertEqual(first, second)
        self.assertEqual(first["game_count"], 2)
        self.assertEqual(first["position_count"], 4)
        self.assertEqual(first["reference"], {"policy": 5.5, "value": 2.0})
        self.assertEqual(first["candidate"], {"policy": 5.25, "value": 2.5})
        # 各対局の差の和は(2,-1)と(-3,3)で、局面数は1と3。
        # 復元抽出の4通りは、差が(2,-1)、(-1/4,1/2)、(-1/4,1/2)、(-1,1)。
        self.assertEqual(first["difference"]["policy"], {"estimate": -0.25, "ci95": [-1.0, 2.0]})
        self.assertEqual(first["difference"]["value"], {"estimate": 0.5, "ci95": [-1.0, 1.0]})
        self.assertEqual(first["decision"], "判定できない")
        # 対局ごとの共通の損失を加えても、対応を保った差は変わらない。
        shared = np.array([[1000, 100], [2000, 4000]], dtype=np.float64)
        shifted = compare.paired_game_bootstrap(
            self.reference + shared, self.candidate + shared, self.counts, 2000, 42,
        )
        self.assertEqual(shifted["difference"], first["difference"])

    def test_percentiles_use_linear_interpolation_and_point_estimate_uses_all_games(self) -> None:
        # PCG64(0)の最初の4個の[0,2)整数は1,1,1,0で、標本の差は(-1,1)と(-1/4,1/2)。
        result = compare.paired_game_bootstrap(self.reference, self.candidate, self.counts, 2, 0)
        np.testing.assert_allclose(result["difference"]["policy"]["ci95"], [-0.98125, -0.26875])
        np.testing.assert_allclose(result["difference"]["value"]["ci95"], [0.5125, 0.9875])
        self.assertEqual(result["difference"]["policy"]["estimate"], -0.25)
        self.assertEqual(result["difference"]["value"]["estimate"], 0.5)

    def test_noninferiority_includes_zero_and_either_positive_lower_bound_is_inferior(self) -> None:
        for difference, decision in (
            ((0, 0), "候補は劣らない"), ((-0.5, 0), "候補は劣らない"),
            ((0, -0.5), "候補は劣らない"), ((0.125, -0.5), "候補は劣る"),
            ((-0.5, 0.125), "候補は劣る"),
        ):
            with self.subTest(difference=difference):
                reference = np.array([[2, 2]], dtype=np.float64)
                result = compare.paired_game_bootstrap(
                    reference, reference + difference, np.array([1]), 10, 0,
                )
                self.assertEqual(result["decision"], decision)

    def test_zero_lower_bound_with_positive_upper_bound_is_inconclusive(self) -> None:
        for changed in (0, 1):
            with self.subTest(changed=changed):
                reference = np.ones((2, 2), dtype=np.float64)
                candidate = reference.copy()
                candidate[1, changed] += 1
                result = compare.paired_game_bootstrap(reference, candidate, np.ones(2, dtype=np.int64), 2000, 42)
                self.assertEqual(result["difference"][("policy", "value")[changed]]["ci95"], [0.0, 1.0])
                self.assertEqual(result["decision"], "判定できない")

    def test_empty_invalid_or_nonfinite_statistics_are_rejected(self) -> None:
        for reference, candidate, counts, resamples, seed in (
            (self.reference[:0], self.candidate[:0], self.counts[:0], 10, 0),
            (self.reference, self.candidate, np.array([0, 3]), 10, 0),
            (self.reference, self.candidate, np.array([1.5, 3.0]), 10, 0),
            (self.reference, self.candidate[:1], self.counts, 10, 0),
            (self.reference * np.nan, self.candidate, self.counts, 10, 0),
            (self.reference, self.candidate * np.inf, self.counts, 10, 0),
            (self.reference, self.candidate, self.counts, 0, 0),
            (self.reference, self.candidate, self.counts, 10, -1),
        ):
            with self.subTest(counts=counts, resamples=resamples, seed=seed), self.assertRaises(ValueError):
                compare.paired_game_bootstrap(reference, candidate, counts, resamples, seed)


class ArgumentTests(unittest.TestCase):
    def setUp(self) -> None:
        self.options = {
            "--reference": "a.pt", "--candidate": "b.pt", "--data": "data.mnsd",
            "--value-lambda": "0.333", "--batch": "16", "--resamples": "100",
            "--seed": "42", "--device": "cpu", "--out": "out.json",
        }

    def assert_invalid(self, options: dict[str, str]) -> None:
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            compare.parse_args([f"{name}={value}" for name, value in options.items()])
        self.assertEqual(error.exception.code, 2)

    def test_every_argument_is_required(self) -> None:
        for omitted in self.options:
            with self.subTest(omitted=omitted):
                self.assert_invalid({name: value for name, value in self.options.items() if name != omitted})

    def test_invalid_counts_seed_lambda_and_device_are_rejected(self) -> None:
        for option, value in (
            ("--batch", "0"), ("--batch", "-1"), ("--resamples", "0"), ("--seed", "-1"),
            ("--value-lambda", "-0.1"), ("--value-lambda", "1.1"), ("--value-lambda", "nan"),
            ("--value-lambda", "inf"), ("--device", "mps"),
        ):
            with self.subTest(option=option, value=value):
                self.assert_invalid({**self.options, option: value})


class ComparisonFileTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        threads = torch.get_num_threads()
        cls.addClassCleanup(torch.set_num_threads, threads)
        torch.set_num_threads(1)
        directory = tempfile.TemporaryDirectory(prefix="minase-compare-validation-")
        cls.addClassCleanup(directory.cleanup)
        cls.root = Path(directory.name)
        cls.reference = cls.root / "reference" / "checkpoint.pt"
        cls.candidate = cls.root / "candidate" / "checkpoint.pt"
        for checkpoint, seed in ((cls.reference, 7), (cls.candidate, 11)):
            require_success(run_cli(init_arguments(checkpoint.parent, seed)))

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(dir=self.root)
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)
        self.out = self.directory / "comparison.json"

    def arguments(self, paths: list[Path], candidate: Path) -> list[str]:
        return [
            "--reference", str(self.reference), "--candidate", str(candidate),
            "--data", *(str(path) for path in paths), "--value-lambda", "0.333", "--batch", "17",
            "--resamples", "1000", "--seed", "42", "--device", "cpu", "--out", str(self.out),
        ]

    def test_cpu_cli_with_two_initialized_networks_writes_means_matching_diagnostics(self) -> None:
        hashes = [hashlib.sha256(path.read_bytes()).hexdigest() for path in (self.reference, self.candidate)]
        require_success(run_command([sys.executable, str(SCRIPT), *self.arguments([FIXTURE], self.candidate)]))
        report = json.loads(self.out.read_text(encoding="utf-8"))
        # 固定データの741採用局面のうち643局面が訓練側で、検証側は4局98局面。
        self.assertEqual((report["game_count"], report["position_count"]), (4, 98))
        self.assertEqual((report["resamples"], report["seed"]), (1000, 42))
        self.assertEqual((report["value_lambda"], report["batch"], report["device"]), (0.333, 17, "cpu"))
        data, _ = read_file(FIXTURE)
        mask = torch.from_numpy(policy_mask())
        for name, path, checksum in zip(("reference", "candidate"), (self.reference, self.candidate), hashes, strict=True):
            self.assertEqual(report[name]["path"], str(path))
            self.assertEqual(report[name]["sha256"], checksum)
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), checksum)
            expected = diagnose(
                _model(_load_checkpoint(path)), data, np.flatnonzero(data.validation), torch.device("cpu"),
                {"precision": "fp32", "value_lambda": 0.333, "batch_size": 17}, mask, auxiliary=False,
            )
            for loss in ("policy", "value"):
                torch.testing.assert_close(torch.tensor(report[name]["mean_losses"][loss]), torch.tensor(expected[loss]))
        for loss in ("policy", "value"):
            expected = report["candidate"]["mean_losses"][loss] - report["reference"]["mean_losses"][loss]
            self.assertAlmostEqual(report["difference"][loss]["estimate"], expected, places=14)
            self.assertTrue(all(math.isfinite(value) for value in report["difference"][loss]["ci95"]))

    def test_identical_checkpoint_has_exactly_zero_difference_and_is_noninferior(self) -> None:
        self.assertEqual(compare.main(self.arguments([FIXTURE], self.reference)), 0)
        report = json.loads(self.out.read_text(encoding="utf-8"))
        self.assertEqual(report["reference"], report["candidate"])
        for loss in ("policy", "value"):
            self.assertEqual(report["difference"][loss], {"estimate": 0.0, "ci95": [0.0, 0.0]})
        self.assertEqual(report["decision"], "候補は劣らない")

    def test_all_files_use_their_header_seed_and_only_trainer_selected_validation_positions(self) -> None:
        paths = [self.directory / f"part-{index}.mnsd" for index in range(3)]
        # Rustの既知の分割ではseed=11の4,15,22とseed=22の6が検証側である。
        # 基本シードが同じでも番号が違う対局は別の対局で、別のシードなら番号を再利用できる。
        for path, seed, games in (
            (paths[0], 11, [single_game(4, 0, 0, 123, (65535,)),
                            single_game(6, 0, 0, 123, (65535,)), single_game(15, 0, 0, None, ())]),
            (paths[1], 22, [single_game(4, 1, 1, 456, (65535,)), single_game(6, 1, 1, 456, (65535,))]),
            (paths[2], 11, [single_game(22, 0, 2, 32768, (65535,))]),
        ):
            write_small(path, games, "engine-default")
            content = bytearray(path.read_bytes())
            struct.pack_into("<Q", content, 128, seed)
            path.write_bytes(content)
        self.assertEqual(compare.main(self.arguments(paths, self.candidate)), 0)
        report = json.loads(self.out.read_text(encoding="utf-8"))
        self.assertEqual((report["game_count"], report["position_count"]), (3, 3))
        self.assertEqual([item["seed"] for item in report["data"]], [11, 22, 11])
        self.assertEqual([item["position_count"] for item in report["data"]], [1, 1, 1])

    def test_duplicate_paths_or_games_and_empty_validation_fail_without_output(self) -> None:
        copied = self.directory / "copy.mnsd"
        shutil.copyfile(FIXTURE, copied)
        empty = self.directory / "empty.mnsd"
        write_small(empty, [], "engine-default")
        training = self.directory / "training.mnsd"
        write_small(training, [single_game(3, 0, 0, 123, (65535,))], "engine-default")
        alias = self.directory / "alias.mnsd"
        alias.symlink_to(FIXTURE)
        for paths in ([FIXTURE, alias], [FIXTURE, copied], [empty], [training]):
            with self.subTest(paths=paths), redirect_stderr(io.StringIO()):
                self.assertEqual(compare.main(self.arguments(paths, self.candidate)), 1)
                self.assertFalse(self.out.exists())

    def test_nonfinite_network_losses_fail_without_output(self) -> None:
        checkpoint = self.directory / "nonfinite.pt"
        state = _load_checkpoint(self.candidate)
        state["model"]["policy_head.bias"].fill_(torch.nan)
        torch.save(state, checkpoint)
        with redirect_stderr(io.StringIO()):
            self.assertEqual(compare.main(self.arguments([FIXTURE], checkpoint)), 1)
        self.assertFalse(self.out.exists())

    def test_existing_output_is_preserved(self) -> None:
        self.out.write_text("existing result\n", encoding="utf-8")
        with redirect_stderr(io.StringIO()):
            self.assertEqual(compare.main(self.arguments([FIXTURE], self.candidate)), 1)
        self.assertEqual(self.out.read_text(encoding="utf-8"), "existing result\n")


if __name__ == "__main__":
    unittest.main()
