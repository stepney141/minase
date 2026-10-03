"""駒状態と左右鏡映による特徴番号の対応を検証する。"""

from __future__ import annotations

import unittest

import numpy as np

from minase_train.data.features import (
    FEATURE_COUNT,
    MIRRORED_FEATURE_COUNT,
    canonical_feature_indices,
)


class MirroredModelTest(unittest.TestCase):

    """strength-stage7.md「鏡映の重み共有」の全特徴と学習経路の契約を検証する。"""

    def test_canonical_mapping_preserves_ranks_and_has_exactly_6840_pairs(self) -> None:
        # 各陣営・駒状態と先獅子の表は12段×12筋で、左右の筋だけを同一視する。
        indices = np.arange(FEATURE_COUNT, dtype=np.int32).reshape(95, 12, 12)
        canonical = canonical_feature_indices(indices)
        np.testing.assert_array_equal(canonical, canonical[:, :, ::-1])
        values, counts = np.unique(canonical, return_counts=True)
        np.testing.assert_array_equal(values, np.arange(6840))
        np.testing.assert_array_equal(counts, 2)
        self.assertEqual(MIRRORED_FEATURE_COUNT, 6840)
        # 先頭、中央、次段、次状態、敵駒、先獅子、末尾、paddingの境界。
        np.testing.assert_array_equal(
            canonical_feature_indices(np.array([0, 5, 6, 11, 12, 144, 6768, 13536, 13679, 13680])),
            [0, 5, 5, 0, 6, 72, 3384, 6768, 6834, 6840],
        )


if __name__ == "__main__":
    unittest.main()
