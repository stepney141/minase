"""明示した設定と完了記録だけで学習工程を進める契約を検証する。"""

from contextlib import ExitStack
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

from features import FEATURE_COUNT
import pst_workflow as workflow
from test_pst_diagnostics import python_probe
from test_train_pst import write_mnsd
from train_pst import initial_piece_values, read_mnpt, write_mnpt


CONFIG = '''[run]
directory = "data/run"
base_commit = "0000000000000000000000000000000000000000"
data = ["data/old.bin"]
[generate]
seeds = [100, 110]
games = 10
nodes = 100000
concurrency = 1
max_ply = 600
hash_mb = 16
random_moves = 0
[train]
model = "single"
learning_rate = 3
epochs = 10
batch = 16384
seed = 1
lambda = 0.75
device = "cpu"
validation_sample = 10000
[diagnose]
sample_size = 10000
seed = 1
'''


class WorkflowTest(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / "data").mkdir()
        self.config_path = self.root / "config.toml"
        self.config_path.write_text(CONFIG)
        self.config = workflow.load_config(self.config_path, self.root)
        write_mnsd(self.root / "data/old.bin", seed=0, checksum=b"a" * 32,
                   games=[1, 10])

    def test_explicit_config_resolves_paths_and_accepts_touching_seed_ranges(self) -> None:
        self.assertEqual(self.config["run"]["directory"], str(self.root / "data/run"))
        self.assertEqual(self.config["run"]["data"], [str(self.root / "data/old.bin")])
        self.assertEqual(self.config["generate"]["seeds"], [100, 110])
        self.assertEqual(self.config["train"]["device"], "cpu")
        self.assertEqual(self.config["train"]["model"], "single")
        # 生成シードの空配列は、既存データだけで学習する明示的な指定として受理する。
        self.config_path.write_text(CONFIG.replace("seeds = [100, 110]", "seeds = []"))
        self.assertEqual(workflow.load_config(self.config_path, self.root)["generate"]["seeds"], [])

    def test_missing_and_unknown_fields_are_rejected(self) -> None:
        for content in (CONFIG.replace("epochs = 10\n", ""),
                        CONFIG.replace("epochs = 10", "epochs = 10\nepoch = 10"),
                        CONFIG + "\n[extra]\nvalue = 1\n"):
            with self.subTest(content=content):
                self.config_path.write_text(content)
                with self.assertRaises(ValueError):
                    workflow.load_config(self.config_path, self.root)

    def test_invalid_numbers_seed_ranges_and_paths_are_rejected(self) -> None:
        changes = [
            ("games = 10", "games = 0"),
            ("epochs = 10", "epochs = true"),
            ("batch = 16384", "batch = -1"),
            ("learning_rate = 3", "learning_rate = nan"),
            ("learning_rate = 3", "learning_rate = inf"),
            ("learning_rate = 3", "learning_rate = 0"),
            ('model = "single"', 'model = "dual"'),
            ("lambda = 0.75", "lambda = 1.01"),
            ("lambda = 0.75", "lambda = -0.01"),
            ("sample_size = 10000", "sample_size = 0"),
            ("seeds = [100, 110]", "seeds = [100, 100]"),
            ("seeds = [100, 110]", "seeds = [100, 109]"),
            ("seeds = [100, 110]", "seeds = [18446744073709551615]"),
            ('directory = "data/run"', 'directory = "../outside"'),
            ('data = ["data/old.bin"]', 'data = ["data/old.bin", "data/../data/old.bin"]'),
        ]
        for before, after in changes:
            with self.subTest(after=after):
                self.config_path.write_text(CONFIG.replace(before, after))
                with self.assertRaises(ValueError):
                    workflow.load_config(self.config_path, self.root)

    def test_new_seed_ranges_must_not_overlap_recorded_history(self) -> None:
        self.config["generate"]["seeds"] = [10]
        files = workflow.check_existing_data(self.config)
        self.assertEqual(len(files), 1)
        self.assertEqual(files[0]["sha256"], hashlib.sha256(
            (self.root / "data/old.bin").read_bytes()).hexdigest())
        self.config["generate"]["seeds"] = [9]
        with self.assertRaises(ValueError):
            workflow.check_existing_data(self.config)

    def prepared(self) -> tuple[Path, ExitStack]:
        """外部git操作だけを置換し、完了記録とファイル検証は実際に通す。"""
        run = self.root / "data/run"
        run.mkdir()
        binary = run / "generator/target/release/selfplay_gen"
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b"fixture executable")
        probe = run / "generator/target/release/pst_probe"
        probe.write_bytes(b"fixture probe")
        zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
        write_mnpt(run / "pst-base.bin", zeros, zeros, initial_piece_values(), 1000)
        state = {
            "config": self.config,
            "existing_data": workflow.check_existing_data(self.config),
            "sources": workflow.source_hashes(),
            "base_sha256": workflow.digest(run / "pst-base.bin"),
            "base_piece_values_sha256": hashlib.sha256(
                (run / "pst-base.bin").read_bytes()[-workflow.PIECE_VALUE_BYTES:]).hexdigest(),
            "generator_sha256": workflow.digest(binary),
            "probe_sha256": workflow.digest(probe),
            "repository": str(self.root),
        }
        (run / "prepared.json").write_text(json.dumps(state))
        stack = ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(patch.object(workflow, "ROOT", self.root))
        stack.enter_context(patch.object(workflow, "git", side_effect=lambda repo, *args:
                                        "0" * 40 if args == ("rev-parse", "HEAD") else ""))
        return run, stack

    def test_existing_output_without_completion_is_not_reused(self) -> None:
        run, stack = self.prepared()
        output = run / "generated-100.bin"
        output.write_bytes(b"interrupted data")
        execute = stack.enter_context(patch.object(workflow, "run_command"))
        with self.assertRaises(ValueError):
            workflow.generate(run, 100)
        execute.assert_not_called()
        self.assertEqual(output.read_bytes(), b"interrupted data")
        self.assertFalse((run / "generated-100.json").exists())

    def test_modified_completed_output_is_rejected(self) -> None:
        run, stack = self.prepared()
        output = run / "generated-100.bin"
        write_mnsd(output, seed=100,
                   checksum=(run / "pst-base.bin").read_bytes()[48:80], games=[0])
        (run / "generated-100.json").write_text(json.dumps({"sha256": workflow.digest(output)}))
        modified = bytearray(output.read_bytes())
        modified[136 + 147] ^= 1  # 探索値だけを変え、MNSD構造は有効に保つ。
        output.write_bytes(modified)
        execute = stack.enter_context(patch.object(workflow, "run_command"))
        with self.assertRaises(ValueError):
            workflow.generate(run, 100)
        execute.assert_not_called()

    def test_failed_generation_does_not_create_completion_record(self) -> None:
        run, stack = self.prepared()

        def fail(run, label, command, cwd):
            (run / "generated-100.bin").write_bytes(b"partial output")
            raise subprocess.CalledProcessError(1, command)

        stack.enter_context(patch.object(workflow, "run_command", side_effect=fail))
        with self.assertRaises(subprocess.CalledProcessError):
            workflow.generate(run, 100)
        self.assertFalse((run / "generated-100.json").exists())

    def test_completed_valid_generation_is_reused_without_running_engine(self) -> None:
        run, stack = self.prepared()
        output = run / "generated-100.bin"
        checksum = (run / "pst-base.bin").read_bytes()[48:80]
        write_mnsd(output, seed=100, checksum=checksum, games=[0])
        (run / "generated-100.json").write_text(json.dumps({"sha256": workflow.digest(output)}))
        execute = stack.enter_context(patch.object(workflow, "run_command"))
        workflow.generate(run, 100)
        execute.assert_not_called()

    def test_diagnosis_rejects_modified_completed_weights(self) -> None:
        run, _ = self.prepared()
        training = run / "training"
        training.mkdir()
        candidate = training / "pst.bin"
        candidate.write_bytes((run / "pst-base.bin").read_bytes())
        float_path = training / "pst-float.npz"
        float_path.write_bytes(b"fixture")
        (training / "complete.json").write_text(json.dumps({
            "sha256": workflow.digest(candidate), "float_sha256": workflow.digest(float_path)}))
        candidate.write_bytes(candidate.read_bytes() + b"changed")
        with self.assertRaises(ValueError):
            workflow.diagnose(run)
        self.assertFalse((run / "diagnostics").exists())

    def test_cpu_training_and_diagnostics_preserve_scale_and_outputs(self) -> None:
        """小さな実データで尺度推定から量子化・診断までを実際に接続する。"""
        games = list(range(1, 81))
        write_mnsd(self.root / "data/old.bin", seed=0, checksum=b"a" * 32,
                   games=games)
        self.config["generate"]["seeds"] = [100, 200]
        self.config["generate"]["games"] = 80
        self.config["train"].update(epochs=1, batch=32, validation_sample=16)
        self.config["diagnose"]["sample_size"] = 16
        run, _ = self.prepared()
        checksum = (run / "pst-base.bin").read_bytes()[48:80]
        for seed in (100, 200):
            output = run / f"generated-{seed}.bin"
            write_mnsd(output, seed=seed, checksum=checksum, games=games)
            (run / f"generated-{seed}.json").write_text(
                json.dumps({"sha256": workflow.digest(output)}))

        workflow.train(run)
        candidate = run / "training/pst.bin"
        inputs = json.loads((run / "training/inputs.json").read_text())
        completion = json.loads((run / "training/complete.json").read_text())
        middlegame, endgame, piece_values, saved_k = read_mnpt(candidate)
        np.testing.assert_array_equal(middlegame, endgame)
        np.testing.assert_array_equal(piece_values, initial_piece_values())
        self.assertEqual(completion["float_sha256"], workflow.digest(run / "training/pst-float.npz"))
        # MNPTは尺度をfloat32で保存する。推定値の保存時丸めだけを許容する。
        self.assertAlmostEqual(saved_k / inputs["k"], 1, places=6)
        self.assertEqual(completion["sha256"], workflow.digest(candidate))
        self.assertEqual(inputs["training_records"] + inputs["validation_records"], 240)
        self.assertEqual(inputs["options"]["device"], "cpu")

        with patch.object(workflow, "diagnose_probe", return_value=python_probe):
            workflow.diagnose(run)
        report = json.loads((run / "diagnostics/report.json").read_text())
        self.assertEqual(len(report["bands"]), 10)
        validation = inputs["validation_records"]
        self.assertEqual(sum(entry["samples"] for entry in report["bands"]), validation)
        for entry in report["bands"]:
            self.assertTrue((run / "diagnostics" / entry["indices_file"]).is_file())
        self.assertEqual(report["rust_agreement"], {"base": validation, "candidate": validation})
        original = candidate.read_bytes()
        with self.assertRaises(ValueError):
            workflow.train(run)
        self.assertEqual(candidate.read_bytes(), original)


if __name__ == "__main__":
    unittest.main()
