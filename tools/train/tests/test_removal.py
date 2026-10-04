"""駒除去の参照評価と追加損失を独立した式で検証する。"""

from __future__ import annotations

import unittest
from fractions import Fraction

import numpy as np
import torch

import minase_train.pst.removal as removal
from helpers import CPU, PROMOTABLE, constant_base, direct_loss, model_and_loss, records_with
from minase_train.data.features import feature_indices, mirror


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
            for candidate_factor, expected in [(-1, Fraction(5, 8)), (Fraction(1, 8), Fraction(1, 16)),
                                               (Fraction(1, 4), Fraction()), (1, Fraction()), (2, Fraction())]:
                with self.subTest(byte=byte, factor=candidate_factor):
                    candidate = base.astype(np.float64) / 8 * float(candidate_factor)
                    _, loss = self.assert_oracle(records, base, candidate)
                    self.assertAlmostEqual(float(loss.detach()), float(expected), delta=1e-12)

    def test_physically_wrong_baseline_has_lower_and_upper_penalties_on_both_sides(self):
        base = constant_base(-8, 8)
        for byte in (1, 65):
            records = records_with([[(0, byte), (142, 12), (143, 76)]])
            for factor, expected in [(-1, Fraction(5, 8)), (Fraction(1, 2), Fraction()),
                                     (1, Fraction()), (2, Fraction(1, 2))]:
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
        self.assertAlmostEqual(float(loss.detach()), float(Fraction(1, 16)), delta=1e-12)

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
        # Third has only kings. Loss=(5/8 + 5/16 + 0)/3=5/16.
        records = records_with([[(0, 1), (142, 12), (143, 76)],
                                [(0, 1), (2, 2), (142, 12), (143, 76)], [(142, 12), (143, 76)]])
        _, loss = self.assert_oracle(records, base, candidate)
        self.assertAlmostEqual(float(loss.detach()), float(Fraction(5, 16)), delta=1e-12)

    def test_integer_zero_removal_is_not_counted_in_a_positions_denominator(self):
        base = constant_base()
        base[30 * 144:31 * 144] = 1
        candidate = base.astype(np.float64) / 8
        candidate[29 * 144:30 * 144] = -1
        records = records_with([[(0, 1), (2, 2), (142, 12), (143, 76)]])
        _, loss = self.assert_oracle(records, base, candidate)
        self.assertAlmostEqual(float(loss.detach()), float(Fraction(5, 8)), delta=1e-12)

    def test_royals_are_excluded_and_shared_canonical_occurrences_remove_only_one(self):
        base = constant_base()
        candidate = -base.astype(np.float64) / 8
        # Squares 0 and 11 share one parameter; removing one pawn must retain the other.
        records = records_with([[(0, 1), (11, 1), (130, 51), (142, 12), (143, 76)]])
        _, loss = self.assert_oracle(records, base, candidate)
        self.assertAlmostEqual(float(loss.detach()), float(Fraction(5, 8)), delta=1e-12)

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
                self.assertAlmostEqual(float(loss.detach()), float(Fraction(5, 8)), delta=1e-12)
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
        self.assertAlmostEqual(float(loss.detach()), float(Fraction(13, 8)), delta=1e-12)

    def test_violation_gradient_is_constant_over_k_and_zero_at_each_boundary(self):
        # One synthetic pawn isolates S=s0*delta from phase and averaging factors.
        records = records_with([[(0, 1)]])
        features = torch.from_numpy(feature_indices(records["board"], records["stm"], records["lion"]))
        counts = torch.tensor([1], dtype=torch.int64, device=CPU)
        for baseline_wrong in (False, True):
            base = constant_base(-8, 8) if baseline_wrong else constant_base()
            reference = removal.make_removal_reference(base[:, 0], base[:, 1], CPU)
            base_sign = 1 if baseline_wrong else -1
            for k in (2, 4):
                for signed_value in (Fraction(-20), Fraction(-2), Fraction(1, 8), Fraction(1, 4),
                                     Fraction(1, 2), Fraction(1), Fraction(2), Fraction(20)):
                    with self.subTest(baseline_wrong=baseline_wrong, k=k, signed_value=signed_value):
                        signed_delta = torch.tensor(float(signed_value), dtype=torch.float64, requires_grad=True)
                        weights = torch.zeros((1, 145, 2), dtype=torch.float64, device=CPU)
                        # q=0 both before and after removal, so delta=-pawn_EG.
                        weights[0, 0, 1] = -base_sign * signed_delta
                        loss = removal.removal_loss(weights, features, reference, counts, k)
                        expected = max(Fraction(), Fraction(1, 4) - signed_value)
                        if baseline_wrong:
                            expected += max(Fraction(), signed_value - 1)
                        self.assertAlmostEqual(float(loss.detach()), float(expected / k), delta=1e-12)
                        loss.backward()
                        if signed_value < Fraction(1, 4):
                            gradient = -Fraction(1, k)
                        elif baseline_wrong and signed_value > 1:
                            gradient = Fraction(1, k)
                        else:
                            gradient = Fraction()
                        self.assertEqual(float(signed_delta.grad), float(gradient))

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

class RemovalApiTest(unittest.TestCase):

    def test_reference_rejects_asymmetry_and_non_i16_inputs(self):
        good = constant_base()
        removal.make_removal_reference(good[:, 0], good[:, 1], CPU)
        for endpoint in (0, 1):
            bad = good.copy()
            bad[0, endpoint] += 1
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError):
                removal.make_removal_reference(bad[:, 0], bad[:, 1], CPU)
        with self.assertRaises(ValueError):
            removal.make_removal_reference(good[:, 0].astype(np.float32), good[:, 1], CPU)


if __name__ == "__main__":
    unittest.main()
