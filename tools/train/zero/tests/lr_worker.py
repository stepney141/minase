"""250更新のウォームアップと、サイクル・再開をまたぐ累計更新数を検証する。

実際の最適化直前の学習率と保存区間の末尾を照合する。
中断後のONNXは連続学習とのバイト一致で確かめる。
"""

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

from minase_zero import train
from minase_zero.__main__ import _parse
from minase_zero.data import is_training_sample, is_validation

from tests.cli import COMMIT, set_training_data, train_arguments
from tests.test_data import single_game, write_small


class LearningRateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        threads = torch.get_num_threads()
        torch.set_num_threads(1)
        cls.addClassCleanup(torch.set_num_threads, threads)
        directory = tempfile.TemporaryDirectory(prefix="minase-lr-tests-")
        cls.addClassCleanup(directory.cleanup)
        cls.root = Path(directory.name)
        cls.initial = cls.root / "initial"
        train.initialize(1, 2, 23, cls.initial, COMMIT)
        cls.data_path = cls.root / "eight-training-positions.mnsd"
        numbers = np.arange(100, dtype=np.uint32)
        validation = is_validation(20261038, numbers)
        sample = is_training_sample(20261038, numbers)
        games = sorted([int(numbers[sample][0]), *map(int, numbers[~(validation | sample)][:7]),
                        int(numbers[validation][0])])
        write_small(cls.data_path, [single_game(game, game % 2, game % 3, 32768, (65_535,))
                                   for game in games], "lishogi")
        arguments = train_arguments(cls.initial / "checkpoint.pt", cls.root / "unused")
        set_training_data(arguments, [str(cls.data_path)])
        args = vars(_parse(arguments))
        args.pop("command")
        args.pop("out_dir")
        cls.config = {**args, "latest_data": [str(cls.data_path)],
                      "batch_size": 2, "micro_batch_size": 1, "checkpoint_steps": 2}

    def test_every_group_uses_the_hand_calculated_rate_before_each_update(self):
        sgd = torch.optim.SGD
        for batch_size, rates_in_units in ((8, [1]), (4, [1, 2]), (2, [1, 2, 3, 4])):
            with self.subTest(batch_size=batch_size):
                rates = []

                def optimizer(parameters, **kwargs):
                    parameters = list(parameters)
                    result = sgd([{"params": parameters[::2], "lr": 0.7},
                                  {"params": parameters[1::2], "lr": 0.9}], **kwargs)
                    result.register_step_pre_hook(
                        lambda opt, args, kw: rates.append([group["lr"] for group in opt.param_groups]))
                    return result

                out = self.root / f"rates-{batch_size}"
                with patch("minase_zero.train.torch.optim.SGD", side_effect=optimizer):
                    train.train_new({**self.config, "batch_size": batch_size}, out)
                expected = [0.01 * units / 250 for units in rates_in_units]
                self.assertEqual(len(rates), len(expected))
                for groups, rate in zip(rates, expected, strict=True):
                    self.assertEqual(len(groups), 2)
                    for actual in groups:
                        self.assertAlmostEqual(actual, rate, places=15)
                state = torch.load(out / "checkpoint.pt", weights_only=True)
                summary = json.loads((out / "summary.json").read_text())
                for record in (state["config"], summary["config"], summary):
                    self.assertEqual(record["lr"], 0.01)
                self.assertEqual(summary["total_steps"], len(expected))
                self.assertEqual(summary["optimizer_steps"], len(expected))
                self.assertEqual(summary["optimizer_steps_start"], 0)
                ends = {1: [1], 2: [2], 4: [2, 4]}[len(expected)]
                self.assertEqual([point["completed_steps"] for point in summary["learning_curve"]], ends)
                for point, end in zip(summary["learning_curve"], ends, strict=True):
                    self.assertAlmostEqual(point["lr"], expected[end - 1], places=15)
                    self.assertEqual(point["optimizer_steps"], end)
                self.assertEqual(state["learning_curve"], summary["learning_curve"])

    def test_cycle_boundary_preserves_warmup_and_base_rate_is_constant_afterward(self):
        state = torch.load(self.initial / "checkpoint.pt", weights_only=True)
        state["optimizer_steps"] = 245
        initial = self.root / "near-warmup.pt"
        torch.save(state, initial)
        for cycle, rates in ((1, [246 / 250, 247 / 250, 248 / 250, 249 / 250]),
                             (2, [1, 1, 1, 1])):
            out = self.root / f"warmup-boundary-{cycle}"
            train.train_new({**self.config, "init": str(initial), "cycle": cycle, "checkpoint_steps": 1}, out)
            summary = json.loads((out / "summary.json").read_text())
            start = 245 + (cycle - 1) * 4
            self.assertEqual(summary["optimizer_steps_start"], start)
            self.assertEqual(summary["optimizer_steps"], start + 4)
            self.assertEqual(summary["positions_seen"], 8)
            for step, (point, factor) in enumerate(zip(summary["learning_curve"], rates, strict=True)):
                self.assertAlmostEqual(point["lr"], 0.01 * factor, places=15)
                self.assertEqual(point["optimizer_steps_start"], start)
                self.assertEqual(point["optimizer_steps"], start + step + 1)
            self.assertEqual(summary["last_lr"], summary["learning_curve"][-1]["lr"])
            initial = out / "checkpoint.pt"

    def test_resume_preserves_onnx_bytes_and_learning_curve_and_rejects_changed_lr(self):
        save = train._save_checkpoint

        def interrupt(path, state):
            save(path, state)
            if state["completed_steps"] == 2:
                raise OSError("test interruption after a committed checkpoint")

        initial_state = torch.load(self.initial / "checkpoint.pt", weights_only=True)
        initial_state["optimizer_steps"] = 247
        near_limit = self.root / "resume-near-warmup.pt"
        torch.save(initial_state, near_limit)
        config = {**self.config, "init": str(near_limit)}
        continuous = self.root / "continuous"
        interrupted = self.root / "interrupted"
        train.train_new(config, continuous)
        with patch("minase_zero.train._save_checkpoint", side_effect=interrupt), self.assertRaises(OSError):
            train.train_new(config, interrupted)
        checkpoint = torch.load(interrupted / "checkpoint.pt", weights_only=True)
        self.assertEqual((checkpoint["completed_steps"], checkpoint["total_steps"]), (2, 4))
        self.assertEqual(checkpoint["optimizer_steps"], 249)
        self.assertEqual(checkpoint["optimizer_steps_start"], 247)
        self.assertEqual(checkpoint["config"]["lr"], 0.01)
        self.assertFalse((interrupted / "summary.json").exists())
        saved = (interrupted / "checkpoint.pt").read_bytes()
        with self.assertRaisesRegex(ValueError, "lr.*differs from checkpoint lr"):
            train.resume(interrupted, "cpu", 1, 0.02)
        self.assertEqual(saved, (interrupted / "checkpoint.pt").read_bytes())
        train.resume(interrupted, "cpu", 1, 0.01)
        expected = json.loads((continuous / "summary.json").read_text())
        resumed = json.loads((interrupted / "summary.json").read_text())
        for key in ("learning_curve", "training_losses", "diagnostics", "data_order_sha256",
                    "optimizer_steps", "optimizer_steps_start", "last_lr", "lr"):
            self.assertEqual(resumed[key], expected[key])
        self.assertEqual(resumed["optimizer_steps"], 251)
        self.assertEqual(resumed["lr"], 0.01)
        self.assertEqual(resumed["last_lr"], 0.01)
        self.assertEqual((interrupted / "model.onnx").read_bytes(), (continuous / "model.onnx").read_bytes())
        (interrupted / "model.onnx").unlink()
        # 全更新完了後の再開でも、最後の学習率のまま出力を再生成できる。
        (interrupted / "summary.json").unlink()
        train.resume(interrupted, "cpu", 1, 0.01)
        self.assertEqual((interrupted / "model.onnx").read_bytes(), (continuous / "model.onnx").read_bytes())


if __name__ == "__main__":
    unittest.main()
