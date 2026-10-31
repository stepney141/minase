"""指示書の学習率の式、一定の学習、サイクルのリセットと再開をCPUで検証する。

T=4のcosineの倍率は1、(2+√2)/4、1/2、(2−√2)/4であり、最後も正となる。
実際の最適化直前の値と保存区間の末尾を照合する。constantにも累計標本数のウォームアップを掛け、
中断後の通常重みと指数移動平均のONNXは連続学習とのバイト一致で確かめる。
"""

import json
import math
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

from minase_zero import train
from minase_zero.__main__ import _parse
from minase_zero.data import is_training_sample, is_validation

from .cli import COMMIT, require_success, run_cli, train_arguments
from .test_data import single_game, write_small


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
        cls.data_path = cls.root / "seven-training-positions.mnsd"
        numbers = np.arange(100, dtype=np.uint32)
        validation = is_validation(20261038, numbers)
        sample = is_training_sample(20261038, numbers)
        games = sorted([int(numbers[sample][0]), *map(int, numbers[~(validation | sample)][:6]),
                        int(numbers[validation][0])])
        write_small(cls.data_path, [single_game(game, game % 2, game % 3, 32768, (65_535,))
                                   for game in games], "engine-default")
        args = vars(_parse(train_arguments(cls.initial / "checkpoint.pt", cls.root / "unused")))
        args.pop("command")
        args.pop("out_dir")
        cls.config = {**args, "data": [str(cls.data_path)], "latest_data": [str(cls.data_path)],
                      "batch_size": 2, "checkpoint_steps": 2, "ema_period_samples": 3, "ema_scale": 2}

    def test_every_group_uses_the_hand_calculated_rate_before_each_update(self):
        sgd = torch.optim.SGD
        for schedule in ("constant", "cosine"):
            for batch_size, multipliers in ((32, [1]), (16, [1, 0.5]),
                                            (10, [1, 0.75, 0.25]),
                                            (7, [1, (2 + math.sqrt(2)) / 4, 0.5, (2 - math.sqrt(2)) / 4])):
                with self.subTest(schedule=schedule, batch_size=batch_size):
                    rates = []

                    def optimizer(parameters, **kwargs):
                        parameters = list(parameters)
                        result = sgd([{"params": parameters[::2], "lr": 0.7},
                                      {"params": parameters[1::2], "lr": 0.9}], **kwargs)
                        result.register_step_pre_hook(
                            lambda opt, args, kw: rates.append([group["lr"] for group in opt.param_groups]))
                        return result

                    out = self.root / f"rates-{schedule}-{batch_size}"
                    with patch("minase_zero.train.torch.optim.SGD", side_effect=optimizer):
                        train.train_new({**self.config, "lr_schedule": schedule, "batch_size": batch_size}, out)
                    expected = ([0.01 * multiplier for multiplier in multipliers] if schedule == "cosine"
                                else [0.01] * len(multipliers))
                    expected = [rate * min(1, (step + 1) * batch_size / 2_000_000)
                                for step, rate in enumerate(expected)]
                    self.assertEqual(len(rates), len(expected))
                    for groups, rate in zip(rates, expected, strict=True):
                        self.assertEqual(len(groups), 2)
                        for actual in groups:
                            self.assertAlmostEqual(actual, rate, places=15)
                            self.assertGreater(actual, 0)
                    state = torch.load(out / "checkpoint.pt", weights_only=True)
                    summary = json.loads((out / "summary.json").read_text())
                    for record in (state["config"], summary["config"], summary):
                        self.assertEqual(record["lr_schedule"], schedule)
                        self.assertEqual(record["lr"], 0.01)
                    self.assertEqual(summary["total_steps"], len(expected))
                    ends = {1: [1], 2: [2], 3: [2, 3], 4: [2, 4]}[len(expected)]
                    self.assertEqual([point["completed_steps"] for point in summary["learning_curve"]], ends)
                    for point, end in zip(summary["learning_curve"], ends, strict=True):
                        self.assertAlmostEqual(point["lr"], expected[end - 1], places=15)
                    self.assertEqual(state["learning_curve"], summary["learning_curve"])

    def test_warmup_rates_cross_two_million_samples(self):
        # B=256、s=1,999,488から始めると、倍率は0.999872、1、1、1となる。
        multipliers = [1, (2 + math.sqrt(2)) / 4, 0.5, (2 - math.sqrt(2)) / 4]
        for schedule in ("constant", "cosine"):
            for step, warmup in enumerate((0.999872, 1, 1, 1)):
                expected = 0.01 * warmup * (multipliers[step] if schedule == "cosine" else 1)
                actual = train.learning_rate(0.01, schedule, step, 4, 1_999_488 + step * 256, 256)
                self.assertAlmostEqual(actual, expected, places=15)

    def test_cycle_boundary_preserves_warmup_and_reaches_the_limit(self):
        state = torch.load(self.initial / "checkpoint.pt", weights_only=True)
        state["samples_seen"] = 1_999_970
        initial = self.root / "near-warmup.pt"
        torch.save(state, initial)
        for cycle in (1, 2):
            out = self.root / f"warmup-boundary-{cycle}"
            train.train_new({**self.config, "init": str(initial), "cycle": cycle,
                             "lr_schedule": "constant", "checkpoint_steps": 1}, out)
            summary = json.loads((out / "summary.json").read_text())
            start = 1_999_970 + (cycle - 1) * 28
            self.assertEqual(summary["samples_seen_start"], start)
            self.assertEqual(summary["samples_seen"], start + 28)
            self.assertEqual(summary["positions_seen"], 28)
            for step, point in enumerate(summary["learning_curve"]):
                expected = 0.01 * min(1, (start + (step + 1) * 2) / 2_000_000)
                self.assertAlmostEqual(point["lr"], expected, places=15)
                self.assertEqual(point["samples_seen_start"], start)
                self.assertEqual(point["samples_seen"], start + (step + 1) * 2)
            self.assertEqual(summary["last_lr"], summary["learning_curve"][-1]["lr"])
            initial = out / "checkpoint.pt"

    def test_cosine_restarts_while_warmup_continues_for_each_cycle(self):
        initial = self.initial / "checkpoint.pt"
        for cycle in (1, 2):
            out = self.root / f"restart-{cycle}"
            train.train_new({**self.config, "init": str(initial), "cycle": cycle,
                             "lr_schedule": "cosine", "checkpoint_steps": 1}, out)
            curve = json.loads((out / "summary.json").read_text())["learning_curve"]
            self.assertAlmostEqual(curve[0]["lr"], 0.01 * ((cycle - 1) * 28 + 2) / 2_000_000, places=15)
            self.assertAlmostEqual(curve[-1]["lr"], 0.01 * 0.5 * (1 + math.cos(math.pi * 13 / 14)) * (cycle * 28 / 2_000_000), places=15)
            initial = out / "checkpoint.pt"

    def test_resume_preserves_onnx_bytes_and_learning_curve_with_and_without_ema(self):
        save = train._save_checkpoint

        def interrupt(path, state):
            save(path, state)
            if state["completed_steps"] == 2:
                raise OSError("test interruption after a committed checkpoint")

        initial_state = torch.load(self.initial / "checkpoint.pt", weights_only=True)
        initial_state["samples_seen"] = 1_999_995
        near_limit = self.root / "resume-near-warmup.pt"
        torch.save(initial_state, near_limit)
        for schedule in ("constant", "cosine"):
            for period, scale in ((0, 0), (3, 2)):
                with self.subTest(schedule=schedule, ema_period_samples=period):
                    config = {**self.config, "init": str(near_limit), "lr_schedule": schedule,
                              "ema_period_samples": period, "ema_scale": scale}
                    continuous = self.root / f"continuous-{schedule}-{period}"
                    interrupted = self.root / f"interrupted-{schedule}-{period}"
                    train.train_new(config, continuous)
                    with patch("minase_zero.train._save_checkpoint", side_effect=interrupt), self.assertRaises(OSError):
                        train.train_new(config, interrupted)
                    checkpoint = torch.load(interrupted / "checkpoint.pt", weights_only=True)
                    self.assertEqual((checkpoint["completed_steps"], checkpoint["total_steps"]), (2, 14))
                    self.assertFalse((interrupted / "summary.json").exists())
                    require_success(run_cli(["train", "--resume", str(interrupted), "--device", "cpu"]))
                    expected = json.loads((continuous / "summary.json").read_text())
                    resumed = json.loads((interrupted / "summary.json").read_text())
                    for key in ("learning_curve", "training_losses", "diagnostics", "data_order_sha256",
                                "samples_seen", "samples_seen_start", "last_lr"):
                        self.assertEqual(resumed[key], expected[key])
                    names = ["model.onnx"]
                    if period:
                        names.append("model-ema.onnx")
                        self.assertEqual(resumed["ema_diagnostics"], expected["ema_diagnostics"])
                    else:
                        self.assertFalse((interrupted / "model-ema.onnx").exists())
                    for name in names:
                        self.assertEqual((interrupted / name).read_bytes(), (continuous / name).read_bytes())
                        (interrupted / name).unlink()
                    # 全更新完了後の再開でも、最後の学習率のまま出力を再生成できる。
                    (interrupted / "summary.json").unlink()
                    train.resume(interrupted, "cpu")
                    for name in names:
                        self.assertEqual((interrupted / name).read_bytes(), (continuous / name).read_bytes())

    def test_schedule_is_required_and_invalid_values_fail_before_writing(self):
        arguments = train_arguments(self.initial / "checkpoint.pt", self.root / "invalid")
        index = arguments.index("--lr-schedule")
        for values in ([], ["--lr-schedule", "linear"]):
            with self.subTest(values=values), self.assertRaises(ValueError):
                _parse(arguments[:index] + values + arguments[index + 2:])
        out = self.root / "invalid"
        with self.assertRaises(ValueError):
            train.train_new({**self.config, "lr_schedule": "linear"}, out)
        self.assertFalse(out.exists())


if __name__ == "__main__":
    unittest.main()
