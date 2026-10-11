"""初期値、鏡映、除去再計算、学習と停止、保存候補の契約をCPUで検証する。"""

from contextlib import redirect_stdout
import hashlib
from io import StringIO
import json
import struct
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

from helpers import direct_score, records_with, write_mnsd
from minase_train.data import mnpt
from minase_train.data.features import feature_indices, mirror
from minase_train.data.mnsd import Dataset
from minase_train.data.taper import phase_ratios, piece_counts
from minase_train.fm.train import feature_observations
from minase_train.nnue.format import read_mnue
from minase_train.nnue.model import NNUE, float_evaluate, load_initial
import minase_train.nnue.train as train


CPU = torch.device("cpu")


def fixture(root):
    """共有生成器で、同じ対局の2局面と非整数の鏡映PSTを含む小標本を作る。"""
    rng = np.random.default_rng(170)
    board = np.zeros((120, 144), dtype=np.uint8)
    for i, row in enumerate(board):
        squares = rng.choice(140, size=2 + i % 5, replace=False)
        row[squares] = rng.choice(np.array([1, 65, 2, 66, 47, 111], dtype=np.uint8), len(squares))
        row[142:144] = [12, 76]
    data = root / "data.bin"
    write_mnsd(data, seed=0, checksum=b"a" * 32, games=np.repeat(np.arange(60), 2).tolist(),
               scores=(np.arange(120) * 13 - 500).tolist(), board=board)
    half = rng.uniform(-0.2, 0.2, size=(95, 12, 6, 2)).astype(np.float32)
    initial = np.concatenate((half, half[:, :, ::-1]), axis=2).reshape(13680, 2)
    v0 = mnpt.initial_weights().astype(np.float32) / 8
    initial[:, 0] += v0
    initial[:, 1] += v0 * np.float32(0.9)
    init = root / "pst-float.npz"
    np.savez(init, middlegame=initial[:, 0], endgame=initial[:, 1], V=np.array([np.nan]),
             a=np.array([np.inf]), mask=np.array([False]))
    base = root / "base.bin"
    mnpt.write_mnpt_v3(base, mnpt.quantize(initial[:, 0]), mnpt.quantize(initial[:, 1]),
                       mnpt.initial_piece_values(), 1072.6529541015625,
                       np.zeros((13680, 2), dtype=np.int16), np.array([1, -1]), 4)
    return data, init, base, initial


def arguments(data, init, base, output, epochs=2):
    return ["train", "--data", str(data), "--init-pst", str(init), "--base", str(base),
            "--output", str(output), "--width", "4", "--epochs", str(epochs), "--batch", "32",
            "--removal-chunk", "13", "--validation-sample", "10", "--device", "cpu", "--seed", "13"]


def inputs(records, dtype=torch.float32):
    return (torch.from_numpy(feature_indices(records["board"], records["stm"], records["lion"])),
            torch.from_numpy(feature_indices(records["board"], 1 - records["stm"], records["lion"])),
            torch.tensor(phase_ratios(records["board"]), dtype=dtype))


def from_floating(weights, k):
    """公開npzの全特徴から正準側を選び、出力を再計算できるモデルを復元する。"""
    initial = np.stack((weights["middlegame"], weights["endgame"]), axis=1)
    width = weights["weights"].shape[1]
    model = NNUE(initial, weights["mask"], width, k, CPU, 13)
    with torch.no_grad():
        model.embedding.weight[:6840].copy_(torch.from_numpy(
            weights["weights"].reshape(95, 12, 12, width)[:, :, :6].reshape(6840, width)))
        model.biases.copy_(torch.from_numpy(weights["biases"][:width // 2]))
        model.output_weights.copy_(torch.from_numpy(weights["output_weights"][:, :width // 2]))
        model.output_bias.copy_(torch.from_numpy(weights["output_bias"]))
    return model


class NNUETrainingTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.threads = torch.get_num_threads()
        torch.set_num_threads(1)

    @classmethod
    def tearDownClass(cls):
        torch.set_num_threads(cls.threads)

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.data, self.init, self.base, self.initial = fixture(self.root)
        self.output = self.root / "nnue.bin"

    def run_training(self, epochs=2, extra=(), output=None):
        output = self.output if output is None else output
        with redirect_stdout(StringIO()):
            train.main(arguments(self.data, self.init, self.base, output, epochs) + list(extra))
        return json.loads(train.training_report_path(output).read_text())

    def assert_mirrors(self, model, records):
        reflected = records.copy()
        reflected["board"], reflected["lion"] = mirror(records["board"], records["lion"])
        with torch.no_grad():
            for name, expected, actual in zip(("linear", "nonlinear"),
                    model.components(*inputs(records))[:2], model.components(*inputs(reflected))[:2]):
                with self.subTest(component=name):
                    self.assertTrue(torch.equal(expected, actual))

    def test_validation_uses_training_batch_and_is_invariant_to_partition(self):
        # 要求1: エポック0と学習後の検証でbatchを使い、除去用分割に依存しない。
        original = train.validation_loss
        calls = []

        def validation(model, dataset, teacher_ks, batch, deadline):
            calls.append(batch)
            result = original(model, dataset, teacher_ks, batch, deadline)
            for chunk in (1, 3, 13):
                loss, statistics = original(model, dataset, teacher_ks, chunk, deadline)
                self.assertAlmostEqual(result[0], loss, delta=1e-9)
                self.assertEqual(statistics["samples"], result[1]["samples"])
                self.assertEqual(statistics["saturation"], result[1]["saturation"])
                self.assertAlmostEqual(statistics["nonlinear_std_cp"], result[1]["nonlinear_std_cp"], delta=1e-9)
            return result

        with patch.object(train, "validation_loss", side_effect=validation):
            self.run_training(1)
        self.assertEqual(calls, [32, 32])

    def test_initial_pst_validation_rejects_asymmetry_type_and_nonfinite_values(self):
        np.testing.assert_array_equal(load_initial(self.init), self.initial)
        for kind in ("mirror", "dtype", "shape", "nan", "range", "missing"):
            columns = {"middlegame": self.initial[:, 0].copy(), "endgame": self.initial[:, 1].copy()}
            if kind == "mirror":
                columns["endgame"][0] += 1
            elif kind == "dtype":
                columns["middlegame"] = columns["middlegame"].astype(np.float64)
            elif kind == "shape":
                columns["middlegame"] = columns["middlegame"][:-1]
            elif kind == "nan":
                columns["middlegame"][0] = np.nan
            elif kind == "range":
                columns["middlegame"][0] = 5000
            else:
                del columns["middlegame"]
            bad = self.root / f"{kind}.npz"
            np.savez(bad, **columns)
            with self.subTest(kind=kind), self.assertRaisesRegex(ValueError, str(bad)):
                load_initial(bad)

    def test_initial_logits_are_pst_only_and_seeded_rows_are_reproducible(self):
        dataset = Dataset([self.data])
        observed = feature_observations(dataset, 16) > 0
        model = NNUE(self.initial, observed, 8, 1072.6529541015625, CPU, 18)
        other = NNUE(self.initial, observed, 8, model.k, CPU, 18)
        torch.testing.assert_close(model.embedding.weight, other.embedding.weight, rtol=0, atol=0)
        deviation = float(model.embedding.weight[model.canonical_observed].detach().std())
        self.assertGreater(deviation, 0.04)
        self.assertLess(deviation, 0.06)
        records = dataset.gather(dataset.validation_indices)
        self.assert_mirrors(model, records)
        floating = model.floating_weights()
        self.assertTrue(np.all(floating["weights"][~observed] == 0))
        self.assertTrue(np.all(floating["output_weights"] == 0))
        expected = [float(direct_score(record, self.initial)) for record in records]
        with torch.no_grad():
            np.testing.assert_allclose(model(*inputs(records)).numpy() * model.k, expected, rtol=0, atol=0.001)
        np.testing.assert_allclose(float_evaluate(floating, model.k, records["board"], records["stm"], records["lion"]),
                                   expected, rtol=0, atol=1e-10)

    def test_epoch_zero_exports_requantized_pst_and_fixed_search_constants(self):
        report = self.run_training(0)
        net = read_mnue(self.output)
        self.assertEqual(net.feature_set, "P")
        self.assertEqual(struct.unpack_from("<I", self.output.read_bytes(), 4)[0], 5)
        np.testing.assert_array_equal(net.linear, mnpt.quantize(self.initial))
        _, _, values, k = mnpt.read_mnpt(self.base)
        np.testing.assert_array_equal(net.piece_values, values)
        self.assertEqual(net.k, k)
        self.assertEqual(report["status"], "candidate")
        self.assertEqual(report["total_steps"], 0)
        self.assertEqual(len(report["epochs"]), 1)
        self.assertIsNone(report["best_epoch"])
        self.assertTrue(np.isfinite(report["initial_validation_loss"]))
        self.assertEqual(report["output_sha256"], hashlib.sha256(self.output.read_bytes()).hexdigest())
        with np.load(mnpt.float_weights_path(self.output)) as floating:
            np.testing.assert_array_equal(floating["middlegame"], self.initial[:, 0])
            np.testing.assert_array_equal(floating["endgame"], self.initial[:, 1])
            self.assertEqual(floating["output_bias"].shape, ())
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.run_training(0)

    def test_training_updates_observed_rows_preserves_others_and_records_contract(self):
        report = self.run_training(2)
        net = read_mnue(self.output)
        self.assertIn(report["best_epoch"], (1, 2))
        self.assertEqual(report["stopped_by"], "max_epochs")
        self.assertEqual(len(report["gradients"]), 5)
        for row in report["gradients"]:
            self.assertTrue(np.isfinite([row["linear"], row["nonlinear"]]).all())
        for row in report["epochs"]:
            self.assertTrue(np.isfinite(row["validation_loss"]))
            for name in ("linear_movement", "nonlinear_std_cp", "saturation", "elapsed_seconds", "steps", "total_steps"):
                self.assertIn(name, row)
            self.assertTrue(all(0 <= value <= 1 for value in row["saturation"].values()))
            if row["epoch"]:
                self.assertAlmostEqual(row["total_loss"], row["train_loss"] + 1000 * row["removal_loss"], delta=1e-8)
        for name in ("inputs", "base", "init_pst", "width", "split", "teacher_classes", "teacher_ks",
                     "unobserved_features", "lr_linear", "lr_nonlinear", "weight_decay_nonlinear", "seed",
                     "batch", "max_epochs", "patience", "time_limit_hours", "packages", "quantization"):
            self.assertIn(name, report)
        self.assertEqual(report["lambda_override"], 1.0)
        self.assertEqual(report["weight_decay_linear"], 0)
        self.assertEqual(report["inputs"][0]["sha256"], hashlib.sha256(self.data.read_bytes()).hexdigest())
        self.assertEqual(len(report["inputs"][0]["provenance_sha256"]), 64)
        dataset = Dataset([self.data], lambda_override=1.0)
        self.assertEqual(report["split"]["training_indices_sha256"],
                         hashlib.sha256(dataset.training_indices.astype("<i8").tobytes()).hexdigest())
        self.assertTrue(set(dataset.gather(dataset.training_indices)["game"]).isdisjoint(
            set(dataset.gather(dataset.validation_indices)["game"])))
        with np.load(mnpt.float_weights_path(self.output)) as archive:
            floating = dict(archive)
        observed = floating["mask"]
        self.assertGreater(np.count_nonzero(~observed), 0)
        self.assertTrue(np.all(floating["weights"][~observed] == 0))
        for i, name in enumerate(("middlegame", "endgame")):
            np.testing.assert_array_equal(floating[name][~observed], self.initial[~observed, i])
            self.assertTrue(np.any(floating[name][observed] != self.initial[observed, i]))
            self.assertTrue(np.all((floating[name] >= -4096) & (floating[name] <= 4095.875)))
        for name, bound in (("weights", 8), ("biases", 8), ("output_weights", 1.98), ("output_bias", 8)):
            self.assertTrue(np.all(np.abs(floating[name]) <= bound))
        self.assertTrue(np.any(floating["output_weights"]))
        records = dataset.gather(np.arange(dataset.record_count))
        records["stm"][::2] = 1
        records["lion"][::3] = 17
        self.assert_mirrors(from_floating(floating, net.k), records)

    def test_removal_matches_actual_board_recalculation_at_phase_boundaries(self):
        model = NNUE(self.initial, np.ones(13680, dtype=bool), 4, 1072.6529541015625, CPU, 16).double()
        with torch.no_grad():
            model.output_weights.copy_(torch.tensor([[0.2, -0.3], [0.4, 0.1]], dtype=torch.float64))
            model.biases.fill_(0.07)
            model.output_bias.fill_(0.125)
        for count in (1, 2, 3, 91, 92, 93):
            for stm in (0, 1):
                with self.subTest(count=count, stm=stm), torch.no_grad():
                    records = records_with([[(i, (1, 65, 47, 111)[i % 4]) for i in range(count)]], stm=stm, lion=0)
                    own, opposite, ratios = inputs(records, torch.float64)
                    delta = model.removal_deltas(own, opposite, torch.from_numpy(piece_counts(records["board"])))
                    after = np.repeat(records, count)
                    after["board"][np.arange(count), np.arange(count)] = 0
                    expected = (model(*inputs(after, torch.float64)) - model(own, opposite, ratios)) * model.k
                    torch.testing.assert_close(delta[0, :count], expected, atol=1e-6, rtol=0)
                    self.assertEqual(delta.dtype, torch.float64)

    def test_constraints_restore_initial_unobserved_rows_after_each_update(self):
        dataset = Dataset([self.data])
        observed = feature_observations(dataset, 16) > 0
        model = NNUE(self.initial, observed, 4, 1000, CPU, 1)
        for sign in (-1, 1):
            with torch.no_grad():
                for parameter in model.parameters():
                    parameter.fill_(sign * 100000)
            model.constrain()
            floating = model.floating_weights()
            for name in ("weights", "biases", "output_bias"):
                self.assertTrue(np.all(np.abs(floating[name]) <= 8))
            self.assertTrue(np.all(np.abs(floating["output_weights"]) <= 1.98))
            self.assertTrue(np.all(floating["weights"][~observed] == 0))
            for i, name in enumerate(("middlegame", "endgame")):
                np.testing.assert_array_equal(floating[name][~observed], self.initial[~observed, i])
                self.assertTrue(np.all(floating[name][observed] == (-4096 if sign < 0 else 4095.875)))
            self.assertTrue(torch.all(model.linear.weight[-1] == 0))
            self.assertTrue(torch.all(model.embedding.weight[-1] == 0))
            train.quantized_weights(floating)

    def test_time_limit_before_first_completed_epoch_writes_error(self):
        for epochs in (0, 2):
            output = self.root / f"timeout-{epochs}.bin"
            with self.subTest(epochs=epochs), self.assertRaisesRegex(ValueError, "time limit.*0 completed"):
                self.run_training(epochs, ["--time-limit-hours", "1e-9"], output=output)
            report = json.loads(train.training_report_path(output).read_text())
            self.assertEqual(report["status"], "error")
            self.assertEqual(report["stopped_by"], "time_limit")
            self.assertIn("収束は未確認", report["convergence"])
            self.assertFalse(output.exists())
            self.assertFalse(mnpt.float_weights_path(output).exists())

    def test_time_limit_discards_partial_epoch_and_restores_completed_best(self):
        completed = self.root / "completed.bin"
        first = self.run_training(1, output=completed)
        now = [0.0]
        updates = [0]
        step = torch.optim.AdamW.step

        def timed_step(optimizer, *args, **kwargs):
            result = step(optimizer, *args, **kwargs)
            updates[0] += 1
            if updates[0] == first["steps_per_epoch"] + 1:
                now[0] = 3601.0
            return result

        with patch.object(train.time, "monotonic", side_effect=lambda: now[0]), \
                patch.object(torch.optim.AdamW, "step", timed_step):
            report = self.run_training(3, ["--time-limit-hours", "1"])
        self.assertEqual(report["stopped_by"], "time_limit")
        self.assertEqual(report["last_epoch"], 1)
        self.assertEqual(report["best_epoch"], 1)
        self.assertEqual(report["total_steps"], first["steps_per_epoch"] + 1)
        self.assertEqual(self.output.read_bytes(), completed.read_bytes())

    def test_time_limit_during_validation_does_not_select_incomplete_epoch(self):
        now = [0.0]
        original = train.validation_loss
        calls = [0]

        def validation(*args):
            calls[0] += 1
            if calls[0] == 2:
                now[0] = 3601
            return original(*args)

        with patch.object(train.time, "monotonic", side_effect=lambda: now[0]), \
                patch.object(train, "validation_loss", side_effect=validation), \
                self.assertRaisesRegex(ValueError, "0 completed"):
            self.run_training(2, ["--time-limit-hours", "1"])
        report = json.loads(train.training_report_path(self.output).read_text())
        self.assertEqual(report["last_epoch"], 0)
        self.assertIsNone(report["best_epoch"])
        self.assertEqual(report["status"], "error")

    def test_early_stopping_ignores_epoch_zero_and_restores_best_weights(self):
        losses = iter([0.01, 0.6, 0.5, 0.5, 0.55])
        snapshots = []
        original = train.validation_loss

        def validation(model, *args):
            _, statistics = original(model, *args)
            snapshots.append(model.floating_weights())
            return next(losses), statistics

        with patch.object(train, "validation_loss", side_effect=validation):
            report = self.run_training(10, ["--patience", "2"])
        self.assertEqual(report["best_epoch"], 2)
        self.assertEqual(report["last_epoch"], 4)
        self.assertEqual(report["stopped_by"], "early_stopping")
        self.assertFalse(report["improved_over_epoch_zero"])
        with np.load(mnpt.float_weights_path(self.output)) as actual:
            for name, expected in snapshots[2].items():
                np.testing.assert_array_equal(actual[name], expected)

    def test_lookahead_configuration_and_teacher_class_are_recorded(self):
        report = self.run_training(1, ["--lookahead-gamma", "0.9", "--lookahead-plies", "40"])
        self.assertEqual(report["lookahead"], {"gamma": 0.9, "plies": 40})
        self.assertEqual(len(report["teacher_ks"]), len(report["teacher_classes"]))
        self.assertEqual(report["teacher_training_counts"], [report["split"]["training_positions"]])
        self.assertEqual(report["teacher_classes"][0]["lookahead_gamma"], 0.9)

    def test_nonfinite_loss_or_gradient_stops_and_records_error(self):
        original_init = NNUE.__init__
        original_forward = NNUE.forward

        def bad_gradient(model, *args, **kwargs):
            original_init(model, *args, **kwargs)
            model.output_bias.register_hook(lambda gradient: gradient * float("nan"))

        for kind in ("loss", "gradient"):
            output = self.root / f"{kind}.bin"
            target = (patch.object(NNUE, "forward", lambda model, *args: original_forward(model, *args) * float("nan"))
                      if kind == "loss" else patch.object(NNUE, "__init__", bad_gradient))
            with self.subTest(kind=kind), target, self.assertRaisesRegex(ValueError, "non-finite"):
                self.run_training(1, output=output)
            report = json.loads(train.training_report_path(output).read_text())
            self.assertEqual(report["status"], "error")
            self.assertEqual(report["total_steps"], 0)
            self.assertFalse(output.exists())

    def test_quantization_failure_keeps_artifacts_and_returns_nonzero(self):
        original_init = NNUE.__init__

        def saturated(model, *args, **kwargs):
            original_init(model, *args, **kwargs)
            with torch.no_grad():
                model.embedding.weight.zero_()
                model.biases.fill_(1)
                model.output_weights.fill_(1.98)

        with patch.object(NNUE, "__init__", saturated), self.assertRaises(SystemExit) as error:
            self.run_training(0)
        self.assertEqual(error.exception.code, 1)
        report = json.loads(train.training_report_path(self.output).read_text())
        self.assertEqual(report["status"], "quantization_error")
        self.assertGreater(report["quantization"]["mean_absolute_error_cp"], report["k"] / 40)
        read_mnue(self.output)
        self.assertTrue(mnpt.float_weights_path(self.output).exists())

    def test_any_existing_output_is_refused_without_overwriting(self):
        for path in (self.output, train.training_report_path(self.output), mnpt.float_weights_path(self.output)):
            with self.subTest(path=path):
                path.write_bytes(b"existing")
                with self.assertRaisesRegex(ValueError, str(path)):
                    self.run_training(0)
                self.assertEqual(path.read_bytes(), b"existing")
                path.unlink()

    def test_quantization_uses_ties_to_even_and_rejects_overflow(self):
        model = NNUE(self.initial, np.ones(13680, dtype=bool), 2, 1000, CPU, 1)
        floating = model.floating_weights()
        floating["middlegame"][:4] = [0.0625, 0.1875, -0.0625, -0.1875]
        floating["weights"][0] = [0.5, 1.5]
        floating["biases"][:] = [-0.5, -1.5]
        floating["output_weights"][:] = np.array([[0.5, 1.5], [-0.5, -1.5]]) / 4096
        floating["output_bias"] = np.float32(1 / 8192)
        quantized = train.quantized_weights(floating)
        np.testing.assert_array_equal(quantized[0][:4, 0], [0, 2, 0, -2])
        np.testing.assert_array_equal(quantized[1][0], [64, 190])
        np.testing.assert_array_equal(quantized[2], [-64, -190])
        np.testing.assert_array_equal(quantized[3], [[0, 2], [0, -2]])
        self.assertEqual(quantized[4], 64)
        for name, value in (("weights", 9), ("biases", -9), ("output_weights", 2.1), ("output_bias", 9)):
            invalid = {key: np.array(item, copy=True) for key, item in floating.items()}
            invalid[name].fill(value)
            with self.subTest(name=name), self.assertRaises(ValueError):
                train.quantized_weights(invalid)


if __name__ == "__main__":
    unittest.main()
