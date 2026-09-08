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
from test_pst_diagnostics import python_fm_probe, python_probe
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
        self.assertFalse((run / "diagnostics/candidate-probe.json").exists())
        original = candidate.read_bytes()
        with self.assertRaises(ValueError):
            workflow.train(run)
        self.assertEqual(candidate.read_bytes(), original)


class FMWorkflowTest(unittest.TestCase):
    """FM固有の設定、基準尺度の維持、および候補の適格性を検証する。"""

    setUp = WorkflowTest.setUp
    prepared = WorkflowTest.prepared

    def test_fm_requires_all_and_only_its_fields(self) -> None:
        content = CONFIG.replace('model = "single"',
                                 'model = "fm"\nrank = 16\nweight_decay = 0.0001\nlambda_res = 0.001\npatience = 3')
        self.config_path.write_text(content)
        self.assertEqual(workflow.load_config(self.config_path, self.root)["train"]["rank"], 16)
        for before, after in (("rank = 16\n", ""), ("patience = 3\n", ""),
                              ("weight_decay = 0.0001\n", ""), ("lambda_res = 0.001\n", ""),
                              ("rank = 16", "rank = 0"), ("rank = 16", "rank = true"),
                              ("patience = 3", "patience = 0"),
                              ("lambda_res = 0.001", "lambda_res = nan"),
                              ("weight_decay = 0.0001", "weight_decay = -1"),
                              ("rank = 16", "rank = 16\nextra = 1"),
                              ('model = "fm"', 'model = "tapered"')):
            self.config_path.write_text(content.replace(before, after))
            with self.subTest(after=after), self.assertRaises(ValueError):
                workflow.load_config(self.config_path, self.root)

    def fm_prepared(self, *, epoch_zero: bool = False) -> Path:
        from test_train_fm import fixture
        data, _ = fixture(self.root, epoch_zero=epoch_zero)
        self.config["run"]["data"] = [str(data)]
        self.config["generate"]["seeds"] = []
        self.config["train"].update(model="fm", rank=2, weight_decay=0.001, lambda_res=0.001,
                                    patience=1, learning_rate=0.01, epochs=2, batch=8,
                                    validation_sample=10, **{"lambda": 0})
        run, _ = self.prepared()
        return run

    def test_fm_uses_baseline_k_and_passes_teacher_ks_then_diagnoses(self) -> None:
        from train_fm import read_mnpt_v3
        run = self.fm_prepared()
        with patch("train_pst.estimate_mixed_k", side_effect=AssertionError("FM must not estimate output K")):
            workflow.train(run)
        inputs = json.loads((run / "training/inputs.json").read_text())
        self.assertEqual(inputs["k"], 1000)
        self.assertIn("train_fm.py", json.loads((run / "prepared.json").read_text())["sources"])
        commands = [json.loads(line) for line in (run / "commands.jsonl").read_text().splitlines()]
        argv = commands[0]["argv"]
        self.assertEqual(float(argv[argv.index("--teacher-ks") + 1]), inputs["teacher_ks"][0])
        candidate = read_mnpt_v3(run / "training/pst.bin")
        for expected, actual in zip(read_mnpt(run / "pst-base.bin"), candidate[:4]):
            np.testing.assert_array_equal(actual, expected)
        binary = self.root / "target/release/pst_probe"
        base_binary = run / "generator/target/release/pst_probe"
        built_bytes = b"newly built FM probe"
        commit = "1234567890" * 4
        status = " M src/eval/fm.rs\n?? untracked.rs"

        def build_probe(run_directory, label, command, cwd):
            self.assertEqual(cwd, self.root)
            self.assertEqual(command, ["cargo", "build", "--release", "--locked", "--bin", "pst_probe"])
            binary.parent.mkdir(parents=True)
            binary.write_bytes(built_bytes)

        def select_probe(path):
            if path == binary:
                self.assertEqual(binary.read_bytes(), built_bytes)
                return python_fm_probe
            self.assertEqual(path, base_binary)
            return python_probe

        with patch.object(workflow, "run_command", side_effect=build_probe) as build, \
                patch.object(workflow, "diagnose_probe", side_effect=select_probe) as probes, \
                patch.object(workflow, "git", side_effect=lambda repo, *args:
                             commit if args == ("rev-parse", "HEAD") else status) as git:
            workflow.diagnose(run)
        build.assert_called_once()
        self.assertCountEqual([call.args[0] for call in probes.call_args_list], [binary, base_binary])
        self.assertCountEqual([call.args for call in git.call_args_list],
                              [(self.root, "rev-parse", "HEAD"), (self.root, "status", "--porcelain")])
        provenance = json.loads((run / "diagnostics/candidate-probe.json").read_text())
        self.assertEqual(provenance, {
            "repository": str(self.root), "binary": str(binary),
            "sha256": hashlib.sha256(built_bytes).hexdigest(), "commit": commit,
            "status_porcelain": status,
        })
        report = json.loads((run / "diagnostics/report.json").read_text())
        samples = sum(band["samples"] for band in report["bands"])
        self.assertGreater(samples, 0)
        self.assertEqual(report["rust_agreement"],
                         {"base": samples, "candidate": samples, "candidate_pst": samples})
        self.assertTrue((run / "training/complete.json").exists())

    def test_git_status_preserves_index_and_worktree_columns(self) -> None:
        with patch.object(workflow.subprocess, "check_output", return_value=" M tracked.rs\n?? new.rs\n"):
            self.assertEqual(workflow.git(self.root, "status", "--porcelain"), " M tracked.rs\n?? new.rs")

    def test_fm_epoch_zero_completes_as_excluded_without_candidate(self) -> None:
        run = self.fm_prepared(epoch_zero=True)
        workflow.train(run)
        self.assertTrue((run / "training/excluded.json").exists())
        self.assertFalse((run / "training/complete.json").exists())
        self.assertFalse((run / "training/pst.bin").exists())
        with self.assertRaisesRegex(ValueError, "epoch is 0"):
            workflow.diagnose(run)

    def test_fm_rejects_changes_to_each_fixed_component_before_completion(self) -> None:
        from train_fm import training_report_path, write_mnpt_v3
        # 出力プロセスが正常終了しても、不変性違反には完了記録を作らない。
        run = self.fm_prepared()
        for changed_component in range(4):
            def changed_output(run, label, command, cwd):
                mg, eg, values, k = read_mnpt(run / "pst-base.bin")
                if changed_component == 0:
                    mg[0] += 1
                elif changed_component == 1:
                    eg[0] += 1
                elif changed_component == 2:
                    values[29] += 1
                    values[11] += 1
                    values[21] += 1
                else:
                    k += 1
                output = run / "training/pst.bin"
                write_mnpt_v3(output, mg, eg, values, k, np.ones((FEATURE_COUNT, 2), dtype=np.int16),
                              np.ones(2, dtype=np.int8), 0)
                training_report_path(output).write_text(json.dumps({"status": "candidate", "best_epoch": 1}))
            with self.subTest(component=changed_component), patch.object(workflow, "run_command", side_effect=changed_output):
                with self.assertRaisesRegex(ValueError, "changed fixed"):
                    workflow.train(run)
                self.assertFalse((run / "training/complete.json").exists())
            (run / "training").rename(run / f"rejected-{changed_component}")


if __name__ == "__main__":
    unittest.main()
