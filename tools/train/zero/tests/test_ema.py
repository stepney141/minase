"""brief-ema.mdの周期、初期値、引き継ぎ、および出力の契約を検証する。

周期3、係数2の例は手計算した値で確かめる。2標本ずつの実際の学習では、
最初のサイクルは更新0回、次は1回、その次は2回となる。
BatchNormの係数だけを平均し、統計は更新時に複写するというKataGoの契約も固定する。
中断からの再開による両ONNXのバイト一致はtest_train.pyの実プロセス試験で検証する。
"""

import copy
import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch
from torch import nn

from minase_zero.__main__ import _parse
from minase_zero.data import is_training_sample, is_validation, read_window
from minase_zero.encoding import policy_mask
from minase_zero.net import Ema, ZeroNet, validate_ema
from minase_zero.train import diagnose, export_onnx, fixture_eval, initialize, resume, train_new

from .cli import COMMIT, ZERO, require_success, run_cli, train_arguments
from .test_data import single_game, write_small


class EmaTests(unittest.TestCase):
    def test_period_boundaries_preserve_remainder_and_apply_every_update(self) -> None:
        model = nn.Linear(1, 1, bias=False)
        with torch.no_grad():
            model.weight.fill_(2)
        ema = Ema(model, 3, 2)
        # 2標本では更新せず、3標本で(2+10)/2=6になる。
        with torch.no_grad():
            model.weight.fill_(6)
        ema.update(model, 2)
        self.assertEqual(ema.model.weight.item(), 2)
        self.assertEqual((ema.updates, ema.samples_since_update), (0, 2))
        with torch.no_grad():
            model.weight.fill_(10)
        ema.update(model, 1)
        self.assertEqual(ema.model.weight.item(), 6)
        self.assertEqual((ema.updates, ema.samples_since_update), (1, 0))
        # 7標本は周期2つと余り1。6から10、12の順に更新する。
        with torch.no_grad():
            model.weight.fill_(14)
        ema.update(model, 7)
        self.assertEqual(ema.model.weight.item(), 12)
        self.assertEqual((ema.updates, ema.samples_since_update), (3, 1))

    def test_batchnorm_parameters_are_averaged_but_buffers_are_copied_only_at_updates(self) -> None:
        model = nn.BatchNorm1d(1)
        ema = Ema(model, 3, 2)
        with torch.no_grad():
            model.weight.fill_(5)
            model.bias.fill_(4)
            model.running_mean.fill_(20)
            model.running_var.fill_(30)
            model.num_batches_tracked.fill_(7)
        ema.update(model, 2)
        self.assertEqual(ema.model.running_mean.item(), 0)
        self.assertEqual(ema.model.running_var.item(), 1)
        self.assertEqual(ema.model.num_batches_tracked.item(), 0)
        ema.update(model, 1)
        self.assertEqual(ema.model.weight.item(), 3)
        self.assertEqual(ema.model.bias.item(), 2)
        self.assertEqual(ema.model.running_mean.item(), 20)
        self.assertEqual(ema.model.running_var.item(), 30)
        self.assertEqual(ema.model.num_batches_tracked.item(), 7)
        with torch.no_grad():
            model.running_mean.fill_(40)
            model.num_batches_tracked.fill_(8)
        ema.update(model, 1)
        self.assertEqual(ema.model.running_mean.item(), 20)
        self.assertEqual(ema.model.num_batches_tracked.item(), 7)
        self.assertFalse(ema.model.training)
        self.assertTrue(all(not parameter.requires_grad for parameter in ema.model.parameters()))

    def test_saved_average_restores_weights_remainder_and_update_count(self) -> None:
        model = nn.Linear(1, 1, bias=False)
        with torch.no_grad():
            model.weight.fill_(2)
        ema = Ema(model, 3, 2)
        with torch.no_grad():
            model.weight.fill_(10)
        ema.update(model, 5)
        with tempfile.TemporaryDirectory() as directory:
            checkpoint = Path(directory) / "ema.pt"
            torch.save(ema.state_dict(), checkpoint)
            restored = Ema(model, 3, 2)
            restored.load_state_dict(torch.load(checkpoint, weights_only=True))
        self.assertEqual(restored.model.weight.item(), 6)
        self.assertEqual((restored.updates, restored.samples_since_update), (1, 2))
        with torch.no_grad():
            model.weight.fill_(14)
        restored.update(model, 1)
        self.assertEqual(restored.model.weight.item(), 10)
        self.assertEqual((restored.updates, restored.samples_since_update), (2, 0))

    def test_settings_require_both_zero_or_both_positive_finite_values(self) -> None:
        for period, scale in ((0, 0), (500_000, 8), (3, 0.5)):
            validate_ema(period, scale)
        for period, scale in ((0, 8), (3, 0), (-1, 2), (3, -1), (1.5, 2),
                              (True, 2), (3, float("inf")), (3, float("nan"))):
            with self.subTest(period=period, scale=scale), self.assertRaises(ValueError):
                validate_ema(period, scale)


class EmaTrainingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        previous_threads = torch.get_num_threads()
        torch.set_num_threads(1)
        cls.addClassCleanup(torch.set_num_threads, previous_threads)
        directory = tempfile.TemporaryDirectory(prefix="minase-ema-tests-")
        cls.addClassCleanup(directory.cleanup)
        cls.root = Path(directory.name)
        cls.initial = cls.root / "initial"
        initialize(1, 2, 23, cls.initial, COMMIT)
        cls.data_path = cls.root / "two-training-positions.mnsd"
        numbers = np.arange(100, dtype=np.uint32)
        validation = is_validation(20261038, numbers)
        sample = is_training_sample(20261038, numbers)
        games = sorted([int(numbers[sample][0]), int(numbers[~(validation | sample)][0]),
                        int(numbers[validation][0])])
        write_small(cls.data_path, [single_game(game, 0, 2, 32768, (65_535,)) for game in games], "engine-default")
        args = vars(_parse(train_arguments(cls.initial / "checkpoint.pt", cls.root / "unused")))
        args.pop("command")
        args.pop("out_dir")
        cls.config = {**args, "data": [str(cls.data_path)], "latest_data": [str(cls.data_path)],
                      "batch_size": 2, "ema_period_samples": 3, "ema_scale": 2}
        cls.states = [torch.load(cls.initial / "checkpoint.pt", weights_only=True)]
        cls.summaries = []
        for cycle in (1, 2, 3):
            initial = cls.initial if cycle == 1 else cls.root / f"cycle-{cycle - 1}"
            out = cls.root / f"cycle-{cycle}"
            train_new({**cls.config, "init": str(initial / "checkpoint.pt"), "cycle": cycle}, out)
            cls.states.append(torch.load(out / "checkpoint.pt", weights_only=True))
            cls.summaries.append(json.loads((out / "summary.json").read_text()))

    def test_initial_weights_and_sample_count_survive_a_cycle_without_an_update(self) -> None:
        self.assertNotIn("ema", self.states[0])
        ema = self.states[1]["ema"]
        self.assertEqual((ema["updates"], ema["samples_since_update"]), (0, 2))
        for name, value in self.states[0]["model"].items():
            self.assertTrue(torch.equal(value, ema["model"][name]), name)
        self.assertFalse(torch.equal(self.states[0]["model"]["trunk.0.weight"],
                                     self.states[1]["model"]["trunk.0.weight"]))
        self.assertEqual((self.root / "cycle-1/model-ema.onnx").read_bytes(),
                         (self.initial / "model.onnx").read_bytes())

    def test_average_remainder_and_update_count_continue_across_cycles(self) -> None:
        names = dict(ZeroNet(1, 2).named_parameters())
        second, third = self.states[2], self.states[3]
        self.assertEqual((second["ema"]["updates"], second["ema"]["samples_since_update"]), (1, 1))
        self.assertEqual((third["ema"]["updates"], third["ema"]["samples_since_update"]), (2, 0))
        for name in names:
            initial = self.states[0]["model"][name]
            expected_second = 0.5 * initial + 0.5 * second["model"][name]
            expected_third = 0.25 * initial + 0.25 * second["model"][name] + 0.5 * third["model"][name]
            torch.testing.assert_close(second["ema"]["model"][name], expected_second)
            torch.testing.assert_close(third["ema"]["model"][name], expected_third)
        for state in (second, third):
            for name in state["model"].keys() - names.keys():
                self.assertTrue(torch.equal(state["ema"]["model"][name], state["model"][name]), name)
        for summary, updates in zip(self.summaries, (0, 1, 2), strict=True):
            self.assertEqual((summary["ema_period_samples"], summary["ema_scale"], summary["ema_updates"]),
                             (3, 2, updates))
            onnx = self.root / f"cycle-{summary['cycle']}/model-ema.onnx"
            self.assertEqual(summary["ema_network_sha256"], hashlib.sha256(onnx.read_bytes()).hexdigest())

    def test_disabled_average_is_not_created_or_carried_from_init(self) -> None:
        for initial in (self.initial, self.root / "cycle-3"):
            with self.subTest(initial=initial):
                out = self.root / f"disabled-{initial.name}"
                train_new({**self.config, "init": str(initial / "checkpoint.pt"),
                           "ema_period_samples": 0, "ema_scale": 0}, out)
                state = torch.load(out / "checkpoint.pt", weights_only=True)
                summary = json.loads((out / "summary.json").read_text())
                self.assertNotIn("ema", state)
                self.assertFalse((out / "model-ema.onnx").exists())
                self.assertFalse(any(name.startswith("ema_") for name in summary))
        self.assertEqual((self.root / "disabled-initial/model.onnx").read_bytes(),
                         (self.root / "cycle-1/model.onnx").read_bytes())

    def test_invalid_ema_settings_fail_before_creating_training_output(self) -> None:
        for period, scale in ((0, 2), (3, 0), (-3, 2), (3, float("nan"))):
            out = self.root / "invalid-settings"
            with self.subTest(period=period, scale=scale), self.assertRaises(ValueError):
                train_new({**self.config, "ema_period_samples": period, "ema_scale": scale}, out)
            self.assertFalse(out.exists())

    def test_all_three_diagnostics_use_saved_average_and_the_same_samples(self) -> None:
        data, _, _ = read_window([self.data_path], [self.data_path], 42)
        state = self.states[2]
        model = ZeroNet(1, 2)
        model.load_state_dict(state["ema"]["model"])
        mask = torch.from_numpy(policy_mask())
        summary = self.summaries[1]
        for name, membership in (("training_sample", data.training_sample),
                                  ("latest_training_sample", data.training_sample),
                                  ("validation", data.validation)):
            actual = summary["ema_diagnostics"][name]
            for key in ("game_count", "position_count", "hash_modulus", "hash_remainder", "games"):
                self.assertEqual(actual[key], summary["diagnostics"][name][key])
            expected = diagnose(model, data, np.flatnonzero(membership), torch.device("cpu"), self.config, mask)
            for key, value in expected.items():
                self.assertEqual(actual[key], value)
        self.assertNotEqual(summary["ema_diagnostics"], summary["diagnostics"])

    def test_fixture_eval_selects_average_and_rejects_raw_or_missing_weights(self) -> None:
        out = self.root / "ema-fixture.json"
        trained = self.root / "cycle-2"
        require_success(run_cli([
            "fixture-eval", "--weights", "ema", "--checkpoint", str(trained / "checkpoint.pt"),
            "--onnx", str(trained / "model-ema.onnx"), "--fixture", "fixtures/nn-positions.jsonl", "--out", str(out),
        ]))
        result = json.loads(out.read_text())
        self.assertEqual(result["network_sha256"], self.summaries[1]["ema_network_sha256"])
        self.assertEqual(len(result["positions"]), 288)
        for checkpoint, onnx, weights in (
            (trained / "checkpoint.pt", trained / "model.onnx", "ema"),
            (trained / "checkpoint.pt", trained / "model-ema.onnx", "raw"),
            (self.initial / "checkpoint.pt", self.initial / "model.onnx", "ema"),
        ):
            rejected = self.root / "rejected-fixture.json"
            with self.subTest(weights=weights, onnx=onnx), self.assertRaises(ValueError):
                fixture_eval(checkpoint, onnx, ZERO / "fixtures/nn-positions.jsonl", rejected, weights)
            self.assertFalse(rejected.exists())

    def test_failed_ema_export_leaves_no_completion_marker_and_can_resume(self) -> None:
        out = self.root / "export-interrupted"

        def export(model, path):
            if (out / "model.onnx").exists():
                raise OSError("interrupted EMA export")
            export_onnx(model, path)

        with patch("minase_zero.train.export_onnx", side_effect=export), self.assertRaises(OSError):
            train_new(self.config, out)
        self.assertTrue((out / "checkpoint.pt").is_file())
        self.assertTrue((out / "model.onnx").is_file())
        self.assertFalse((out / "summary.json").exists())
        self.assertFalse((out / "model-ema.onnx").exists())
        resume(out, "cpu")
        for name in ("model.onnx", "model-ema.onnx"):
            self.assertEqual((out / name).read_bytes(), (self.root / "cycle-1" / name).read_bytes())

    def test_final_checkpoint_diagnostics_and_export_preserve_average_buffers(self) -> None:
        out = self.root / "final-checkpoint"
        shutil.copytree(self.root / "cycle-2", out)
        (out / "summary.json").unlink()
        state = copy.deepcopy(self.states[2])
        state["model"]["trunk.1.running_mean"].add_(3)
        torch.save(state, out / "checkpoint.pt")
        resume(out, "cpu")
        summary = json.loads((out / "summary.json").read_text())
        self.assertEqual(summary["ema_diagnostics"], self.summaries[1]["ema_diagnostics"])
        self.assertNotEqual(summary["diagnostics"], self.summaries[1]["diagnostics"])
        self.assertEqual((out / "model-ema.onnx").read_bytes(), (self.root / "cycle-2/model-ema.onnx").read_bytes())

    def test_cli_requires_each_ema_option_and_fixture_weight_selection(self) -> None:
        args = train_arguments(self.initial / "checkpoint.pt", self.root / "unused")
        for flag in ("--ema-period-samples", "--ema-scale"):
            index = args.index(flag)
            with self.subTest(flag=flag), self.assertRaises(ValueError):
                _parse(args[:index] + args[index + 2:])
        with self.assertRaises(ValueError):
            _parse(["fixture-eval", "--checkpoint", "checkpoint.pt", "--onnx", "model.onnx",
                    "--fixture", "positions.jsonl", "--out", "out.json"])


if __name__ == "__main__":
    unittest.main()
