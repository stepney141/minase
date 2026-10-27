"""独立に組み立てた推論結果で、方策と価値の照合条件を検証する。"""

from copy import deepcopy
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ZERO = Path(__file__).resolve().parents[1]


def inference_result(logits: list[list[float]], values: list[float]) -> dict:
    return {
        "network_sha256": "0123456789abcdef" * 4,
        "positions": [
            {
                "sfen": f"12/12/12/12/12/12/12/12/12/12/12/12 b - {index + 1}",
                "value": value,
                "moves": [[label, logit] for label, logit in enumerate(position_logits)],
            }
            for index, (position_logits, value) in enumerate(zip(logits, values, strict=True))
        ],
    }


class CompareTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-nn-compare-test-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def run_cli(self, reference: dict, candidate: dict, arguments: list[str]) -> subprocess.CompletedProcess[str]:
        reference_path = self.root / "reference.json"
        candidate_path = self.root / "candidate.json"
        reference_path.write_text(json.dumps(reference), encoding="utf-8")
        candidate_path.write_text(json.dumps(candidate), encoding="utf-8")
        return subprocess.run(
            [sys.executable, str(ZERO / "nn_compare.py"),
             "--reference", str(reference_path), "--candidate", str(candidate_path), *arguments],
            capture_output=True, text=True, timeout=10,
        )

    def compare(
        self, reference: dict, candidate: dict, policy: str,
        policy_tolerance: str, value_tolerance: str,
    ) -> tuple[subprocess.CompletedProcess[str], dict]:
        result = self.run_cli(reference, candidate, [
            "--policy", policy, "--policy-tolerance", policy_tolerance,
            "--value-tolerance", value_tolerance,
        ])
        self.assertEqual(len(result.stdout.splitlines()), 1, result.stderr)
        return result, json.loads(result.stdout)

    def test_proportional_error_passes_probabilities_but_fails_logits(self) -> None:
        reference = inference_result([[12.0, 10.0, 8.0]], [0.5])
        candidate = inference_result([[logit * 1.003 for logit in (12.0, 10.0, 8.0)]], [0.5])
        for policy, expected_code in (("probabilities", 0), ("logits", 1)):
            with self.subTest(policy=policy):
                result, summary = self.compare(reference, candidate, policy, "1e-2", "0")
                self.assertEqual(result.returncode, expected_code, result.stderr)
                self.assertAlmostEqual(summary["max_logit_abs_diff"], 0.036, places=14)
                self.assertLess(summary["max_probability_abs_diff"], 1e-2)

    def test_adjacent_move_logits_are_rejected_in_probability_mode(self) -> None:
        reference = inference_result([[8.0, 1.0, 0.0]], [0.5])
        candidate = inference_result([[1.0, 0.0, 8.0]], [0.5])
        result, summary = self.compare(reference, candidate, "probabilities", "1e-2", "0")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertGreater(summary["max_probability_abs_diff"], 0.9)

    def test_value_tolerance_is_independent_of_policy_tolerance(self) -> None:
        reference = inference_result([[2.0, 1.0]], [0.0])
        candidate = inference_result([[2.0, 1.0]], [0.015])
        for policy in ("logits", "probabilities"):
            for tolerance, expected_code in (("2e-2", 0), ("1e-2", 1)):
                with self.subTest(policy=policy, tolerance=tolerance):
                    result, summary = self.compare(reference, candidate, policy, "0", tolerance)
                    self.assertEqual(result.returncode, expected_code, result.stderr)
                    self.assertAlmostEqual(summary["max_value_abs_diff"], 0.015)
                    self.assertEqual(summary["max_probability_abs_diff"], 0.0)

    def test_probability_difference_matches_independent_softmax_in_both_modes(self) -> None:
        reference_logits = [[2.0, -1.0, 0.0], [10.0, 8.0]]
        candidate_logits = [[1.75, -0.5, 0.25], [10.0, 8.5]]
        # 小さな入力では、最大値を引かずに定義式から参照確率を計算できる。
        differences = []
        for left, right in zip(reference_logits, candidate_logits, strict=True):
            left_weights = [math.exp(logit) for logit in left]
            right_weights = [math.exp(logit) for logit in right]
            differences.extend(
                abs(left_weight / math.fsum(left_weights) - right_weight / math.fsum(right_weights))
                for left_weight, right_weight in zip(left_weights, right_weights, strict=True)
            )
        expected = max(differences)
        reference = inference_result(reference_logits, [0.25, 0.75])
        candidate = inference_result(candidate_logits, [0.25, 0.75])
        for policy in ("logits", "probabilities"):
            with self.subTest(policy=policy):
                result, summary = self.compare(reference, candidate, policy, "1", "0")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(summary["positions"], 2)
                self.assertAlmostEqual(summary["max_probability_abs_diff"], expected, delta=1e-15)

    def test_softmax_is_stable_and_invariant_under_each_positions_constant_shift(self) -> None:
        reference = inference_result([[1000.0, 1001.0, 999.0], [-1000.0, -999.0]], [0.25, 0.75])
        candidate = inference_result([[-1000.0, -999.0, -1001.0], [1000.0, 1001.0]], [0.25, 0.75])
        result, summary = self.compare(reference, candidate, "probabilities", "0", "0")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(summary["max_probability_abs_diff"], 0.0)
        self.assertEqual(summary["max_logit_abs_diff"], 2000.0)

    def test_tolerance_boundary_is_inclusive(self) -> None:
        reference = inference_result([[0.0, 0.0]], [0.0])
        for policy, logits, tolerance in (
            ("logits", [[0.125, 0.0]], 0.125),
            ("probabilities", [[0.0, -1000.0]], 0.5),
        ):
            candidate = inference_result(logits, [0.25])
            for limit, expected_code in ((tolerance, 0), (math.nextafter(tolerance, 0.0), 1)):
                with self.subTest(policy=policy, limit=limit):
                    result, _ = self.compare(reference, candidate, policy, str(limit), "0.25")
                    self.assertEqual(result.returncode, expected_code, result.stderr)

    def test_best_move_mismatch_is_reported_without_failing(self) -> None:
        reference = inference_result([[0.0, 0.001]], [0.5])
        candidate = inference_result([[0.001, 0.0]], [0.5])
        for policy in ("logits", "probabilities"):
            with self.subTest(policy=policy):
                result, summary = self.compare(reference, candidate, policy, "1e-2", "0")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(summary["best_move_mismatches"], 1)
                self.assertEqual(summary["best_move_mismatch_gaps"], [[0, 0.001]])

    def test_zero_tolerances_accept_identical_outputs_including_empty_moves(self) -> None:
        reference = inference_result([[2.0, 1.0], []], [0.5, 1.0])
        for policy in ("logits", "probabilities"):
            with self.subTest(policy=policy):
                result, summary = self.compare(reference, reference, policy, "0", "0")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(summary["max_probability_abs_diff"], 0.0)

    def test_each_comparison_option_is_required(self) -> None:
        reference = inference_result([[0.0]], [0.5])
        options = ["--policy", "probabilities", "--policy-tolerance", "0.01", "--value-tolerance", "0.02"]
        for index in (0, 2, 4):
            with self.subTest(missing=options[index]):
                result = self.run_cli(reference, reference, options[:index] + options[index + 2:])
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn(options[index], result.stderr)

    def test_old_tolerance_option_is_rejected(self) -> None:
        reference = inference_result([[0.0]], [0.5])
        for options in (
            [],
            ["--policy", "logits", "--policy-tolerance", "0.01", "--value-tolerance", "0.02"],
        ):
            with self.subTest(options=options):
                result = self.run_cli(reference, reference, [*options, "--tolerance", "0.01"])
                self.assertEqual(result.returncode, 2, result.stderr)

    def test_invalid_policy_is_rejected(self) -> None:
        reference = inference_result([[0.0]], [0.5])
        result = self.run_cli(reference, reference, [
            "--policy", "invalid", "--policy-tolerance", "0.01", "--value-tolerance", "0.02",
        ])
        self.assertEqual(result.returncode, 2, result.stderr)

    def test_negative_and_nonfinite_tolerances_are_argument_errors(self) -> None:
        reference = inference_result([[0.0]], [0.5])
        for option in ("--policy-tolerance", "--value-tolerance"):
            for invalid in ("-0.01", "nan", "inf", "-inf"):
                with self.subTest(option=option, invalid=invalid):
                    options = {"--policy": "probabilities", "--policy-tolerance": "0", "--value-tolerance": "0"}
                    options[option] = invalid
                    result = self.run_cli(reference, reference, [f"{key}={value}" for key, value in options.items()])
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertIn(option, result.stderr)

    def test_identity_mismatches_are_rejected_even_with_matching_values(self) -> None:
        reference = inference_result([[0.0, 0.0]], [0.5])
        for field in ("network_sha256", "sfen", "labels"):
            with self.subTest(field=field):
                candidate = deepcopy(reference)
                if field == "network_sha256":
                    candidate[field] = "f" * 64
                elif field == "sfen":
                    candidate["positions"][0][field] = "12/12/12/12/12/12/12/12/12/12/12/12 w - 1"
                else:
                    candidate["positions"][0]["moves"].reverse()
                result, _ = self.compare(reference, candidate, "probabilities", "1", "1")
                self.assertEqual(result.returncode, 1, result.stderr)

    def test_incomparable_policy_metrics_are_null(self) -> None:
        reference = inference_result([[0.0, 1.0], [2.0, 3.0]], [0.25, 0.75])
        for mismatch in ("positions", "moves"):
            with self.subTest(mismatch=mismatch):
                candidate = deepcopy(reference)
                if mismatch == "positions":
                    candidate["positions"].pop()
                else:
                    candidate["positions"][0]["moves"].pop()
                result, summary = self.compare(reference, candidate, "probabilities", "1", "1")
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIsNone(summary["max_logit_abs_diff"])
                self.assertIsNone(summary["max_probability_abs_diff"])

    def test_nonfinite_inputs_are_rejected_and_only_affected_metrics_are_null(self) -> None:
        for side in ("reference", "candidate"):
            for field in ("logit", "value"):
                for invalid in (math.nan, math.inf, -math.inf):
                    with self.subTest(side=side, field=field, invalid=invalid):
                        reference = inference_result([[0.0, 1.0]], [0.5])
                        candidate = deepcopy(reference)
                        position = (reference if side == "reference" else candidate)["positions"][0]
                        if field == "logit":
                            position["moves"][0][1] = invalid
                        else:
                            position["value"] = invalid
                        result, summary = self.compare(reference, candidate, "probabilities", "1", "1")
                        self.assertEqual(result.returncode, 1, result.stderr)
                        if field == "logit":
                            self.assertIsNone(summary["max_logit_abs_diff"])
                            self.assertIsNone(summary["max_probability_abs_diff"])
                            self.assertEqual(summary["max_value_abs_diff"], 0.0)
                        else:
                            self.assertIsNone(summary["max_value_abs_diff"])
                            self.assertEqual(summary["max_probability_abs_diff"], 0.0)

    def test_invalid_result_structure_still_reports_probability_metric(self) -> None:
        reference = inference_result([[0.0]], [0.5])
        candidate = inference_result([], [])
        result, summary = self.compare(reference, candidate, "probabilities", "1", "1")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIsNone(summary["max_probability_abs_diff"])


if __name__ == "__main__":
    unittest.main()
