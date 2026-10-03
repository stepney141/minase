"""先読み教師の幾何加重平均と窓の統計を検証する。"""

from __future__ import annotations

import unittest

import numpy as np

from minase_train.data.lookahead import compute_lookahead, lookahead_options, window_statistics


class LookaheadTest(unittest.TestCase):

    def test_hand_calculated_missing_plies_boundary_sign_fraction_and_fallback(self):
        game = np.array([1, 1, 1, 1, 2, 2], dtype='u4')
        ply = np.array([0, 1, 3, 8, 0, 2], dtype='u2')
        score = np.array([7, 2, 1, 99, 10, -5], dtype='i2')
        # t=0: (-2 + .25*(-1))/(1+.25) = -1.8。t=1から3は同じ手番。
        expected = [-1.8, 1, 1, 99, -5, -5]
        actual = compute_lookahead(game, ply, score, .5, 3)
        np.testing.assert_allclose(actual, expected)
        self.assertEqual(actual.dtype, np.float64)
        np.testing.assert_array_equal(actual, compute_lookahead(game, ply, score, .5, 3))
        np.testing.assert_array_equal(score, [7, 2, 1, 99, 10, -5])
        stats = window_statistics(game, ply, 3)
        np.testing.assert_array_equal(stats['record_count'], [2, 1, 0, 0, 1, 0])
        np.testing.assert_array_equal(stats['first_record_plies'], [1, 2, 0, 0, 2, 0])
        np.testing.assert_array_equal(stats['fallback'], [False, False, True, True, False, True])

    def test_bounds_empty_and_underflow(self):
        np.testing.assert_array_equal(compute_lookahead(np.array([], dtype='u4'), np.array([], dtype='u2'), [], .9, 40), [])
        np.testing.assert_array_equal(compute_lookahead([1, 1], [65534, 65535], [1, -32768], .9, 1), [32768, -32768])
        # 正規化前の重みがアンダーフローする場合も、1記録の平均はその値。
        np.testing.assert_array_equal(compute_lookahead([1, 1], [0, 4096], [7, 123], 1e-200, 4096), [123, 123])
        np.testing.assert_array_equal(compute_lookahead([1, 1], [0, 4097], [7, 123], .9, 4096), [7, 123])

    def test_invalid_parameters_and_order(self):
        for gamma, plies in [(0, 40), (1, 40), (-.1, 40), (float('nan'), 40), (float('inf'), 40),
                             (.9, 0), (.9, 4097), (.9, 1.5), (.9, True), (True, 40)]:
            with self.subTest(gamma=gamma, plies=plies), self.assertRaises(ValueError):
                compute_lookahead([1], [0], [0], gamma, plies)
        for game, ply in [([2, 1], [0, 1]), ([1, 1], [1, 1]), ([1, 1], [2, 1]), ([1, 2, 1], [0, 0, 1])]:
            with self.subTest(game=game, ply=ply), self.assertRaises(ValueError):
                compute_lookahead(game, ply, [0] * len(game), .9, 40)
        for options in ((.9, None), (None, 40)):
            with self.assertRaises(ValueError):
                lookahead_options(*options)
        self.assertIsNone(lookahead_options(None, None))

    def test_block_boundary_matches_direct_definition(self):
        # ブロック境界をまたぐ長い対局でも、少数の指定局面を式から独立に確認する。
        game = np.repeat(np.arange(2, dtype='u4'), 40000)
        ply = np.tile(np.arange(40000, dtype='u2'), 2)
        score = ((np.arange(80000) % 11) - 5).astype('i2')
        actual = compute_lookahead(game, ply, score, .9, 40)
        for t in (0, 39998, 39999, 40000, 65534, 65535, 65536, 79999):
            successors = [s for s in range(t + 1, min(t + 41, len(game))) if game[s] == game[t]]
            weights = [.9 ** (int(ply[s]) - int(ply[t]) - 1) for s in successors]
            expected = (sum(w * (-1) ** (int(ply[s]) - int(ply[t])) * int(score[s])
                            for s, w in zip(successors, weights)) / sum(weights)) if successors else score[t]
            self.assertAlmostEqual(actual[t], expected)


if __name__ == "__main__":
    unittest.main()
