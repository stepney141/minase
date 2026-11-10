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
import tempfile
import time
import unittest

import numpy as np
import torch

from minase_zero.data import is_training_sample, is_validation, read_replay_buffer
from minase_zero.encoding import encode_planes, policy_mask
from minase_zero.net import ZeroNet
from minase_zero.train import compute_losses

from tests.cli import (COMMIT, ZERO, command, environment, init_arguments, kill_training,
                  require_success, run_cli, set_training_data, train_arguments)
from tests.test_data import single_game, write_small
from tests.test_replay_buffer import reference_sample


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

    def test_policy_loss_and_gradients_match_positionwise_scatter_mean(self) -> None:
        # 局面ごとの旧集計と、候補数が異なり確率0も含む教師で照合する。
        mask = torch.from_numpy(policy_mask())
        logits = torch.randn(3, len(mask), generator=torch.Generator().manual_seed(31), requires_grad=True)
        reference_logits = logits.detach().clone().requires_grad_()
        rows = torch.tensor([0, 1, 1, 1, 2, 2])
        labels = torch.tensor([2, 4, 23, 500, 2, 23])
        probabilities = torch.tensor([65_535, 0, 12_345, 53_190, 32_768, 32_767])
        results = torch.tensor([1.0, 0.0, 0.5])
        actual = compute_losses(
            logits, torch.zeros(3, 1), rows, labels, probabilities,
            results, torch.tensor([0, 65_535, 32_768]), 0.333, mask,
        )[0]
        candidate_losses = -torch.log_softmax(reference_logits.masked_fill(~mask, -torch.inf), dim=1)[rows, labels]
        candidate_losses = candidate_losses * (probabilities.float() / 65_535)
        expected = torch.zeros(3, dtype=torch.float32).scatter_add_(0, rows, candidate_losses).mean()
        torch.testing.assert_close(actual, expected)
        torch.testing.assert_close(
            torch.autograd.grad(actual, logits)[0], torch.autograd.grad(expected, reference_logits)[0],
        )

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
        cls.runs = {n: cls.train_pair(n) for n in (1, 2)}
        for name, value in cls.runs[1].items():
            setattr(cls, name, value)
        cls.data, cls.infos, _, _ = read_replay_buffer(
            [ZERO / "fixtures/zero-selfplay-small.mnsd"], [ZERO / "fixtures/zero-selfplay-small.mnsd"], 42,
            rank=0, world_size=1, batch_size=16, micro_batch_size=4, replay_threads=1,
        )

    @classmethod
    def train_pair(cls, n):
        a, b, partial = (cls.root / f"{name}-{n}" for name in ("continuous", "resumed", "partial"))
        uninterrupted = run_cli(train_arguments(cls.initial / "checkpoint.pt", a), n)
        require_success(uninterrupted)
        with (cls.root / f"interrupted-{n}.log").open("w+") as log:
            process = subprocess.Popen(
                command(train_arguments(cls.initial / "checkpoint.pt", b), n),
                cwd=ZERO, env=environment(), stdout=log, stderr=log, start_new_session=True,
            )
            try:
                deadline = time.monotonic() + 120
                while not (b / "checkpoint.pt").exists():
                    if process.poll() is not None or time.monotonic() >= deadline:
                        log.seek(0)
                        raise AssertionError("training ended before a checkpoint appeared: " + log.read())
                    time.sleep(0.002)
                interrupted_returncode = kill_training(process)
            finally:
                if process.poll() is None:
                    kill_training(process)
        interrupted_state = torch.load(b / "checkpoint.pt", weights_only=True)
        if (b / "summary.json").exists() or not (
            0 < interrupted_state["completed_steps"] < interrupted_state["total_steps"]
        ):
            raise AssertionError("SIGKILL did not interrupt an unfinished cycle")
        shutil.copytree(b, partial)
        b.with_name(b.name + "-data.json").unlink()
        resumption = run_cli(["train", "--resume", str(b), "--device", "cpu", "--replay-threads", "1"], n)
        require_success(resumption)
        return dict(a=a, b=b, partial=partial, uninterrupted=uninterrupted,
                    interrupted_returncode=interrupted_returncode, interrupted_state=interrupted_state,
                    resumption=resumption,
                    state_a=torch.load(a / "checkpoint.pt", weights_only=True),
                    state_b=torch.load(b / "checkpoint.pt", weights_only=True),
                    summary_a=json.loads((a / "summary.json").read_text()),
                    summary_b=json.loads((b / "summary.json").read_text()))

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
        for n, run in self.runs.items():
            with self.subTest(world_size=n):
                self.assertEqual(run["interrupted_returncode"], -signal.SIGKILL)
                for key in run["state_a"].keys() - {"elapsed_seconds", "loading_peak_rss_bytes"}:
                    self.assert_state_equal(run["state_a"][key], run["state_b"][key])
                for key in run["summary_a"].keys() - {"elapsed_seconds", "loading_peak_rss_bytes", "positions_per_second"}:
                    self.assert_state_equal(run["summary_a"][key], run["summary_b"][key])
                self.assertEqual((run["a"] / "model.onnx").read_bytes(), (run["b"] / "model.onnx").read_bytes())
                self.assertIn(f"completed_steps={run['interrupted_state']['completed_steps']}/", run["resumption"].stderr)

    def assert_close_nested(self, left, right):
        if isinstance(left, dict):
            self.assertEqual(left.keys(), right.keys())
            for key in left:
                self.assert_close_nested(left[key], right[key])
        elif isinstance(left, list):
            self.assertEqual(len(left), len(right))
            for a, b in zip(left, right, strict=True):
                self.assert_close_nested(a, b)
        elif isinstance(left, float):
            torch.testing.assert_close(torch.tensor(left, dtype=torch.float32), torch.tensor(right, dtype=torch.float32))
        elif isinstance(left, torch.Tensor):
            torch.testing.assert_close(left, right)
        else:
            self.assertEqual(left, right)

    def test_one_and_two_ranks_have_the_same_updates_and_batch_norm_statistics(self):
        states = {}
        for n in (1, 2):
            out = self.root / f"short-{n}"
            args = train_arguments(self.initial / "checkpoint.pt", out)
            args[args.index("--batch-size") + 1] = "512"
            args[args.index("--micro-batch-size") + 1] = "128"
            require_success(run_cli(args, n))
            state = torch.load(out / "checkpoint.pt", weights_only=True)
            self.assertEqual(state["sample_count"], 2572)
            self.assertEqual(state["completed_steps"], 5)
            self.assertEqual(state["total_steps"], 5)
            self.assertEqual(state["provenance"]["world_size"], n)
            self.assertNotIn("replay_threads", state["config"])
            self.assertEqual({p.name for p in out.iterdir()}, {"checkpoint.pt", "model.onnx", "summary.json"})
            states[n] = state
        one, two = (states[n] for n in (1, 2))
        for key in ("data_order_sha256", "completed_steps", "total_steps", "positions_seen", "samples_seen",
                    "sample_count", "sample_sha256", "training_count", "validation_count"):
            self.assertEqual(one[key], two[key], key)
        buffers = ("running_mean", "running_var", "num_batches_tracked")
        for name in one["model"]:
            with self.subTest(name=name, buffer=name.endswith(buffers)):
                if name.endswith("num_batches_tracked"):
                    self.assertTrue(torch.equal(one["model"][name], two["model"][name]))
                else:
                    torch.testing.assert_close(one["model"][name], two["model"][name])
        for key in ("optimizer", "loss_sums", "learning_curve"):
            self.assert_close_nested(one[key], two[key])

    def test_resume_can_change_world_size_and_replay_threads(self):
        out = self.root / "changed-world-size"
        shutil.copytree(self.partial, out)
        result = run_cli(["train", "--resume", str(out), "--device", "cpu", "--replay-threads", "2"], 2)
        require_success(result)
        state = torch.load(out / "checkpoint.pt", weights_only=True)
        self.assertEqual(state["provenance"]["world_size"], 1)
        self.assertEqual(state["completed_steps"], state["total_steps"])
        for key in ("data_order_sha256", "completed_steps", "total_steps", "positions_seen", "samples_seen"):
            self.assertEqual(state[key], self.state_a[key])

    def test_data_list_stores_only_paths_and_resume_does_not_read_the_list(self) -> None:
        self.assertFalse(self.b.with_name(self.b.name + "-data.json").exists())
        for saved in (self.state_a, self.state_b, self.summary_a, self.summary_b):
            self.assertNotIn("data_list", saved["config"])
            self.assertEqual(saved["config"]["data"], ["fixtures/zero-selfplay-small.mnsd"])
            self.assertEqual([item["path"] for item in saved["provenance"]["files"]], saved["config"]["data"])

    def test_order_digest_counts_and_batch_normalization_follow_the_contract(self) -> None:
        training = np.flatnonzero(~self.data.validation)
        ordered = reference_sample(len(training), 4 * len(training), 42)
        digest = bytes(32)
        for start in range(0, len(ordered) // 16 * 16, 16):
            digest = hashlib.sha256(digest + b"".join(struct.pack("<q", index) for index in ordered[start:start + 16])).digest()
        updates = 4 * len(training) // 16
        self.assertGreaterEqual(updates, 30)
        self.assertNotEqual(4 * len(training) % 16, 0)
        for state in (self.state_a, self.state_b, self.summary_a, self.summary_b):
            self.assertEqual(state["data_order_sha256"], digest.hex())
            self.assertEqual(state["completed_steps"], updates)
            self.assertEqual(state["total_steps"], updates)
            self.assertEqual(state["positions_seen"], 16 * updates)
            self.assertEqual(state["sample_count"], 4 * len(training))
            self.assertEqual(state["samples_seen"], state["samples_seen_start"] + 16 * updates)
        checkpoints = list(range(4, updates + 1, 4))
        if checkpoints[-1] != updates:
            checkpoints.append(updates)
        self.assertEqual([point["completed_steps"] for point in self.summary_a["learning_curve"]], checkpoints)
        self.assertEqual(sum(line.startswith("steps=") for line in self.uninterrupted.stderr.splitlines()), len(checkpoints))
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
        require_success(run_cli(["train", "--resume", str(out), "--device", "cpu", "--replay-threads", "1"]))
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

    def test_init_ignores_legacy_fields_and_carries_momentum_with_new_settings(self) -> None:
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
        data, _, _, _ = read_replay_buffer([path], [path], 42,
            rank=0, world_size=1, batch_size=1, micro_batch_size=1, replay_threads=1,
        )
        indices = np.flatnonzero(~data.validation)
        indices = indices[reference_sample(len(indices), 4 * len(indices), 42)]
        model = ZeroNet(1, 16).train()
        model.load_state_dict(self.state_a["model"])
        inputs = encode_planes(*(torch.from_numpy(column[indices]) for column in (data.boards, data.sides, data.lions)))
        logits = model.forward_logits(inputs)
        policy, value = logits.policy, logits.value[:, 0]
        # 参照計算では密な方策と、結果・根の価値に対する別々のBCEを使う。
        targets = torch.zeros(8, 35_856)
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
        legal = torch.zeros_like(targets)
        for row, index in enumerate(indices):
            labels = data.legal_labels[data.legal_offsets[index]:data.legal_offsets[index + 1]]
            legal[row, torch.tensor(labels.astype(np.int64))] = 1
        softened = (targets + 1e-7).pow(0.25) * legal
        softened /= softened.sum(dim=1, keepdim=True)
        soft_loss = -(softened[:, mask] * logits.soft_policy[:, mask].log_softmax(1)).sum(1).mean()
        # この棋譜は投了の最終項目だけなので、3つの短期目標はすべて対局結果となる。
        short_loss = sum(torch.nn.functional.binary_cross_entropy_with_logits(logits.value[:, column], results)
                         for column in (1, 2, 3))
        loss = policy_loss + value_loss + 8 * soft_loss + short_loss
        loss += 0 * (logits.opponent_policy.sum() + logits.opponent_soft_policy.sum())
        loss.backward()
        norm = sum(parameter.grad.double().square().sum() for parameter in model.parameters()).sqrt().item()
        self.assertGreater(norm, 0.1)
        expected_parameters, expected_buffers = [], []
        old_optimizer = self.state_a["optimizer"]
        rate = 0.03 * (self.state_a["samples_seen"] + 8) / 2_000_000
        for parameter, identifier in zip(model.parameters(), old_optimizer["param_groups"][0]["params"]):
            old_buffer = old_optimizer["state"][identifier]["momentum_buffer"].double()
            gradient = parameter.grad.double() * (0.1 / norm) + 0.0002 * parameter.detach().double()
            buffer = 0.8 * old_buffer + gradient
            expected_buffers.append(buffer)
            expected_parameters.append(parameter.detach().double() - rate * (gradient + 0.8 * buffer))

        out = self.root / "next-cycle"
        # 本線のサイクル21〜25から26への移行を守る試験。余分な設定は引き継がず、
        # 余分な重みを読み込めない状態でも、通常の重みとモーメンタムで次の学習を始める。
        legacy = torch.load(self.a / "checkpoint.pt", weights_only=True)
        legacy["ema"] = {"model": {}, "samples_since_update": 0, "updates": 1}
        obsolete_settings = {"ema_period_samples": 500_000, "ema_scale": 8, "lr_schedule": "constant"}
        legacy["config"].update(obsolete_settings)
        del legacy["config"]["micro_batch_size"]
        initial = self.root / "legacy-checkpoint.pt"
        torch.save(legacy, initial)
        arguments = train_arguments(initial, out)
        set_training_data(arguments, [str(path)])
        for flag, value in (("--latest-data", str(path)), ("--cycle", "2"), ("--batch-size", "8"),
                            ("--micro-batch-size", "8"), ("--lr", "0.03"), ("--momentum", "0.8"), ("--weight-decay", "0.0002"),
                            ("--clip-grad-norm", "0.1")):
            arguments[arguments.index(flag) + 1] = value
        require_success(run_cli(arguments))
        state = torch.load(out / "checkpoint.pt", weights_only=True)
        summary = json.loads((out / "summary.json").read_text())
        self.assertNotIn("ema", state)
        for record in (state["config"], summary["config"], summary):
            self.assertTrue(obsolete_settings.keys().isdisjoint(record))
        self.assertEqual({path.name for path in out.iterdir()}, {"checkpoint.pt", "model.onnx", "summary.json"})
        self.assertEqual(state["cycle"], 2)
        self.assertEqual(state["completed_steps"], 1)
        self.assertEqual(state["positions_seen"], 8)
        group = state["optimizer"]["param_groups"][0]
        self.assertEqual((group["lr"], group["momentum"], group["weight_decay"], group["nesterov"]), (rate, 0.8, 0.0002, True))
        names = [name for name, _ in model.named_parameters()]
        for name, identifier, expected_parameter, expected_buffer in zip(
            names, group["params"], expected_parameters, expected_buffers,
        ):
            torch.testing.assert_close(state["model"][name].double(), expected_parameter, rtol=2e-6, atol=2e-7)
            torch.testing.assert_close(state["optimizer"]["state"][identifier]["momentum_buffer"].double(), expected_buffer,
                                       rtol=1e-5, atol=2e-6)
        for key, expected in (("policy", policy_loss), ("value", value_loss), ("value_result", result_loss), ("soft_policy", soft_loss)):
            self.assertAlmostEqual(summary["training_losses"][key], expected.item(), places=5)

    def test_finished_nonempty_unfinished_init_and_cpu_bf16_are_rejected(self) -> None:
        cases = [
            ["train", "--resume", str(self.a), "--device", "cpu", "--replay-threads", "1"],
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
                self.assertEqual(sum(line.startswith("minase_zero:") for line in result.stderr.splitlines()), 1, result.stderr)

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
                result = run_cli(["train", "--resume", str(out), "--device", "cpu", "--replay-threads", "1"])
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(sum(line.startswith("minase_zero:") for line in result.stderr.splitlines()), 1, result.stderr)
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
        state["config"]["latest_data"] = [str(data_path)]
        state["latest_data"] = [str(data_path)]
        state["provenance"]["files"][0]["path"] = str(data_path)
        recorded = state["provenance"]["files"][0]["sha256"]
        self.assertNotEqual(recorded, hashlib.sha256(content).hexdigest())
        torch.save(state, out / "checkpoint.pt")
        require_success(run_cli(["train", "--resume", str(out), "--device", "cpu", "--replay-threads", "1"]))
        summary = json.loads((out / "summary.json").read_text())
        self.assertEqual(summary["provenance"]["files"][0]["sha256"], recorded)
        self.assertEqual(summary["diagnostics"], self.summary_a["diagnostics"])

    def test_summary_contains_three_final_diagnostics_and_loading_measurements(self) -> None:
        seed = self.infos[0].header.seed
        for name, selected, remainder in (("training_sample", self.data.training_sample, 1),
                                          ("latest_training_sample", self.data.training_sample, 1),
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
        set_training_data(arguments, [str(path)])
        arguments[arguments.index("--latest-data") + 1] = str(path)
        arguments[arguments.index("--batch-size") + 1] = "4"
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
                 ["train", "--resume", str(self.partial), "--device", "cpu", "--replay-threads", "1", "--seed", "1"]]
        for arguments in cases:
            with self.subTest(arguments=arguments):
                result = run_cli(arguments)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(sum(line.startswith("minase_zero:") for line in result.stderr.splitlines()), 1, result.stderr)


class DataListTests(unittest.TestCase):
    def test_invalid_data_lists_report_filename_and_one_error_line(self):
        with tempfile.TemporaryDirectory(prefix="minase-data-list-") as directory:
            root = Path(directory)
            arguments = train_arguments(root / "unused.pt", root / "out")
            path = Path(arguments[arguments.index("--data-list") + 1])
            for content in (b"[]", b"{}", b"[", b'["valid.mnsd", 7]', b'[""]', b"\xff"):
                with self.subTest(content=content):
                    path.write_bytes(content)
                    result = run_cli(arguments)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    self.assertEqual(sum(line.startswith("minase_zero:") for line in result.stderr.splitlines()), 1, result.stderr)
                    self.assertIn(str(path), result.stderr)
                    self.assertFalse((root / "out").exists())
            path.unlink()
            result = run_cli(arguments)
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn(str(path), result.stderr)


if __name__ == "__main__":
    unittest.main()
