"""層別標準化の期待値を手計算例と区間の定義から検証する。"""
import json
from pathlib import Path
import tempfile
import unittest

import numpy as np

from fm_distribution_comparison import compare, validate_evaluators


def sample(bands, pst, correction):
    return tuple(np.array(values) for values in (bands, pst, correction))


class StandardizationTests(unittest.TestCase):
    def test_match_frequencies_replace_saved_frequencies(self):
        # 保存: Aが3件×0、Bが1件×100で平均25。
        # 対局: Aが1件、Bが3件なので保存標準化平均75。
        saved = sample([0, 0, 0, 1], [0, 0, 0, 0], [0, 0, 0, 100])
        match = sample([0, 1, 1, 1], [0, 0, 0, 0], [10, 110, 110, 110])
        report = compare(saved, match, False)
        self.assertEqual(float(saved[2].mean()), 25)
        self.assertEqual(report["saved_standardized_mean"], 75)
        self.assertEqual(report["match_common_mean"], 85)
        self.assertEqual(report["difference"], 10)

    def test_negative_boundaries_use_floor_intervals(self):
        # [-1000,-500)には-501、[-500,0)には-500と-1。
        saved = sample([0, 0, 0, 0], [-501, -500, -1, 0], [10, 20, 40, 80])
        report = compare(saved, saved, False)
        rows = report["strata"]
        self.assertEqual([row["pst_lower_inclusive"] for row in rows], [-1000, -500, 0])
        self.assertEqual([row["saved_count"] for row in rows], [1, 2, 1])
        self.assertEqual([row["saved_mean"] for row in rows], [10, 30, 80])

    def test_center_window_includes_lower_excludes_upper_endpoint(self):
        saved = sample([0, 0, 0, 0], [-1001, -1000, 999, 1000], [900, 10, 30, 900])
        report = compare(saved, saved, True)
        self.assertEqual(report["source_counts"], {"saved": 2, "match": 2})
        self.assertEqual(report["saved_standardized_mean"], 20)

    def test_noncommon_strata_are_excluded_from_both_means_and_coverage(self):
        # 共通Aのみ。保存2/3件、対局1/4件を被覆し、非共通の大値は無関係。
        saved = sample([0, 0, 1], [0, 0, 0], [10, 30, 9000])
        match = sample([0, 2, 2, 2], [0, 0, 0, 0], [50, 9000, 9000, 9000])
        report = compare(saved, match, False)
        self.assertEqual(report["common_counts"], {"saved": 2, "match": 1})
        self.assertEqual(report["coverage"], {"saved": 2 / 3, "match": 1 / 4})
        self.assertEqual(report["saved_standardized_mean"], 20)
        self.assertEqual(report["difference"], 30)

    def test_no_common_strata_fails_explicitly(self):
        with self.assertRaises(ValueError):
            compare(sample([0], [0], [0]), sample([1], [0], [0]), False)

    def test_model_and_probe_must_match(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            left, right = root / "saved", root / "match"
            left.mkdir()
            right.mkdir()
            def write(path, pst, probe):
                (path / "report.json").write_text(json.dumps({"inputs": {
                    "pst": {"sha256": pst}, "probe": {"sha256": probe}}}))
            write(left, "weights-a", "probe-a")
            write(right, "weights-a", "probe-a")
            validate_evaluators(left, right)
            for pst, probe in (("weights-b", "probe-a"), ("weights-a", "probe-b")):
                write(right, pst, probe)
                with self.assertRaises(ValueError):
                    validate_evaluators(left, right)


if __name__ == "__main__":
    unittest.main()
