"""初期重みとONNXの同一性、および固定局面集の288局面の出力を検証する。"""

import hashlib
import json
import math
from pathlib import Path
import tempfile
import unittest

import torch

from tests.cli import COMMIT, ZERO, init_arguments, require_success, run_cli


FIXTURE = ZERO.parents[2] / "crates/minase/tests/fixtures/nn-positions.jsonl"


class FixtureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-fixture-test-")
        cls.addClassCleanup(directory.cleanup)
        cls.root = Path(directory.name)
        cls.initial = cls.root / "initial"
        require_success(run_cli(init_arguments(cls.initial, 23)))

    def test_init_is_complete_and_records_only_initialization_summary(self) -> None:
        state = torch.load(self.initial / "checkpoint.pt", weights_only=True)
        summary = json.loads((self.initial / "summary.json").read_text())
        self.assertIsNone(state["optimizer"])
        self.assertEqual(state["samples_seen"], 0)
        self.assertEqual(state["completed_steps"], state["total_steps"])
        self.assertEqual(set(summary), {"blocks", "channels", "seed", "parameters", "network_sha256", "provenance"})
        self.assertEqual((summary["blocks"], summary["channels"], summary["seed"]), (1, 16, 23))
        self.assertEqual(summary["provenance"]["trainer_commit"], COMMIT)
        self.assertEqual(summary["network_sha256"], hashlib.sha256((self.initial / "model.onnx").read_bytes()).hexdigest())
        repeated = self.root / "repeated"
        require_success(run_cli(init_arguments(repeated, 23)))
        self.assertEqual((repeated / "model.onnx").read_bytes(), (self.initial / "model.onnx").read_bytes())

    def test_fixture_eval_preserves_all_positions_and_legal_labels(self) -> None:
        output = self.root / "fixture.json"
        require_success(run_cli([
            "fixture-eval", "--checkpoint", str(self.initial / "checkpoint.pt"),
            "--onnx", str(self.initial / "model.onnx"),
            "--fixture", str(FIXTURE), "--out", str(output),
        ]))
        result = json.loads(output.read_text())
        fixtures = [json.loads(line) for line in FIXTURE.read_text().splitlines()]
        self.assertEqual(result["network_sha256"], hashlib.sha256((self.initial / "model.onnx").read_bytes()).hexdigest())
        self.assertEqual(result["backend"], "pytorch")
        self.assertEqual(result["batch"], 288)
        self.assertEqual(len(result["positions"]), 288)
        for actual, fixture in zip(result["positions"], fixtures):
            self.assertEqual(actual["sfen"], fixture["sfen"])
            self.assertEqual([move[0] for move in actual["moves"]], [move[4] for move in fixture["moves"]])
            self.assertTrue(0 <= actual["value"] <= 1)
            self.assertTrue(all(math.isfinite(move[1]) for move in actual["moves"]))

    def test_legacy_checkpoint_uses_regular_weights(self) -> None:
        # 移行前のチェックポイントの余分な重みは、固定局面の評価でも読み込まない。
        state = torch.load(self.initial / "checkpoint.pt", weights_only=True)
        state["ema"] = {"model": {}}
        checkpoint = self.root / "legacy-checkpoint.pt"
        torch.save(state, checkpoint)
        output = self.root / "legacy-fixture.json"
        require_success(run_cli([
            "fixture-eval", "--checkpoint", str(checkpoint), "--onnx", str(self.initial / "model.onnx"),
            "--fixture", str(FIXTURE), "--out", str(output),
        ]))
        result = json.loads(output.read_text())
        self.assertEqual(result["network_sha256"], hashlib.sha256((self.initial / "model.onnx").read_bytes()).hexdigest())
        self.assertEqual(len(result["positions"]), 288)

    def test_other_weights_are_rejected_before_output(self) -> None:
        different = self.root / "different"
        require_success(run_cli(init_arguments(different, 24)))
        output = self.root / "mismatch.json"
        result = run_cli([
            "fixture-eval", "--checkpoint", str(self.initial / "checkpoint.pt"),
            "--onnx", str(different / "model.onnx"),
            "--fixture", str(FIXTURE), "--out", str(output),
        ])
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)
        self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
