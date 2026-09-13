"""承認された2項の駒除去損失を、合成盤面の直接評価から検証する。"""
from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
from fractions import Fraction
from io import StringIO
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

from features import FEATURE_COUNT, PADDING_INDEX, feature_indices, mirror
from mnsd import Dataset, RECORD_DTYPE
from test_train_pst import PIECE_VALUES, write_mnsd
import train_pst as train


PROMOTABLE = (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16, 17, 18, 19)
CPU = torch.device("cpu")


def decoded(byte: int, square: int, stm: int) -> tuple[int, int, int]:
    """MNSDの公開byte形式から状態・相対陣営・特徴番号を直接求める。"""
    color = int(byte >= 65)
    payload = byte - 1 - 64 * color
    kind = payload % 29
    state = 29 + PROMOTABLE.index(kind) if payload < 29 and kind in PROMOTABLE else kind
    relative = int(color != stm)
    mapped = square if stm == 0 else (11 - square // 12) * 12 + square % 12
    return state, relative, (relative * 47 + state) * 144 + mapped


def direct_score(record: np.void, weights_cp: np.ndarray) -> Fraction:
    """両端点の和を盤面から毎回作り、未丸め評価を厳密に求める。"""
    features = [decoded(int(byte), square, int(record["stm"]))[2]
                for square, byte in enumerate(record["board"]) if byte]
    if record["lion"] != 255:
        square = int(record["lion"])
        mapped = square if record["stm"] == 0 else (11 - square // 12) * 12 + square % 12
        features.append(13536 + mapped)
    mg = sum((Fraction(float(weights_cp[i, 0])) for i in features), Fraction())
    eg = sum((Fraction(float(weights_cp[i, 1])) for i in features), Fraction())
    q = min(90, max(0, int(np.count_nonzero(record["board"])) - 2))
    return (q * mg + (90 - q) * eg) / 90


def integer_score(value: Fraction) -> int:
    return min(28999, max(-28999, int(value)))


def direct_loss(records: np.ndarray, base_i16: np.ndarray, candidate_cp: np.ndarray, k: int) -> Fraction:
    """式の2項と等局面平均を、1枚ずつ実際に消した盤面から計算する。"""
    base_cp = base_i16.astype(np.float64) / 8
    per_position = []
    for record in records:
        before_base = direct_score(record, base_cp)
        before_candidate = direct_score(record, candidate_cp)
        losses = []
        for square, byte in enumerate(record["board"]):
            if not byte:
                continue
            state, relative, _ = decoded(int(byte), square, int(record["stm"]))
            if state in (11, 21):
                continue
            after = record.copy()
            after["board"][square] = 0
            after_base = direct_score(after, base_cp)
            integer_delta = integer_score(after_base) - integer_score(before_base)
            if integer_delta == 0:
                continue
            sign = 1 if integer_delta > 0 else -1
            base_delta = after_base - before_base
            candidate_delta = direct_score(after, candidate_cp) - before_candidate
            margin = min(abs(base_delta), Fraction(1, 4))
            lower = max(Fraction(), margin - sign * candidate_delta)
            bad_base = (relative == 0 and integer_delta > 0) or (relative == 1 and integer_delta < 0)
            upper = max(Fraction(), sign * candidate_delta - abs(base_delta)) if bad_base else Fraction()
            losses.append((lower ** 2 + upper ** 2) / k ** 2)
        per_position.append(sum(losses, Fraction()) / len(losses) if losses else Fraction())
    return sum(per_position, Fraction()) / len(per_position)


def records_with(pieces: list[list[tuple[int, int]]], *, stm: int = 0, lion: int = 255) -> np.ndarray:
    records = np.zeros(len(pieces), dtype=RECORD_DTYPE)
    records["stm"] = stm
    records["lion"] = lion
    for record, positions in zip(records, pieces):
        for square, byte in positions:
            record["board"][square] = byte
    return records


def constant_base(own: int = 8, enemy: int = -8) -> np.ndarray:
    weights = np.zeros((13680, 2), dtype=np.int16)
    for relative, value in [(0, own), (1, enemy)]:
        for state in range(47):
            if state not in (11, 21):
                start = (relative * 47 + state) * 144
                weights[start:start + 144] = value
    return weights


def model_and_loss(records, base, candidate, k=2):
    model = train.make_model(torch.tensor(candidate, dtype=torch.float64), CPU, "mirrored").double()
    # Explicit assignment avoids depending on initialization's floating dtype policy.
    canonical = candidate.reshape(95, 12, 12, 2)[:, :, :6].reshape(6840, 2)
    with torch.no_grad():
        model.weight[:6840].copy_(torch.from_numpy(canonical))
    indices = torch.from_numpy(feature_indices(records["board"], records["stm"], records["lion"]))
    counts = torch.tensor(np.count_nonzero(records["board"], axis=1), device=CPU)
    reference = train.make_removal_reference(base[:, 0], base[:, 1], CPU)
    loss = train.removal_loss(model(indices), indices, reference, counts, k)
    return model, loss


class RemovalFormulaTest(unittest.TestCase):
    """測定文書の2項式を、独立した有理数の盤面評価と照合する。"""

    def assert_oracle(self, records, base, candidate, k=2):
        model, loss = model_and_loss(records, base, candidate, k)
        expected = direct_loss(records, base, candidate, k)
        self.assertAlmostEqual(float(loss.detach()), float(expected), delta=1e-10)
        return model, loss

    def test_correct_baseline_only_requires_a_quarter_cp_margin_on_both_sides(self):
        base = constant_base()
        for byte in (1, 65):
            records = records_with([[(0, byte), (142, 12), (143, 76)]])
            for candidate_factor, expected in [(-1, Fraction(25, 64)), (Fraction(1, 8), Fraction(1, 256)),
                                               (Fraction(1, 4), Fraction()), (1, Fraction()), (2, Fraction())]:
                with self.subTest(byte=byte, factor=candidate_factor):
                    candidate = base.astype(np.float64) / 8 * float(candidate_factor)
                    _, loss = self.assert_oracle(records, base, candidate)
                    self.assertAlmostEqual(float(loss.detach()), float(expected), delta=1e-12)

    def test_physically_wrong_baseline_has_lower_and_upper_penalties_on_both_sides(self):
        base = constant_base(-8, 8)
        for byte in (1, 65):
            records = records_with([[(0, byte), (142, 12), (143, 76)]])
            for factor, expected in [(-1, Fraction(25, 64)), (Fraction(1, 2), Fraction()),
                                     (1, Fraction()), (2, Fraction(1, 4))]:
                with self.subTest(byte=byte, factor=factor):
                    candidate = base.astype(np.float64) / 8 * float(factor)
                    _, loss = self.assert_oracle(records, base, candidate)
                    self.assertAlmostEqual(float(loss.detach()), float(expected), delta=1e-12)

    def test_integer_zero_uses_truncation_of_each_score_and_is_excluded(self):
        # All integer differences are 0; .125 -> -.125 also detects floor in place of truncation.
        for pawn, royal in [(1, 0), (-1, -6), (2, -1)]:
            base = constant_base(pawn, -pawn)
            base[11 * 144:12 * 144] = royal
            records = records_with([[(0, 1), (142, 12)]])
            _, loss = self.assert_oracle(records, base, -base.astype(np.float64) * 100)
            self.assertEqual(float(loss.detach()), 0)

    def test_margin_is_capped_by_the_baseline_unrounded_difference(self):
        # Before=1, after=.875: integer sign is negative but raw magnitude is only .125.
        base = constant_base(1, -1)
        base[11 * 144:12 * 144] = 7
        candidate = base.astype(np.float64) / 8
        candidate[29 * 144:30 * 144] = 0
        records = records_with([[(0, 1), (142, 12)]])
        _, loss = self.assert_oracle(records, base, candidate)
        self.assertAlmostEqual(float(loss.detach()), float(Fraction(1, 256)), delta=1e-12)

    def test_baseline_integer_clipping_excludes_equal_clipped_scores(self):
        base = np.full((13680, 2), 32000, dtype=np.int16)
        records = records_with([[(i, 1) for i in range(10)]])
        _, loss = self.assert_oracle(records, base, -base.astype(np.float64))
        self.assertEqual(float(loss.detach()), 0)

    def test_equal_positions_then_equal_eligible_removals_and_no_target_position(self):
        base = constant_base()
        candidate = base.astype(np.float64) / 8
        candidate[29 * 144:30 * 144] = -1
        # First: 1 wrong pawn. Second: 1 wrong pawn + 1 correct go-between.
        # Third has only kings. Loss=(25/64 + 25/128 + 0)/3=25/128.
        records = records_with([[(0, 1), (142, 12), (143, 76)],
                                [(0, 1), (2, 2), (142, 12), (143, 76)], [(142, 12), (143, 76)]])
        _, loss = self.assert_oracle(records, base, candidate)
        self.assertAlmostEqual(float(loss.detach()), float(Fraction(25, 128)), delta=1e-12)

    def test_integer_zero_removal_is_not_counted_in_a_positions_denominator(self):
        base = constant_base()
        base[30 * 144:31 * 144] = 1
        candidate = base.astype(np.float64) / 8
        candidate[29 * 144:30 * 144] = -1
        records = records_with([[(0, 1), (2, 2), (142, 12), (143, 76)]])
        _, loss = self.assert_oracle(records, base, candidate)
        self.assertAlmostEqual(float(loss.detach()), float(Fraction(25, 64)), delta=1e-12)

    def test_royals_are_excluded_and_shared_canonical_occurrences_remove_only_one(self):
        base = constant_base()
        candidate = -base.astype(np.float64) / 8
        # Squares 0 and 11 share one parameter; removing one pawn must retain the other.
        records = records_with([[(0, 1), (11, 1), (130, 51), (142, 12), (143, 76)]])
        _, loss = self.assert_oracle(records, base, candidate)
        self.assertAlmostEqual(float(loss.detach()), float(Fraction(25, 64)), delta=1e-12)

    def test_each_reachable_nonroyal_state_is_eligible_but_both_royals_are_excluded(self):
        base = constant_base()
        candidate = -base.astype(np.float64) / 8
        # The states are specified by the persisted piece encoding, not the loss predicate.
        states = list(range(29, 47)) + [4, 5, 6, 7, 8, 9, 10, 12, 17, 20, 22, 23, 24, 25, 26, 27, 28]
        for state in states:
            byte = 1 + PROMOTABLE[state - 29] if state >= 29 else 30 + state
            with self.subTest(state=state):
                record = records_with([[(0, byte), (142, 12), (143, 76)]])
                _, loss = self.assert_oracle(record, base, candidate)
                self.assertAlmostEqual(float(loss.detach()), float(Fraction(25, 64)), delta=1e-12)
        royal_base = np.full((13680, 2), 8, dtype=np.int16)
        royal_record = records_with([[(0, 12), (11, 51)]])
        _, loss = self.assert_oracle(royal_record, royal_base, -royal_base.astype(np.float64) / 8)
        self.assertEqual(float(loss.detach()), 0)

    def test_direct_board_removal_matches_phase_boundaries_both_perspectives_and_lion(self):
        rng = np.random.default_rng(103)
        raw = rng.integers(-500, 500, size=(95, 12, 6, 2), dtype=np.int16)
        base = np.concatenate((raw, raw[:, :, ::-1]), axis=2).reshape(13680, 2)
        candidate = -base.astype(np.float64) / 8
        for count in (1, 2, 3, 91, 92, 93):
            for stm in (0, 1):
                with self.subTest(count=count, stm=stm):
                    # Synthetic boards isolate clamp boundaries; no legality claim is made.
                    pieces = [(i, (1, 65, 47, 111)[i % 4]) for i in range(count)]
                    records = records_with([pieces], stm=stm, lion=0)
                    records["kirin"] = 1
                    self.assert_oracle(records, base, candidate)
                    _, self_loss = self.assert_oracle(records, base, base.astype(np.float64) / 8)
                    self.assertEqual(float(self_loss.detach()), 0)

    def test_lion_feature_survives_removing_its_target_and_phase_change_is_not_dropped(self):
        base = constant_base()
        candidate = base.astype(np.float64) / 8
        # Own pawn is +1 at both endpoints; remaining lion contributes MG=-180, EG=180.
        # N=3 -> 2 changes q=1 -> 0, so delta = +3 rather than -1.
        candidate[13536:] = [-180, 180]
        records = records_with([[(0, 1), (142, 12), (143, 76)]], lion=0)
        _, loss = self.assert_oracle(records, base, candidate)
        self.assertAlmostEqual(float(loss.detach()), float(Fraction(169, 64)), delta=1e-12)

    def test_mirror_preserves_loss_and_gradients(self):
        base = constant_base()
        candidate = -base.astype(np.float64) / 8
        candidate[13536:] = [-180, 180]
        records = records_with([[(0, 1), (17, 65), (41, 47), (142, 12), (143, 76)]], lion=17)
        reflected = records.copy()
        reflected["board"], reflected["lion"] = mirror(records["board"], records["lion"])
        left, a = self.assert_oracle(records, base, candidate)
        right, b = self.assert_oracle(reflected, base, candidate)
        a.backward()
        b.backward()
        self.assertEqual(float(a.detach()), float(b.detach()))
        torch.testing.assert_close(left.weight.grad, right.weight.grad, rtol=0, atol=1e-12)

    def test_loss_gradient_corrects_reversal_and_same_sign_worsening(self):
        for baseline_wrong, factor in [(False, -1), (True, 2), (True, -1)]:
            base = constant_base(-8, 8) if baseline_wrong else constant_base()
            for byte in (1, 65):
                with self.subTest(baseline_wrong=baseline_wrong, factor=factor, byte=byte):
                    records = records_with([[(0, byte), (142, 12), (143, 76)]])
                    candidate = base.astype(np.float64) / 8 * factor
                    model, loss = model_and_loss(records, base, candidate)
                    optimizer = torch.optim.SGD(model.parameters(), lr=.1)
                    loss.backward()
                    self.assertTrue(torch.isfinite(model.weight.grad).all())
                    self.assertGreater(float(model.weight.grad.norm()), 0)
                    optimizer.step()
                    after = train.expanded_model_weights(model).detach().numpy()
                    self.assertLess(direct_loss(records, base, after, 2), direct_loss(records, base, candidate, 2))
                    np.testing.assert_array_equal(model.weight[-1].detach().numpy(), 0)


class RemovalApiTest(unittest.TestCase):
    def test_reference_rejects_asymmetry_and_non_i16_inputs(self):
        good = constant_base()
        train.make_removal_reference(good[:, 0], good[:, 1], CPU)
        for endpoint in (0, 1):
            bad = good.copy()
            bad[0, endpoint] += 1
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError):
                train.make_removal_reference(bad[:, 0], bad[:, 1], CPU)
        with self.assertRaises(ValueError):
            train.make_removal_reference(good[:, 0].astype(np.float32), good[:, 1], CPU)

    def test_cli_requires_explicit_finite_nonnegative_model_appropriate_coefficient(self):
        arguments = ["train", "--data", "not-read.bin", "--init", "not-read.mnpt", "--output", "not-written.mnpt",
                     "--model", "mirrored", "--k", "2", "--lr", ".1", "--device", "cpu"]
        with redirect_stderr(StringIO()), self.assertRaises(SystemExit):
            train.main(arguments)
        for value in ("-1", "nan", "inf", "-inf"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                train.main(arguments + ["--removal-penalty=" + value])
        for kind in ("single", "tapered"):
            bad = arguments.copy()
            bad[bad.index("--model") + 1] = kind
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                train.main(bad + ["--removal-penalty", "1"])

    def test_positive_cli_rejects_asymmetric_initial_file_before_training(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, initial, output = root / "data.bin", root / "initial.bin", root / "output.bin"
            write_mnsd(data, seed=5, checksum=b"x" * 32, games=list(range(64)))
            base = constant_base()
            base[0, 1] = 1
            train.write_mnpt(initial, base[:, 0], base[:, 1], PIECE_VALUES, 2)
            with patch("train_pst.train_epoch", side_effect=AssertionError("invalid init reached training")), redirect_stdout(StringIO()):
                with self.assertRaises(ValueError):
                    train.main(["train", "--data", str(data), "--init", str(initial), "--output", str(output),
                                "--model", "mirrored", "--k", "2", "--lr", ".1", "--epochs", "1",
                                "--device", "cpu", "--removal-penalty", "1"])
            self.assertFalse(output.exists())

    def test_teacher_scales_use_only_explicit_fit_indices(self):
        with tempfile.TemporaryDirectory() as directory:
            a = Path(directory) / "a.bin"
            b = Path(directory) / "b.bin"
            scores = [100, -200, 300, -400, 1000, -1000, 2000, -2000]
            results = [0, 2, 2, 0, 2, 0, 2, 0]
            changed = scores[:4] + [-1000, 1000, -2000, 2000]
            write_mnsd(a, seed=5, checksum=b"a" * 32, games=list(range(8)), scores=scores, results=results)
            write_mnsd(b, seed=5, checksum=b"a" * 32, games=list(range(8)), scores=changed, results=results)
            first, second = Dataset([a]), Dataset([b])
            fit = np.array([0, 1, 2, 3], dtype=np.int64)
            ka, counts = train.estimate_generation_ks(first, indices=fit)
            kb, _ = train.estimate_generation_ks(second, indices=fit)
            np.testing.assert_array_equal(ka, kb)
            np.testing.assert_array_equal(counts, [4])
            self.assertEqual(train.estimate_mixed_k(first, indices=fit), train.estimate_mixed_k(second, indices=fit))
            self.assertNotEqual(train.estimate_mixed_k(first, indices=np.arange(8)), train.estimate_mixed_k(second, indices=np.arange(8)))

    def test_validation_is_pure_bce_on_explicit_calibration_indices(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "data.bin"
            board = records_with([[(0, 1), (142, 12), (143, 76)]])[0]["board"]
            write_mnsd(path, seed=5, checksum=b"v" * 32, games=list(range(4)),
                       scores=[-100, 300, 400, 200], results=[0, 1, 1, 2], board=board)
            dataset = Dataset([path])
            base = constant_base()
            model = train.make_model(torch.from_numpy(-base.astype(np.float32) / 8), CPU, "mirrored")
            calibration = np.array([3, 0], dtype=np.int64)
            # Both logits are -.5; targets are 1 and 0. No removal term enters validation.
            expected = float(np.logaddexp(0, -.5) + .25)
            with patch("train_pst.removal_loss", side_effect=AssertionError("validation must not add removal loss")):
                overall, by_generation = train.validation_loss(
                    model, dataset, np.array([2.0]), 2.0, 0.0, 1, CPU, indices=calibration,
                )
            self.assertAlmostEqual(overall, expected, delta=1e-7)
            np.testing.assert_allclose(by_generation, [expected], atol=1e-7, rtol=0)

    def test_positive_coefficient_changes_real_cpu_update_and_retains_projection(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "data.bin"
            records = records_with([[(0, 1), (142, 12), (143, 76)]])
            write_mnsd(path, seed=5, checksum=b"u" * 32, games=[0, 1],
                       scores=[0, 32767], results=[1, 2], board=records[0]["board"])
            dataset = Dataset([path])
            base = constant_base()
            candidate = -base.astype(np.float32) / 8
            expected_r = float(Fraction(25, 64))
            expected_bce = float(np.logaddexp(0, -.5) + .25)
            updated = {}
            for rho, step_size in [(0.0, .1), (2.0, .1), (2.0, 1e8)]:
                with self.subTest(rho=rho, step_size=step_size):
                    model = train.make_model(torch.from_numpy(candidate), CPU, "mirrored")
                    reference = train.make_removal_reference(base[:, 0], base[:, 1], CPU) if rho else None
                    result = train.train_epoch(
                        model, torch.optim.SGD(model.parameters(), lr=step_size), dataset,
                        np.array([2.0]), 2.0, 0.0, 1, torch.Generator().manual_seed(7), CPU,
                        count_features=True, indices=np.array([0]), removal_penalty=rho, removal_reference=reference,
                    )
                    self.assertAlmostEqual(result.bce_loss, expected_bce, delta=1e-7)
                    self.assertAlmostEqual(result.total_loss, expected_bce + rho * expected_r, delta=1e-7)
                    self.assertEqual(int(result.observations.sum()), 3)
                    if rho:
                        self.assertAlmostEqual(result.removal_loss, expected_r, delta=1e-7)
                    expanded = train.expanded_model_weights(model).detach().numpy()
                    self.assertTrue(np.all(np.isfinite(expanded)))
                    self.assertTrue(np.all(expanded >= -4096))
                    self.assertTrue(np.all(expanded <= 4095.875))
                    np.testing.assert_array_equal(model.weight[-1].detach().numpy(), 0)
                    if step_size == .1:
                        updated[rho] = expanded
                        self.assertLess(direct_loss(records, base, expanded, 2), direct_loss(records, base, candidate, 2))
                    else:
                        self.assertTrue(np.any(expanded == 4095.875) or np.any(expanded == -4096))
            self.assertGreater(updated[2.0][29 * 144, 1], updated[0.0][29 * 144, 1])

    def test_best_epoch_uses_bce_instead_of_training_total_or_removal_loss(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, initial, output = root / "data.bin", root / "initial.bin", root / "trained.bin"
            write_mnsd(data, seed=5, checksum=b"b" * 32, games=list(range(64)))
            base = constant_base()
            train.write_mnpt(initial, base[:, 0], base[:, 1], PIECE_VALUES, 2.0)
            calls = 0

            def epoch(model, *_args, **_kwargs):
                nonlocal calls
                calls += 1
                with torch.no_grad():
                    model.weight[:-1].fill_(calls + 1)
                return train.TrainEpochResult(.1 / calls, 500.0 if calls == 1 else 0.0,
                                              500.1 if calls == 1 else .05, None)

            losses = [(v, np.array([v])) for v in [.7, .6, .65]]
            log = StringIO()
            with patch("train_pst.train_epoch", side_effect=epoch), patch("train_pst.validation_loss", side_effect=losses), redirect_stdout(log):
                train.main(["train", "--data", str(data), "--init", str(initial), "--output", str(output),
                            "--model", "mirrored", "--k", "2", "--lr", ".1", "--epochs", "2", "--batch", "64",
                            "--device", "cpu", "--removal-penalty", "1"])
            self.assertIn("best epoch: 1 validation_loss=0.600000000", log.getvalue())
            mg, eg, _, _ = train.read_mnpt(output)
            np.testing.assert_array_equal(mg, 16)
            np.testing.assert_array_equal(eg, 16)


if __name__ == "__main__":
    unittest.main()
