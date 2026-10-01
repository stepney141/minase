"""指示書と計画の数式から定めた、符号、境界、層の重みの検査。

仕様の対応:
* relational-correction.md「誤りを分類する」: 根視点、差 >= 117.7。
* 同「利用者の判断による測定」: 検証だけから非復元抽出、標本ごとにシード1。
* relative-pair-eval.md「自己選択の大きさ…」: 全域、共通層、対局頻度の重み。
"""
import sys
import unittest
from types import SimpleNamespace

from common import BASE
from proxy import compare_scores
from samples import select
import numpy as np

sys.path.insert(0, str(BASE / 'fm-eval-tools/tools/train/pst'))
from fm_distribution_comparison import compare


class PrepTests(unittest.TestCase):
    def test_root_perspective_and_threshold(self):
        for a, s, difference, supports in (
                (-200, -82, 118, True), (-200, -83, 117, False),
                (200, 82, -118, False), (0, 117.7, 117.7, True)):
            with self.subTest(a=a, s=s):
                result = compare_scores(a, s)
                self.assertEqual(result['difference_cp'], difference)
                self.assertEqual(result['supports_a_star'], supports)

    def test_validation_only_without_replacement(self):
        dataset = SimpleNamespace(
            offsets=np.array([0, 4, 8]), validation_indices=np.array([1, 4, 7]),
            validation_masks=(np.array([False, True, False, False]),
                              np.array([True, False, False, True])))
        all_rows = select(dataset, 3)['rows']
        self.assertEqual(set(map(tuple, all_rows)), {(0, 1), (1, 0), (1, 3)})
        self.assertEqual(len(all_rows), 3)
        first = select(dataset, 2)
        select(dataset, 1)
        self.assertEqual(first, select(dataset, 2))

    def test_full_range_common_strata_with_match_weights(self):
        # 共通層は[-500,0), [500,1000), [2000,2500)。対局頻度は1:2:1。
        # [0,500), [3000,3500)は対局側にしかないので平均から除く。
        saved = (np.array([0, 1, 4]), np.array([-100, 500, 2000]),
                 np.array([10, 20, 30]))
        match = (np.array([0, 0, 1, 1, 4, 4]),
                 np.array([-1, 499, 500, 500, 2000, 3000]),
                 np.array([30, 999, 60, 100, 200, 999]))
        result = compare(saved, match, center_only=False)
        self.assertEqual(result['common_stratum_count'], 3)
        self.assertEqual(result['coverage']['match'], 4 / 6)
        self.assertEqual(result['saved_standardized_mean'], 20)
        self.assertEqual(result['match_common_mean'], 97.5)
        self.assertEqual(result['difference'], 77.5)


if __name__ == '__main__':
    unittest.main()
