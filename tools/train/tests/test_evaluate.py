"""整数と実数によるPSTの参照評価を検証する。"""

from __future__ import annotations

import unittest

import numpy as np

from minase_train.data.features import FEATURE_COUNT, PADDING_INDEX
from minase_train.pst.evaluate import integer_evaluate


class TaperedFormatTest(unittest.TestCase):

    """補間係数、整数評価の式、MNPTバージョン2の検査、およびモデル種別の契約を検証する。"""

    def test_integer_evaluate_interpolates_truncates_and_clips(self) -> None:
        middlegame = np.zeros(FEATURE_COUNT, dtype=np.int16)
        endgame = np.zeros(FEATURE_COUNT, dtype=np.int16)
        middlegame[0] = 800  # 100 cp
        endgame[0] = -1600  # -200 cp
        features = np.full((5, 145), PADDING_INDEX, dtype=np.int32)
        features[:, 0] = 0
        numerators = np.array([90, 0, 45, 1, 89], dtype=np.int64)
        # q=45: (45×800 + 45×(−1600))/720 = −50。q=1: (800 − 89×1600)/720 = −196.6 → −196。
        # q=89: (89×800 − 1600)/720 = 96.67 → 96。
        np.testing.assert_array_equal(
            integer_evaluate(middlegame, endgame, features, numerators), [100, -200, -50, -196, 96]
        )
        # 分子−719は0、−720は−1へ切り捨てる。
        middlegame[0] = 0
        endgame[0] = 0
        middlegame[1] = -719
        endgame[1] = 0
        features[:, 1] = 1
        np.testing.assert_array_equal(
            integer_evaluate(middlegame, endgame, features[:1], np.array([1])), [0]
        )
        middlegame[1] = -720
        np.testing.assert_array_equal(
            integer_evaluate(middlegame, endgame, features[:1], np.array([1])), [-1]
        )
        clipped = np.full(FEATURE_COUNT, 32_000, dtype=np.int16)
        wide = np.zeros((1, 145), dtype=np.int32)
        wide[0, :144] = np.arange(144)
        wide[0, 144] = PADDING_INDEX
        np.testing.assert_array_equal(
            integer_evaluate(clipped, clipped, wide, np.array([30])), [28_999]
        )
        with self.assertRaises(ValueError):
            integer_evaluate(middlegame, endgame, features[:1], np.array([91]))


if __name__ == "__main__":
    unittest.main()
