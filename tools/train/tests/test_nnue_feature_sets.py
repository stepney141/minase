"""共通仕様の王駒選択、8区分、元の升との対応、および基底特徴を検証する。"""

from dataclasses import FrozenInstanceError
import unittest

import numpy as np

from minase_train.data.features import feature_indices
from minase_train.nnue.feature_sets import FEATURE_SETS, get_feature_set


class FeatureSetsTest(unittest.TestCase):
    def test_registry_has_only_two_immutable_specs(self):
        self.assertEqual(set(FEATURE_SETS), {"P", "HalfKP-hm-b8"})
        for name, count, mirror, factorized in (("P", 13680, "shared", False),
                ("HalfKP-hm-b8", 109440, "canonical", True)):
            spec = get_feature_set(name)
            self.assertEqual((spec.name, spec.feature_count, spec.mirror, spec.factorized),
                             (name, count, mirror, factorized))
            with self.assertRaises(FrozenInstanceError):
                spec.name = "unknown"
        with self.assertRaises(ValueError):
            get_feature_set("unknown")

    def test_p_base_matches_existing_numbering_for_all_piece_bytes_and_squares(self):
        pieces = np.array([0, *range(1, 59), *range(65, 123)], dtype=np.uint8)
        board = np.repeat(pieces[:, None], 144, axis=1)
        for side in (0, 1):
            sides = np.full(len(board), side, dtype=np.uint8)
            for lion in (0, 143, 255):
                lions = np.full(len(board), lion, dtype=np.uint8)
                expected = feature_indices(board, sides, lions)
                for spec in FEATURE_SETS.values():
                    base, features = spec.indices(board, lions, sides)
                    np.testing.assert_array_equal(base, expected)
                    self.assertEqual(base.dtype, np.int32)
                    self.assertEqual(features.dtype, np.int32)
                    if spec.name == "P":
                        np.testing.assert_array_equal(features, expected)

    def test_eight_buckets_and_reflection_keep_original_square_columns(self):
        # 50升の歩兵は基底4226、鏡映後4233。左右6筋で区分0〜5を固定する。
        cases = [(0, 4233), (1, 17913), (2, 31593), (3, 45273), (4, 58953), (5, 72633),
                 (6, 72626), (7, 58946), (8, 45266), (9, 31586), (10, 17906), (11, 4226),
                 (17, 86313), (18, 86306), (29, 99993), (30, 99986), (143, 99986)]
        board = np.zeros((len(cases), 144), dtype=np.uint8)
        board[:, 50] = 1
        board[np.arange(len(cases)), [square for square, _ in cases]] = 12
        spec = get_feature_set("HalfKP-hm-b8")
        base, features = spec.indices(board, np.full(len(board), 255, dtype=np.uint8),
                                     np.zeros(len(board), dtype=np.uint8))
        np.testing.assert_array_equal(base[:, 50], 4226)
        np.testing.assert_array_equal(features[:, 50], [expected for _, expected in cases])
        np.testing.assert_array_equal(features[:, 57], 109440)
        np.testing.assert_array_equal(features[:, 144], 109440)
        np.testing.assert_array_equal(base[:, 144], 13680)
        np.testing.assert_array_equal(features[:, :144][board == 0], 109440)
        np.testing.assert_array_equal(base[:, :144][board == 0], 13680)

    def test_royal_priority_lowest_absolute_square_and_missing_royals(self):
        # 先手の歩兵50升を固定し、王駒だけで文脈を変える。
        cases = [([(5, 22), (10, 12)], 0, 17906),
                 ([(5, 22)], 0, 72633),
                 ([(5, 12), (10, 12)], 0, 72633),
                 ([(5, 22), (10, 22)], 0, 72633),
                 ([(5, 76), (10, 86)], 0, 4226),
                 ([], 0, 4226),
                 ([(0, 76), (143, 76)], 1, 106797),
                 ([(0, 86), (143, 86)], 1, 106797),
                 ([(0, 86), (143, 76)], 1, 11030),
                 ([], 1, 11030)]
        board = np.zeros((len(cases), 144), dtype=np.uint8)
        board[:, 50] = 1
        for row, (royals, _, _) in zip(board, cases):
            for square, piece in royals:
                row[square] = piece
        _, features = get_feature_set("HalfKP-hm-b8").indices(
            board, np.full(len(board), 255, dtype=np.uint8),
            np.array([side for _, side, _ in cases], dtype=np.uint8))
        np.testing.assert_array_equal(features[:, 50], [expected for _, _, expected in cases])

    def test_white_relative_rank_and_lion_use_same_context(self):
        # 後手の王を最下段の5筋、6筋、2段目、3段目に置く。
        board = np.zeros((4, 144), dtype=np.uint8)
        board[np.arange(4), [137, 138, 131, 119]] = 76
        base, features = get_feature_set("HalfKP-hm-b8").indices(
            board, np.full(4, 13, dtype=np.uint8), np.ones(4, dtype=np.uint8))
        np.testing.assert_array_equal(base[:, 144], [13657, 13657, 13657, 13657])
        np.testing.assert_array_equal(features[:, 144], [82066, 82057, 95737, 109417])
        # 後手も筋は反転せず、段だけを反転する。
        np.testing.assert_array_equal(base[np.arange(4), [137, 138, 131, 119]],
                                      [1589, 1590, 1607, 1619])

    def test_feature_factorization_maps_all_buckets_to_base_rows(self):
        spec = get_feature_set("HalfKP-hm-b8")
        np.testing.assert_array_equal(spec.base_indices(np.array([0, 13679, 13680, 95760, 109439])),
                                      [0, 13679, 0, 0, 13679])
        np.testing.assert_array_equal(get_feature_set("P").base_indices(np.array([0, 13549, 13679])),
                                      [0, 13549, 13679])

    def test_empty_batches_and_invalid_board_inputs(self):
        for spec in FEATURE_SETS.values():
            base, features = spec.indices(np.zeros((0, 144), dtype=np.uint8),
                                         np.zeros(0, dtype=np.uint8), np.zeros(0, dtype=np.uint8))
            self.assertEqual(base.shape, (0, 145))
            self.assertEqual(features.shape, (0, 145))
            for board, lion, side in ((np.zeros((1, 143)), [255], [0]),
                                      (np.zeros((1, 144)), [144], [0]),
                                      (np.zeros((1, 144)), [255], [2]),
                                      (np.full((1, 144), 64), [255], [0])):
                with self.subTest(spec=spec.name), self.assertRaises(ValueError):
                    spec.indices(board, lion, side)


if __name__ == "__main__":
    unittest.main()
