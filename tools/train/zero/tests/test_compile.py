"""順伝播のコンパイル、保存形式、および勾配の比較を指示書の条件から検証する。

1サイクルの学習と最終診断を終えてもコンパイルは1回で、保存する鍵はinitと一致する。
CPUでの比較は4通りともfp32なので、コンパイルの有無が同じ2つの勾配は等しくなる。
"""

import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import torch
import torch._inductor.config

from minase_zero.net import ZeroNet
from minase_zero.train import compile_training, compiled_forward_context

from tests.cli import ZERO, environment, require_success, run_command
from tests.test_replay_buffer import reference_sample


SCRIPT = ZERO.parents[2] / "docs/measurements/alphazero-multi-gpu/compile-gradients.py"


class CompilationSettingsTests(unittest.TestCase):
    def test_compilation_keeps_rounding_and_determinism_enabled_for_later_backward(self):
        with torch._inductor.config.patch(emulate_precision_casts=False, deterministic=False):
            compile_training(ZeroNet(1, 4))
            self.assertIs(torch._inductor.config.emulate_precision_casts, True)
            self.assertIs(torch._inductor.config.deterministic, True)
            with compiled_forward_context():
                self.assertIs(torch._inductor.config.emulate_precision_casts, True)
                self.assertIs(torch._inductor.config.deterministic, True)
            # 逆伝播は順伝播の文脈を抜けてから初めてコンパイルされる。
            self.assertIs(torch._inductor.config.emulate_precision_casts, True)
            self.assertIs(torch._inductor.config.deterministic, True)


class CompileTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        directory = tempfile.TemporaryDirectory(prefix="minase-compile-")
        cls.addClassCleanup(directory.cleanup)
        cls.root = Path(directory.name)
        require_success(run_command([sys.executable, "-m", "tests.compile_worker", str(cls.root)]))

    def test_one_graph_serves_a_complete_cycle_and_diagnostics(self):
        counts = json.loads((self.root / "compile-count.json").read_text())
        self.assertEqual(counts["unique_graphs"], 1)
        self.assertEqual(counts["graph_breaks"], 0)
        self.assertIs(counts["reapply_compile"], False)
        self.assertIs(counts["emulate_precision_casts"], True)
        # 順伝播と遅延コンパイルされる逆伝播の両方で、両設定が有効である。
        expected_settings = {"deterministic": True, "emulate_precision_casts": True}
        self.assertEqual(counts["compilation_settings"], [expected_settings, expected_settings])
        summary = json.loads((self.root / "trained/summary.json").read_text())
        # このfixtureの抽出数は2572であり、全体のバッチ16の端数12局面を捨てる。
        self.assertEqual(summary["sample_count"], 2572)
        self.assertEqual(summary["completed_steps"], 160)
        self.assertEqual(summary["total_steps"], 160)
        self.assertEqual(summary["positions_seen"], 2560)
        self.assertEqual(set(summary["diagnostics"]),
                         {"training_sample", "latest_training_sample", "validation"})
        self.assertTrue((self.root / "trained/model.onnx").is_file())

    def test_checkpoint_uses_the_original_network_keys_and_updated_weights(self):
        initial = torch.load(self.root / "initial/checkpoint.pt", weights_only=True)["model"]
        trained = torch.load(self.root / "trained/checkpoint.pt", weights_only=True)["model"]
        self.assertEqual(set(trained), set(initial))
        self.assertTrue(any(not torch.equal(trained[key], initial[key]) for key in initial))
        for key in initial:
            with self.subTest(key=key):
                self.assertEqual(trained[key].shape, initial[key].shape)
                if key.endswith("num_batches_tracked"):
                    self.assertEqual(trained[key].item(), 160)

    def test_gradient_tool_writes_cpu_results_from_init_and_an_external_directory(self):
        out = self.root / "gradients.json"
        # プロジェクト外から起動し、uvに学習器の作業ディレクトリを指定する。
        result = subprocess.run([
            "uv", "run", "--frozen", "--directory", str(ZERO), "python", str(SCRIPT),
            "--checkpoint", str(self.root / "initial/checkpoint.pt"),
            "--data", str(ZERO / "fixtures/zero-selfplay-small.mnsd"),
            "--micro-batch-size", "4", "--seed", "42", "--device", "cpu", "--out", str(out),
        ], cwd=self.root, env=environment(), capture_output=True, text=True, timeout=180)
        require_success(result)
        actual = json.loads(out.read_text())
        self.assertEqual(actual["device"], "cpu")
        self.assertEqual(actual["precision"], "fp32")
        self.assertTrue(actual["cpu_smoke_test"])
        self.assertIn("all four runs use fp32", actual["precision_note"])
        self.assertEqual(actual["micro_batch_size"], 4)
        self.assertEqual(actual["seed"], 42)
        self.assertEqual(actual["sample_indices"], reference_sample(643, 2572, 42)[:4])
        self.assertEqual(actual["pytorch_version"], str(torch.__version__))
        self.assertIs(actual["emulate_precision_casts"], True)
        self.assertIs(actual["deterministic"], True)
        self.assertTrue(actual["device_name"])
        self.assertEqual(actual["value_lambda"], 0.333)
        self.assertEqual(actual["value_lambda_source"], "init measurement condition")
        self.assertEqual(set(actual["gradient_norms"]), {"eager", "compiled", "eager_fp32", "compiled_fp32"})
        for norm in actual["gradient_norms"].values():
            self.assertTrue(math.isfinite(norm))
            self.assertGreater(norm, 0)
        self.assertEqual(actual["gradient_norms"]["eager"], actual["gradient_norms"]["eager_fp32"])
        self.assertEqual(actual["gradient_norms"]["compiled"], actual["gradient_norms"]["compiled_fp32"])
        self.assertEqual(actual["relative_error_bf16_fp32"], 0)
        self.assertTrue(math.isfinite(actual["relative_error_compile"]))
        self.assertGreaterEqual(actual["relative_error_compile"], 0)
        self.assertEqual(actual["relative_error_compile_fp32"], actual["relative_error_compile"])
        self.assertEqual(actual["acceptance_condition"], "relative_error_compile < relative_error_bf16_fp32")
        self.assertFalse(actual["acceptance_applicable"])
        self.assertFalse(actual["acceptance_passed"])


if __name__ == "__main__":
    unittest.main()
