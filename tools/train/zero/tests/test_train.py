"""学習器の設計契約を、実プロセスの中断と独立した計算で検証する。

再開の有無にかかわらず重み・最適化器・読取順序が一致し、最終保存からは出力を
再生成できることを確かめる。損失は交差エントロピーの定義式、読取順序はPCG64と
リトルエンディアンのint64列を根拠とし、実装が出した値を期待値として固定しない。
"""

import hashlib
import json
import math
from pathlib import Path
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import time
import unittest

import numpy as np
import torch

from minase_zero.data import is_training_sample, is_validation, read_window
from minase_zero.encoding import encode_planes, policy_mask
from minase_zero.net import ZeroNet
from minase_zero.train import compute_losses

from .cli import COMMIT, ZERO, environment, init_arguments, require_success, run_cli, train_arguments
from .test_data import single_game, write_small


class LossTests(unittest.TestCase):
    def setUp(self) -> None:
        previous = torch.get_num_threads()
        torch.set_num_threads(1)
        self.addCleanup(torch.set_num_threads, previous)

    def test_losses_use_masked_labels_quantized_probabilities_and_mixed_results(self) -> None:
        # 1局面目は候補1つ、2局面目は候補3つとし、候補数で平均する誤りも検出する。
        logits = torch.zeros(2, 35_856)
        logits[0, 17] = 2.0
        logits[1, 23] = -3.0
        logits.requires_grad_()
        values = torch.tensor([[80.0], [-80.0]], requires_grad=True)
        losses = compute_losses(
            logits, values, torch.tensor([0, 1, 1, 1]), torch.tensor([2, 4, 23, 500]),
            torch.tensor([65_535, 10_000, 20_000, 35_535]),
            torch.tensor([1.0, 0.5]), torch.tensor([0, 65_535]), 0.333, torch.from_numpy(policy_mask()),
        )
        expected_policy = (
            math.log(16_107 + math.exp(2))
            + math.log(16_107 + math.exp(-3)) + 3 * 20_000 / 65_535
        ) / 2

        def bce(logit: float, target: float) -> float:
            return max(logit, 0) + math.log1p(math.exp(-abs(logit))) - logit * target

        expected_value = (
            0.667 * bce(80, 1) + 0.333 * bce(80, 0)
            + 0.667 * bce(-80, 0.5) + 0.333 * bce(-80, 1)
        ) / 2
        self.assertAlmostEqual(losses[0].item(), expected_policy, places=5)
        self.assertAlmostEqual(losses[1].item(), expected_value, places=5)
        self.assertAlmostEqual(losses[2].item(), (bce(80, 1) + bce(-80, 0.5)) / 2, places=6)
        self.assertFalse(losses[2].requires_grad)
        self.assertTrue(all(loss.dtype == torch.float32 for loss in losses))
        (losses[0] + losses[1]).backward()
        self.assertAlmostEqual(values.grad[0, 0].item(), (1 - 0.667) / 2, places=6)
        self.assertAlmostEqual(values.grad[1, 0].item(), -(0.667 * 0.5 + 0.333) / 2, places=6)
        self.assertGreater(logits.grad[0, 17].item(), 0)

    def test_bfloat_logits_do_not_round_quantized_teachers_before_mixing(self) -> None:
        policy, value, result = compute_losses(
            torch.zeros(1, 35_856, dtype=torch.bfloat16),
            torch.tensor([[20.0]], dtype=torch.bfloat16),
            torch.tensor([0]), torch.tensor([7]), torch.tensor([65_535]),
            torch.tensor([0.0]), torch.tensor([32_769]), 1.0, torch.from_numpy(policy_mask()),
        )
        expected = 20 + math.log1p(math.exp(-20)) - 20 * 32_769 / 65_535
        self.assertAlmostEqual(value.item(), expected, places=5)
        self.assertAlmostEqual(result.item(), 20, places=6)
        self.assertAlmostEqual(policy.item(), math.log(16_108), places=5)

    def test_masked_targets_raise_including_out_of_range_labels(self) -> None:
        for label in (143, -1, 35_856):
            with self.subTest(label=label), self.assertRaises(ValueError):
                compute_losses(
                    torch.zeros(1, 35_856), torch.zeros(1, 1),
                    torch.tensor([0]), torch.tensor([label]), torch.tensor([65_535]),
                    torch.tensor([1.0]), torch.tensor([0]), 0.333, torch.from_numpy(policy_mask()),
                )

    def test_masked_logits_have_no_effect_or_gradient_and_match_reference(self) -> None:
        mask = torch.from_numpy(policy_mask())
        logits = torch.full((2, 35_856), -1000.0)
        logits[0, [2, 4, 23]] = torch.tensor([math.log(2), math.log(3), math.log(5)])
        logits[1, [2, 4, 23]] = torch.tensor([math.log(7), math.log(11), math.log(13)])
        targets = (torch.tensor([0, 1, 1]), torch.tensor([2, 4, 23]),
                   torch.tensor([65_535, 21_845, 43_690]), torch.tensor([1.0, 0.0]), torch.tensor([0, 65_535]))
        expected = (math.log(10 / 2) + math.log(31 / 11) / 3 + math.log(31 / 13) * 2 / 3) / 2
        reference = compute_losses(logits, torch.zeros(2, 1), *targets, 0.333, mask)[0]
        self.assertAlmostEqual(reference.item(), expected, places=6)
        logits[:, ~mask] = 1_000_000
        logits.requires_grad_()
        changed = compute_losses(logits, torch.zeros(2, 1), *targets, 0.333, mask)[0]
        self.assertTrue(torch.equal(reference, changed))
        changed.backward()
        self.assertTrue(torch.all(logits.grad[:, ~mask] == 0))
        self.assertTrue(torch.any(logits.grad[:, mask] != 0))


class TrainTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-train-test-")
        cls.addClassCleanup(directory.cleanup)
        cls.root = Path(directory.name)
        cls.initial = cls.root / "initial"
        require_success(run_cli(init_arguments(cls.initial, 23)))
        cls.a, cls.b, cls.partial = (cls.root / name for name in ("continuous", "resumed", "partial"))
        cls.uninterrupted = run_cli(train_arguments(cls.initial / "checkpoint.pt", cls.a))
        require_success(cls.uninterrupted)
        with (cls.root / "interrupted.log").open("w+") as log:
            process = subprocess.Popen(
                [sys.executable, "-m", "minase_zero", *train_arguments(cls.initial / "checkpoint.pt", cls.b)],
                cwd=ZERO, env=environment(), stdout=log, stderr=log,
            )
            try:
                deadline = time.monotonic() + 120
                while not (cls.b / "checkpoint.pt").exists():
                    if process.poll() is not None or time.monotonic() >= deadline:
                        log.seek(0)
                        raise AssertionError("training ended before a checkpoint appeared: " + log.read())
                    time.sleep(0.002)
                process.kill()
                cls.interrupted_returncode = process.wait(timeout=30)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait(timeout=30)
        cls.interrupted_state = torch.load(cls.b / "checkpoint.pt", weights_only=True)
        if (cls.b / "summary.json").exists() or not (
            0 < cls.interrupted_state["completed_steps"] < cls.interrupted_state["total_steps"]
        ):
            raise AssertionError("SIGKILL did not interrupt an unfinished cycle")
        shutil.copytree(cls.b, cls.partial)
        cls.resumption = run_cli(["train", "--resume", str(cls.b), "--device", "cpu"])
        require_success(cls.resumption)
        cls.state_a = torch.load(cls.a / "checkpoint.pt", weights_only=True)
        cls.state_b = torch.load(cls.b / "checkpoint.pt", weights_only=True)
        cls.summary_a = json.loads((cls.a / "summary.json").read_text())
        cls.summary_b = json.loads((cls.b / "summary.json").read_text())
        cls.data, cls.infos = read_window([ZERO / "fixtures/zero-selfplay-small.mnsd"])

    def assert_state_equal(self, left: object, right: object) -> None:
        """最適化器の入れ子を含め、全テンソルをtorch.equalで比べる。"""
        if isinstance(left, torch.Tensor):
            self.assertIsInstance(right, torch.Tensor)
            self.assertTrue(torch.equal(left, right))
        elif isinstance(left, dict):
            self.assertEqual(left.keys(), right.keys())
            for key in left:
                self.assert_state_equal(left[key], right[key])
        elif isinstance(left, list):
            self.assertEqual(len(left), len(right))
            for a, b in zip(left, right):
                self.assert_state_equal(a, b)
        else:
            self.assertEqual(left, right)

    def test_sigkill_resume_matches_uninterrupted_training(self) -> None:
        self.assertEqual(self.interrupted_returncode, -signal.SIGKILL)
        self.assertGreater(self.interrupted_state["completed_steps"], 0)
        self.assertLess(self.interrupted_state["completed_steps"], self.interrupted_state["total_steps"])
        for key in ("model", "optimizer", "completed_steps", "total_steps", "positions_seen",
                    "data_order_sha256", "loss_sums", "learning_curve"):
            with self.subTest(key=key):
                self.assert_state_equal(self.state_a[key], self.state_b[key])
        self.assertEqual(self.summary_a["diagnostics"], self.summary_b["diagnostics"])
        self.assertEqual((self.a / "model.onnx").read_bytes(), (self.b / "model.onnx").read_bytes())
        self.assertIn(f"completed_steps={self.interrupted_state['completed_steps']}/", self.resumption.stderr)

    def test_order_digest_counts_and_batch_normalization_follow_the_contract(self) -> None:
        training = np.flatnonzero(~self.data.validation)
        permutation = np.random.Generator(np.random.PCG64(42)).permutation(len(training))
        ordered = training[permutation].tolist()
        digest = bytes(32)
        for start in range(0, len(ordered), 16):
            digest = hashlib.sha256(digest + b"".join(struct.pack("<q", index) for index in ordered[start:start + 16])).digest()
        updates = math.ceil(len(training) / 16)
        self.assertGreaterEqual(updates, 30)
        self.assertNotEqual(len(training) % 16, 0)
        for state in (self.state_a, self.state_b, self.summary_a, self.summary_b):
            self.assertEqual(state["data_order_sha256"], digest.hex())
            self.assertEqual(state["completed_steps"], updates)
            self.assertEqual(state["total_steps"], updates)
            self.assertEqual(state["positions_seen"], len(training))
        checkpoints = list(range(4, updates + 1, 4))
        if checkpoints[-1] != updates:
            checkpoints.append(updates)
        self.assertEqual([point["completed_steps"] for point in self.summary_a["learning_curve"]], checkpoints)
        self.assertEqual(len(self.uninterrupted.stderr.splitlines()), len(checkpoints))
        for name, value in self.state_a["model"].items():
            if name.endswith("num_batches_tracked"):
                self.assertEqual(value.item(), updates)
        initial = torch.load(self.initial / "checkpoint.pt", weights_only=True)
        self.assertFalse(torch.equal(initial["model"]["trunk.0.weight"], self.state_a["model"]["trunk.0.weight"]))

    def test_final_checkpoint_recreates_outputs_without_updates(self) -> None:
        out = self.root / "final-only"
        shutil.copytree(self.a, out)
        (out / "model.onnx").unlink()
        (out / "summary.json").unlink()
        saved_bytes = (out / "checkpoint.pt").read_bytes()
        require_success(run_cli(["train", "--resume", str(out), "--device", "cpu"]))
        self.assertEqual(saved_bytes, (out / "checkpoint.pt").read_bytes())
        self.assertEqual((out / "model.onnx").read_bytes(), (self.a / "model.onnx").read_bytes())
        summary = json.loads((out / "summary.json").read_text())
        for key in self.summary_a.keys() - {"elapsed_seconds", "positions_per_second", "loading_peak_rss_bytes"}:
            self.assertEqual(summary[key], self.summary_a[key], key)

    def test_provenance_preserves_argument_paths_header_and_environment(self) -> None:
        provenance = self.summary_a["provenance"]
        info = self.infos[0]
        record = provenance["files"][0]
        self.assertEqual(provenance["trainer_commit"], COMMIT)
        self.assertEqual(record["path"], "fixtures/zero-selfplay-small.mnsd")
        self.assertEqual(record["size"], info.size)
        self.assertEqual(record["sha256"], hashlib.sha256(info.path.read_bytes()).hexdigest())
        self.assertEqual(record["position_count"], len(self.data))
        self.assertEqual(record["header"]["generation_commit"], info.header.generation_commit)
        self.assertEqual(record["header"]["network_checksum"], info.header.network_checksum.hex())
        self.assertEqual(provenance["init_sha256"], hashlib.sha256((self.initial / "checkpoint.pt").read_bytes()).hexdigest())
        for key in ("python_version", "pytorch_version", "numpy_version", "device_name"):
            self.assertTrue(provenance[key])
        self.assertIn("cuda_version", provenance)

    def test_next_cycle_carries_momentum_replaces_settings_and_clips_gradients(self) -> None:
        previous_threads = torch.get_num_threads()
        torch.set_num_threads(1)
        self.addCleanup(torch.set_num_threads, previous_threads)
        numbers = np.arange(100, dtype=np.uint32)
        validation = is_validation(20261038, numbers)
        sample = is_training_sample(20261038, numbers)
        train_games = [int(numbers[sample][0]), int(numbers[~(validation | sample)][0])]
        validation_game = int(numbers[validation][0])
        path = self.root / "one-update.mnsd"
        games = [
            (int(train_games[0]), 0, 0, 12345, (10_000, 55_535)),
            (int(train_games[1]), 1, 0, 54321, (65_535,)),
            (validation_game, 0, 2, 32768, (65_535,)),
        ]
        write_small(path, [single_game(*game) for game in sorted(games)], "engine-default")
        data, _ = read_window([path])
        indices = np.flatnonzero(~data.validation)
        indices = indices[np.random.Generator(np.random.PCG64(42)).permutation(len(indices))]
        model = ZeroNet(1, 16).train()
        model.load_state_dict(self.state_a["model"])
        inputs = encode_planes(*(torch.from_numpy(column[indices]) for column in (data.boards, data.sides, data.lions)))
        policy, value = model.forward_logits(inputs)
        # 参照計算では密な方策と、結果・根の価値に対する別々のBCEを使う。
        targets = torch.zeros(2, 35_856)
        for row, index in enumerate(indices):
            for candidate in range(int(data.policy_offsets[index]), int(data.policy_offsets[index + 1])):
                targets[row, int(data.policy_labels[candidate])] = int(data.policy_probabilities[candidate]) / 65_535
        results = torch.tensor(data.results[indices])
        roots = torch.tensor(data.values[indices], dtype=torch.float32) / 65_535
        mask = torch.from_numpy(policy_mask())
        policy_loss = -(targets[:, mask] * policy[:, mask].log_softmax(1)).sum(1).mean()
        result_loss = torch.nn.functional.binary_cross_entropy_with_logits(value.flatten(), results)
        root_loss = torch.nn.functional.binary_cross_entropy_with_logits(value.flatten(), roots)
        value_loss = 0.667 * result_loss + 0.333 * root_loss
        (policy_loss + value_loss).backward()
        norm = sum(parameter.grad.double().square().sum() for parameter in model.parameters()).sqrt().item()
        self.assertGreater(norm, 0.1)
        expected_parameters, expected_buffers = [], []
        old_optimizer = self.state_a["optimizer"]
        for parameter, identifier in zip(model.parameters(), old_optimizer["param_groups"][0]["params"]):
            old_buffer = old_optimizer["state"][identifier]["momentum_buffer"].double()
            gradient = parameter.grad.double() * (0.1 / norm) + 0.0002 * parameter.detach().double()
            buffer = 0.8 * old_buffer + gradient
            expected_buffers.append(buffer)
            expected_parameters.append(parameter.detach().double() - 0.03 * (gradient + 0.8 * buffer))

        out = self.root / "next-cycle"
        arguments = train_arguments(self.a / "checkpoint.pt", out)
        for flag, value in (("--data", str(path)), ("--cycle", "2"), ("--batch-size", "2"),
                            ("--lr", "0.03"), ("--momentum", "0.8"), ("--weight-decay", "0.0002"),
                            ("--clip-grad-norm", "0.1")):
            arguments[arguments.index(flag) + 1] = value
        require_success(run_cli(arguments))
        state = torch.load(out / "checkpoint.pt", weights_only=True)
        summary = json.loads((out / "summary.json").read_text())
        self.assertEqual(state["cycle"], 2)
        self.assertEqual(state["completed_steps"], 1)
        self.assertEqual(state["positions_seen"], 2)
        group = state["optimizer"]["param_groups"][0]
        self.assertEqual((group["lr"], group["momentum"], group["weight_decay"], group["nesterov"]), (0.03, 0.8, 0.0002, True))
        names = [name for name, _ in model.named_parameters()]
        for name, identifier, expected_parameter, expected_buffer in zip(
            names, group["params"], expected_parameters, expected_buffers,
        ):
            torch.testing.assert_close(state["model"][name].double(), expected_parameter, rtol=2e-6, atol=2e-7)
            torch.testing.assert_close(state["optimizer"]["state"][identifier]["momentum_buffer"].double(), expected_buffer,
                                       rtol=1e-5, atol=2e-6)
        for key, expected in (("policy", policy_loss), ("value", value_loss), ("value_result", result_loss)):
            self.assertAlmostEqual(summary["training_losses"][key], expected.item(), places=5)

    def test_finished_nonempty_unfinished_init_and_cpu_bf16_are_rejected(self) -> None:
        cases = [
            ["train", "--resume", str(self.a), "--device", "cpu"],
            train_arguments(self.initial / "checkpoint.pt", self.a),
            train_arguments(self.partial / "checkpoint.pt", self.root / "bad-init"),
        ]
        bf16 = train_arguments(self.initial / "checkpoint.pt", self.root / "bf16")
        bf16[bf16.index("--precision") + 1] = "bf16"
        cases.append(bf16)
        for arguments in cases:
            with self.subTest(arguments=arguments):
                result = run_cli(arguments)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)

    def test_resume_checks_raw_paths_sizes_and_training_validation_counts(self) -> None:
        for field in ("path", "size", "training_count", "validation_count"):
            with self.subTest(field=field):
                out = self.root / f"wrong-{field}"
                shutil.copytree(self.partial, out)
                state = torch.load(out / "checkpoint.pt", weights_only=True)
                record = state["provenance"]["files"][0]
                if field == "path":
                    record[field] = "./" + record[field]
                else:
                    record[field] += 1
                torch.save(state, out / "checkpoint.pt")
                result = run_cli(["train", "--resume", str(out), "--device", "cpu"])
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)
                self.assertFalse((out / "summary.json").exists())

    def test_resume_uses_recorded_provenance_without_rechecking_content_hash(self) -> None:
        out = self.root / "changed-header"
        shutil.copytree(self.a, out)
        (out / "summary.json").unlink()
        data_path = self.root / "same-size.mnsd"
        content = bytearray(self.infos[0].path.read_bytes())
        # 形式仕様の生成コミット40バイトだけを変え、局面とサイズを保つ。
        content[40:80] = b"f" * 40
        data_path.write_bytes(content)
        state = torch.load(out / "checkpoint.pt", weights_only=True)
        state["config"]["data"] = [str(data_path)]
        state["provenance"]["files"][0]["path"] = str(data_path)
        recorded = state["provenance"]["files"][0]["sha256"]
        self.assertNotEqual(recorded, hashlib.sha256(content).hexdigest())
        torch.save(state, out / "checkpoint.pt")
        require_success(run_cli(["train", "--resume", str(out), "--device", "cpu"]))
        summary = json.loads((out / "summary.json").read_text())
        self.assertEqual(summary["provenance"]["files"][0]["sha256"], recorded)
        self.assertEqual(summary["diagnostics"], self.summary_a["diagnostics"])

    def test_summary_contains_both_final_diagnostics_and_loading_measurements(self) -> None:
        seed = self.infos[0].header.seed
        for name, selected, remainder in (("training_sample", self.data.training_sample, 1),
                                          ("validation", self.data.validation, 0)):
            diagnostic = self.summary_a["diagnostics"][name]
            games = np.unique(self.data.games[selected]).tolist()
            self.assertEqual(diagnostic["game_count"], len(games))
            self.assertEqual(diagnostic["position_count"], int(selected.sum()))
            self.assertEqual(diagnostic["games"], [{"seed": seed, "game": game} for game in games])
            self.assertEqual(diagnostic["hash_modulus"], 20)
            self.assertEqual(diagnostic["hash_remainder"], remainder)
            for metric in ("policy", "value", "value_result", "legal_probability_mass",
                           "log_legal_probability_mass", "policy_agreement"):
                self.assertTrue(math.isfinite(diagnostic[metric]), metric)
            self.assertGreater(diagnostic["legal_probability_mass"], 0)
            self.assertLessEqual(diagnostic["legal_probability_mass"], 1)
            self.assertLessEqual(diagnostic["log_legal_probability_mass"], 0)
            self.assertGreaterEqual(diagnostic["policy_agreement"], 0)
            self.assertLessEqual(diagnostic["policy_agreement"], 1)
        for phase in ("loading", "training", "diagnostics"):
            self.assertGreater(self.summary_a["elapsed_seconds"][phase], 0)
        self.assertGreater(self.summary_a["loading_peak_rss_bytes"], 0)

    def test_empty_training_diagnostic_sample_is_rejected_before_updates(self) -> None:
        numbers = np.arange(100, dtype=np.uint32)
        validation = is_validation(20261038, numbers)
        sample = is_training_sample(20261038, numbers)
        selected = sorted([int(numbers[validation][0]), int(numbers[~(validation | sample)][0])])
        path = self.root / "no-sample.mnsd"
        write_small(path, [single_game(game, 0, 2, 32768, (65_535,)) for game in selected], "engine-default")
        out = self.root / "no-sample"
        arguments = train_arguments(self.initial / "checkpoint.pt", out)
        arguments[arguments.index("--data") + 1] = str(path)
        result = run_cli(arguments)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("training diagnostic sample", result.stderr)
        self.assertFalse((out / "checkpoint.pt").exists())
        self.assertFalse((out / "summary.json").exists())

    def test_invalid_arguments_and_missing_files_return_one_error_line(self) -> None:
        wrong_commit = init_arguments(self.root / "bad-commit", 0)
        wrong_commit[-1] = "z" * 40
        missing = train_arguments(self.root / "missing.pt", self.root / "missing-output")
        cases = [[], ["init"], wrong_commit, missing,
                 ["train", "--resume", str(self.partial), "--device", "cpu", "--seed", "1"]]
        for arguments in cases:
            with self.subTest(arguments=arguments):
                result = run_cli(arguments)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)


if __name__ == "__main__":
    unittest.main()
