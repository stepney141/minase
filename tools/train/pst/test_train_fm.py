"""FM設計書の恒等式、初期勾配、整数化、保存形式、および学習の契約を検証する。"""

from contextlib import redirect_stdout
from io import StringIO
from itertools import combinations
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

from features import FEATURE_COUNT, PADDING_INDEX, BOARD_FEATURE_COUNT, feature_indices, mirror
from mnsd import Dataset, RECORD_DTYPE, write_mnsd, map_records
from taper import phase_numerators
from test_train_pst import reference_hash64
from train_pst import initial_piece_values, integer_evaluate as pst_evaluate, write_mnpt, float_weights_path
import train_fm as fm


def active(indices: list[int]) -> np.ndarray:
    """指定した二値特徴だけが発火する、padding付き1局面を作る。"""
    result = np.full((1, 145), PADDING_INDEX, dtype=np.int32)
    result[0, :len(indices)] = indices
    return result


def fixture(root: Path, *, epoch_zero: bool = False, one_piece: bool = False) -> tuple[Path, Path]:
    """訓練と検証を同じ2駒配置とし、結果だけで学習可能な標本を作る。"""
    records = np.zeros(80, dtype=RECORD_DTYPE)
    records["board"][:, 0] = 1
    if not one_piece:
        records["board"][:, 1] = 1
    records["lion"] = 255
    records["game"] = np.arange(80)
    records["result"] = 2
    if epoch_zero:
        for game in range(80):
            if reference_hash64(0, game) % 20 == 0:
                records["result"][game] = 1
    data = root / "data.bin"
    write_mnsd(data, records, seed=0, network_checksum=b"a" * 32)
    base = root / "base.bin"
    weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
    write_mnpt(base, weights, weights, initial_piece_values(), 1000.125)
    return data, base


def train_arguments(data: Path, base: Path, output: Path) -> list[str]:
    return ["train", "--data", str(data), "--init", str(base), "--output", str(output),
            "--k", "1000.125", "--teacher-ks", "200", "--rank", "2", "--lr", "0.01",
            "--weight-decay", "0.001", "--lambda-res", "0.001", "--epochs", "2",
            "--patience", "1", "--batch", "8", "--seed", "1", "--lambda", "0",
            "--validation-sample", "10", "--device", "cpu"]


class AlgebraTest(unittest.TestCase):
    def test_pairs_identity_and_incremental_add_remove_agree(self) -> None:
        # 二値入力の組の和を、恒等式と独立に直接列挙する。
        v = np.zeros((FEATURE_COUNT, 3))
        v[:4] = [[1, -2, 3], [-4, 5, 2], [3, 1, -2], [2, -1, 4]]
        signs = np.array([1, -1, 1], dtype=np.int8)
        expected = sum(sum(int(signs[f]) * int(v[i, f]) * int(v[j, f]) for f in range(3))
                       for i, j in combinations(range(4), 2))
        self.assertEqual(fm.fm_phi(v, signs, active([0, 1, 2, 3]))[0], expected)
        a, c = fm.fm_accumulators(v.astype(np.int16), signs, active([0, 1]))
        for index in (2, 3):
            a += v[index].astype(np.int64)
            c += sum(int(signs[f]) * int(v[index, f])**2 for f in range(3))
        full_a, full_c = fm.fm_accumulators(v.astype(np.int16), signs, active([0, 1, 2, 3]))
        np.testing.assert_array_equal(a, full_a)
        np.testing.assert_array_equal(c, full_c)
        self.assertEqual(fm.fm_correction(a, c, signs, 0)[0], expected)
        a -= v[1].astype(np.int64)
        c -= sum(int(signs[f]) * int(v[1, f])**2 for f in range(3))
        removed_a, removed_c = fm.fm_accumulators(v.astype(np.int16), signs, active([0, 2, 3]))
        np.testing.assert_array_equal(a, removed_a)
        np.testing.assert_array_equal(c, removed_c)

    def test_zero_coefficients_exactly_match_integer_pst_after_mirroring(self) -> None:
        boards = np.zeros((4, 144), dtype=np.uint8)
        for row, count in enumerate((2, 3, 47, 92)):
            boards[row, :count] = 1
        stm = np.array([0, 1, 0, 1], dtype=np.uint8)
        lions = np.array([255, 4, 17, 255], dtype=np.uint8)
        mg = (np.arange(FEATURE_COUNT) % 32767).astype(np.int16)
        eg = -mg
        original = feature_indices(boards, stm, lions)
        mirrored, lions = mirror(boards, lions)
        selected = feature_indices(mirrored, stm, lions)
        q = phase_numerators(boards)
        baseline = pst_evaluate(mg, eg, selected, q)
        self.assertTrue(np.any(baseline != pst_evaluate(mg, eg, original, q)))
        torch.manual_seed(1)
        model = fm.FM(2, np.ones(FEATURE_COUNT, dtype=bool), torch.device("cpu"))
        v = model.embedding.weight[:FEATURE_COUNT].detach().numpy()
        a = model.a.detach().numpy()
        np.testing.assert_array_equal(fm.float_evaluate(mg, eg, 1000, v, a, selected, q), baseline)
        np.testing.assert_array_equal(fm.integer_evaluate(mg, eg, np.zeros_like(v, dtype=np.int16),
                                                        np.ones(2, dtype=np.int8), 0, selected, q), baseline)
        logits, residual = fm.batch_logits(model, mg, eg, 1000, selected, q, torch.device("cpu"))
        np.testing.assert_array_equal(residual.detach().numpy(), 0)
        np.testing.assert_array_equal(logits.detach().numpy(), baseline.astype(np.float32) / 1000)

    def test_initial_loss_gradient_is_only_in_output_coefficients(self) -> None:
        mask = np.zeros(FEATURE_COUNT, dtype=bool)
        mask[:2] = True
        model = fm.FM(1, mask, torch.device("cpu"))
        with torch.no_grad():
            model.embedding.weight[:2, 0] = torch.tensor([2., 3.])
        loss = torch.nn.functional.binary_cross_entropy_with_logits(model(torch.tensor(active([0, 1]))), torch.ones(1))
        loss.backward()
        # Phi=6a、dBCE/dPhi=-1/2 より dBCE/da=-3。a=0なので dBCE/dV=0。
        self.assertEqual(model.a.grad.item(), -3)
        self.assertEqual(model.embedding.weight.grad.norm().item(), 0)
        optimizer = torch.optim.AdamW(model.parameters(), lr=0.01, weight_decay=0.1)
        optimizer.step()
        optimizer.zero_grad()
        model(torch.tensor(active([0, 1]))).sum().backward()
        self.assertGreater(model.embedding.weight.grad.norm().item(), 0)
        model.fix_unobserved()
        np.testing.assert_array_equal(model.embedding.weight.detach().numpy()[2:], 0)

    def test_negative_division_and_extreme_int64_bounds(self) -> None:
        u = np.zeros((FEATURE_COUNT, 1), dtype=np.int16)
        u[:2, 0] = [1, -1]
        a, c = fm.fm_accumulators(u, np.array([1]), active([0, 1]))
        self.assertEqual(fm.fm_correction(a, c, np.array([1]), 1)[0], 0)
        for sign in (1, -1):
            u = np.full((FEATURE_COUNT, 64), -32768, dtype=np.int16)
            d = np.full(64, sign, dtype=np.int8)
            a, c = fm.fm_accumulators(u, d, active(list(range(145))))
            expected = sign * 64 * (145 * 144 // 2) * 32768**2
            correction = fm.fm_correction(a, c, d, 0)
            self.assertEqual(correction.dtype, np.int64)
            self.assertEqual(int(correction[0]), expected)
            self.assertLess(abs(expected), 2**63)
            zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
            self.assertEqual(fm.integer_evaluate(zeros, zeros, u, d, 0, active(list(range(145))), np.array([90]))[0], sign * 28999)

    def test_quantization_selects_largest_exponent_and_absorbs_signed_scale(self) -> None:
        v = np.zeros((FEATURE_COUNT, 3))
        v[0] = [1, 1, 10]
        u, d, exponent = fm.quantize(v, np.array([4, -1, 0]), 4)
        self.assertEqual(exponent, 11)  # 最大値4。2^12倍では16,384なので不合格。
        np.testing.assert_array_equal(u[0], [8192, 4096, 0])
        np.testing.assert_array_equal(d, [1, -1, 1])
        v[0, 0] = 1e-10
        v[0, 1:] = 0
        self.assertEqual(fm.quantize(v, np.ones(3), 1)[2], 30)
        v[0, 0] = 16383.49
        self.assertEqual(fm.quantize(v, np.ones(3), 1)[2], 0)
        for value in (0, 16383.51, float("nan"), float("inf")):
            v[:] = 0
            v[0, 0] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                fm.quantize(v, np.ones(3), 1)


class FormatTest(unittest.TestCase):
    def test_round_trip_layout_and_corruptions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fm.bin"
            mg = np.arange(FEATURE_COUNT, dtype=np.int16)
            u = np.tile(np.array([12, -13], dtype=np.int16), (FEATURE_COUNT, 1))
            values = initial_piece_values()
            fm.write_mnpt_v3(path, mg, -mg, values, 1000.125, u, np.array([1, -1]), 7)
            decoded = fm.read_mnpt_v3(path)
            for expected, actual in zip((mg, -mg, values, 1000.125, u, [1, -1], 7), decoded):
                np.testing.assert_array_equal(actual, expected)
            original = path.read_bytes()
            offset = 80 + 13680 * 4 + 47 * 4
            self.assertEqual(len(original), offset + 8 + 2 + 13680 * 2 * 2)
            self.assertEqual(struct.unpack_from("<II", original, offset), (2, 7))
            self.assertEqual(original[offset + 8:offset + 10], b'\x01\xff')
            self.assertEqual(struct.unpack_from("<hh", original, offset + 10), (12, -13))
            cases = [("magic", 0, b"NOPE"), ("version", 4, struct.pack("<I", 2)),
                     ("features", 8, struct.pack("<I", 42)), ("rules", 16, b"X"),
                     ("K zero", 12, struct.pack("<f", 0)), ("K nan", 12, struct.pack("<f", float("nan"))),
                     ("K inf", 12, struct.pack("<f", float("inf"))), ("K negative", 12, struct.pack("<f", -1)),
                     ("rank zero", offset, struct.pack("<I", 0)), ("rank length", offset, struct.pack("<I", 3)),
                     ("exponent", offset + 4, struct.pack("<I", 31)), ("sign zero", offset + 8, b"\0"),
                     ("sign two", offset + 8, b"\x02"),
                     ("pawn", 80 + 13680 * 4 + 29 * 4, struct.pack("<i", 0)),
                     ("royal", 80 + 13680 * 4 + 11 * 4, struct.pack("<i", 1)),
                     ("piece range", 80 + 13680 * 4 + 4 * 4, struct.pack("<i", 29000))]
            for name, start, value in cases:
                changed = bytearray(original)
                changed[start:start + len(value)] = value
                changed[48:80] = hashlib.sha256(changed[80:]).digest()
                path.write_bytes(changed)
                with self.subTest(name=name), self.assertRaises(ValueError):
                    fm.read_mnpt_v3(path)
            for changed in (original[:79], original[:-1], original + b"x", original[:48] + b"x" * 32 + original[80:]):
                path.write_bytes(changed)
                with self.assertRaises(ValueError):
                    fm.read_mnpt_v3(path)

    def test_writer_rejects_values_before_narrowing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fm.bin"
            zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
            u = np.zeros((FEATURE_COUNT, 1), dtype=np.int32)
            for bad in (-32769, 32768):
                u[0, 0] = bad
                with self.assertRaises(ValueError):
                    fm.write_mnpt_v3(path, zeros, zeros, initial_piece_values(), 1000, u, np.ones(1, dtype=np.int8), 0)
            self.assertFalse(path.exists())


class TrainingTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.threads = torch.get_num_threads()
        torch.set_num_threads(1)

    @classmethod
    def tearDownClass(cls) -> None:
        torch.set_num_threads(cls.threads)

    def test_seed_reproduces_training_and_mask_includes_all_mirrors(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, base = fixture(root)
            dataset = Dataset([data])
            counts = fm.feature_observations(dataset, 8)
            expected = np.zeros(FEATURE_COUNT, dtype=np.int64)
            # 成れる先手歩兵29の0,1升と、左右鏡映の10,11升。
            expected[29 * 144 + np.array([0, 1, 10, 11])] = dataset.training_indices.size
            np.testing.assert_array_equal(counts, expected)
            first, second = root / "first.bin", root / "second.bin"
            for output in (first, second):
                with redirect_stdout(StringIO()):
                    fm.main(train_arguments(data, base, output))
                fm.validate_fixed_base(base, output)
                report = json.loads(fm.training_report_path(output).read_text())
                self.assertGreater(report["best_epoch"], 0)
                self.assertEqual(report["steps_per_epoch"], (dataset.training_indices.size + 7) // 8)
                self.assertEqual(report["total_steps"], report["steps_per_epoch"] * 2)
                self.assertEqual(report["gradients"][0]["embedding"], 0)
                self.assertGreater(report["gradients"][0]["a"], 0)
                self.assertGreater(report["gradients"][1]["embedding"], 0)
                self.assertEqual(report["feature_observations"], expected.tolist())
                with np.load(float_weights_path(output)) as saved:
                    np.testing.assert_array_equal(saved["mask"], expected > 0)
                    np.testing.assert_array_equal(saved["V"][expected == 0], 0)
                self.assertGreater(report["phi"]["max_absolute"], 0)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            self.assertEqual(float_weights_path(first).read_bytes(), float_weights_path(second).read_bytes())
            self.assertEqual(fm.training_report_path(first).read_bytes(), fm.training_report_path(second).read_bytes())

    def test_lion_observations_include_mirror_and_exclude_validation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, _ = fixture(root)
            records = np.array(map_records(data))
            records["lion"] = 2
            for game in range(80):
                if reference_hash64(0, game) % 20 == 0:
                    records["lion"][game] = 30
            write_mnsd(data, records, seed=0, network_checksum=b"a" * 32)
            dataset = Dataset([data])
            counts = fm.feature_observations(dataset, 8)
            expected = np.zeros(144, dtype=np.int64)
            expected[[2, 9]] = dataset.training_indices.size
            np.testing.assert_array_equal(counts[BOARD_FEATURE_COUNT:], expected)

    def test_training_uses_reflected_features_for_baseline_and_residual(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, base = fixture(root)
            weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
            # 通常局面は0cp、鏡映局面は3cp。片方だけ鏡映すると一致しない。
            weights[29 * 144 + 10] = 24
            write_mnpt(base, weights, weights, initial_piece_values(), 1000.125)
            original = fm.batch_logits
            batches = []
            def checked(model, mg, eg, k, features, q, device):
                logits, residual = original(model, mg, eg, k, features, q, device)
                if torch.is_grad_enabled():
                    for row in features:
                        self.assertEqual(set(row[row != PADDING_INDEX]), {29 * 144 + 10, 29 * 144 + 11})
                    torch.testing.assert_close(logits - residual, torch.full_like(logits, 3 / 1000.125))
                    batches.append(len(features))
                return logits, residual
            with patch.object(fm, "batch_logits", side_effect=checked), patch.object(
                    torch, "rand", side_effect=lambda count, **kwargs: torch.zeros(count, device=kwargs["device"])), redirect_stdout(StringIO()):
                fm.main(train_arguments(data, base, root / "fm.bin"))
            self.assertTrue(batches)

    def test_validation_is_bce_without_residual_penalty(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, _ = fixture(root)
            dataset = Dataset([data])
            mask = fm.feature_observations(dataset, 8) > 0
            model = fm.FM(1, mask, torch.device("cpu"))
            with torch.no_grad():
                model.embedding.weight[29 * 144, 0] = 2
                model.embedding.weight[29 * 144 + 1, 0] = 3
                model.a[0] = 0.5
            zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
            loss, distribution = fm.validation_loss(model, dataset, zeros, zeros, 1000,
                                                     np.array([200]), 0, 8, torch.device("cpu"))
            # Phi=0.5×2×3=3、教師は1。BCE=log(1+exp(-3))。
            self.assertAlmostEqual(loss, float(np.logaddexp(0, -3)), places=6)
            self.assertEqual(distribution, {"mean": 3, "std": 0, "max_absolute": 3,
                                             "samples": int(dataset.validation_indices.size)})

    def test_tied_validation_keeps_epoch_zero(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, base = fixture(root)
            output = root / "fm.bin"
            with patch.object(fm, "validation_loss", return_value=(0.5, {"mean": 0, "std": 0, "max_absolute": 0})), redirect_stdout(StringIO()):
                fm.main(train_arguments(data, base, output))
            report = json.loads(fm.training_report_path(output).read_text())
            self.assertEqual(report["best_epoch"], 0)
            self.assertEqual(report["status"], "excluded_epoch_zero")
            self.assertFalse(output.exists())

    def test_epoch_zero_is_saved_but_never_quantized_and_patience_stops(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, base = fixture(root, epoch_zero=True)
            output = root / "fm.bin"
            with patch.object(fm, "quantize", side_effect=AssertionError("epoch zero must not be quantized")), redirect_stdout(StringIO()):
                fm.main(train_arguments(data, base, output))
            report = json.loads(fm.training_report_path(output).read_text())
            self.assertEqual(report["best_epoch"], 0)
            self.assertEqual(report["status"], "excluded_epoch_zero")
            self.assertEqual(len(report["epochs"]), 2)
            self.assertFalse(output.exists())
            with np.load(float_weights_path(output)) as stored:
                np.testing.assert_array_equal(stored["a"], 0)

    def test_zero_gradient_stops_with_json_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, base = fixture(root, one_piece=True)
            output = root / "fm.bin"
            with redirect_stdout(StringIO()), self.assertRaisesRegex(ValueError, "gradient stayed zero"):
                fm.main(train_arguments(data, base, output))
            report = json.loads(fm.training_report_path(output).read_text())
            self.assertEqual(report["status"], "error")
            self.assertEqual(len(report["gradients"]), 5)
            self.assertFalse(output.exists())

    def test_cuda_and_fixed_scale_are_checked_explicitly(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, base = fixture(root)
            args = train_arguments(data, base, root / "fm.bin")
            args[-1] = "cuda"
            with patch.object(torch.cuda, "is_available", return_value=False), self.assertRaisesRegex(ValueError, "CUDA unavailable"):
                fm.main(args)
            args[-1] = "cpu"
            args[args.index("--k") + 1] = "1000"
            with self.assertRaisesRegex(ValueError, "baseline K"):
                fm.main(args)


if __name__ == "__main__":
    unittest.main()
