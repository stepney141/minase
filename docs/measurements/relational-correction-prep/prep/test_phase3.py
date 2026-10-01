"""設計書のGの定義、被覆率、および既存の駒除去手順を合成標本で検証する。"""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import phase3_common as shared
import g_candidate as candidate
import g_star
import removal
from features import INITIAL_BOARD
from mnsd import write_mnsd

np = shared.np


def arrays(bands, pst, correction):
    return tuple(np.array(x, dtype=np.int64) for x in (bands, pst, correction))


class GTests(unittest.TestCase):
    def test_same_comparator_and_match_frequency_weighting(self):
        # 共通2層の教師平均は10と30、対局平均は50と90。
        # 対局比率1:3による差は(50+270-10-90)/4 = 55。
        self.assertIs(candidate.compare, g_star.compare)
        self.assertIs(candidate.load, g_star.load)
        saved = arrays([0, 1, 1, 4], [-1, 0, 499, 9000], [10, 20, 40, 1000])
        match = arrays([0, 1, 1, 1, 3], [-500, 0, 1, 499, 500], [50, 80, 90, 100, -999])
        actual = candidate.comparison(saved, match)
        self.assertEqual(actual['difference'], 55)
        self.assertEqual(actual['coverage'], {'saved': .75, 'match': .8})
        self.assertEqual(actual['common_counts'], {'saved': 3, 'match': 4})
        self.assertEqual(actual['source_counts'], {'saved': 4, 'match': 5})

    def test_load_uses_phase_fifths_and_final_minus_pst(self):
        # N=2,20,38,56,74,92はq/90の帯境界に当たる。
        with tempfile.TemporaryDirectory(dir='/tmp') as name:
            directory = Path(name)
            records = np.zeros(6, dtype=shared.RECORD_DTYPE)
            records['lion'] = 255
            for record, n in zip(records, [2, 20, 38, 56, 74, 92]):
                record['board'][:n] = 1
            write_mnsd(directory / 'sample.bin', records, seed=1, network_checksum=bytes(32))
            (directory / 'probe.json').write_text(json.dumps([
                {'index': i, 'eval_pst': 28990, 'eval': 28999} for i in range(6)]))
            bands, pst, correction = candidate.load(directory)
            np.testing.assert_array_equal(bands, [0, 1, 2, 3, 4, 4])
            np.testing.assert_array_equal(correction, [9] * 6)

    def test_coverage_boundary_and_baseline_adjustment(self):
        base = {'coverage': {'match': .9}}
        record = {'coverage': {'match': .9}, 'difference': candidate.G_STAR_CP}
        self.assertTrue(candidate.decision(record, base)['meets_criterion'])
        record['coverage']['match'] = .899
        self.assertFalse(candidate.decision(record, base)['determinate'])
        base['coverage']['match'] = .8
        record['coverage']['match'] = .75
        result = candidate.decision(record, base)
        self.assertTrue(result['meets_criterion'])
        self.assertFalse(result['determinate_at_90_percent'])
        record['difference'] += .001
        self.assertFalse(candidate.decision(record, base)['meets_criterion'])

    def test_no_common_strata_is_reported_as_indeterminate(self):
        result = candidate.comparison(arrays([0], [0], [0]), arrays([1], [0], [10]))
        self.assertIsNone(result['difference'])
        self.assertFalse(candidate.decision(result, result)['determinate'])


class RemovalTests(unittest.TestCase):
    def test_strict_sign_product_excludes_zero_and_records_skips(self):
        before = [{'eval_pst': 100, 'eval': 200}]
        after = [{'eval_pst': 90, 'eval': 205}, {'eval_pst': 110, 'eval': 195},
                 {'eval_pst': 100, 'eval': 190}, {'eval_pst': 110, 'eval': 200},
                 {'eval_pst': 90, 'eval': 190}, {'skipped': 'missing lion'}]
        result = removal.summarize(before, after, [{'position': 0, 'square': i} for i in range(6)])
        self.assertEqual(result['sign_reversals'], 2)
        self.assertEqual([r['square'] for r in result['examples']], [0, 1])
        self.assertEqual((result['attempted'], result['checked'], result['skipped']), (6, 5, 1))

    def test_removes_each_nonroyal_and_preserves_context(self):
        records = np.zeros(1, dtype=shared.RECORD_DTYPE)
        # 王将、太子、敵王将、敵太子、双方の歩兵。
        records['board'][0, :6] = [12, 51, 76, 115, 1, 65]
        records['stm'], records['lion'], records['kirin'] = 1, 5, 1
        after, locations = removal.variants(records)
        self.assertEqual([r['square'] for r in locations], [4, 5])
        for row, square in zip(after, [4, 5]):
            self.assertEqual(row['board'][square], 0)
            self.assertEqual(np.count_nonzero(row['board']), 5)
            self.assertEqual((row['stm'], row['lion'], row['kirin']), (1, 5, 1))

    def test_representatives_are_initial_and_minimum_sample_per_band(self):
        class Dataset:
            def gather(self, indices):
                result = np.zeros(len(indices), dtype=shared.RECORD_DTYPE)
                result['game'] = indices
                return result
        samples = [{'generation': 0, 'band': 1, 'indices': np.array([8, 10])},
                   {'generation': 1, 'band': 1, 'indices': np.array([3, 12])},
                   {'generation': 0, 'band': 4, 'indices': np.array([5])},
                   {'generation': 0, 'band': 0, 'indices': np.array([], dtype=int)}]
        with patch.object(removal, 'band_samples', return_value=samples) as select:
            dataset = Dataset()
            records, labels, _ = removal.representatives(dataset, 10000, 1)
            select.assert_called_once_with(dataset, 10000, 1)
        self.assertEqual([r['index'] for r in labels], [None, 3, 5])
        np.testing.assert_array_equal(records[0]['board'], INITIAL_BOARD)
        self.assertEqual(records[0]['lion'], 255)

    def test_zeroing_preserves_pst_and_scale(self):
        from dataclasses import replace
        from mnpt_v3 import decode
        with tempfile.TemporaryDirectory(dir='/tmp') as name:
            source, target = Path(name) / 'candidate.bin', Path(name) / 'zero.bin'
            model = decode((shared.WT / 'nets/pst.bin').read_bytes())
            changed = replace(model, pair_weights=(8,) * len(model.pair_weights))
            source.write_bytes(changed.encode())
            shared.zero_weights(source, target)
            result = decode(target.read_bytes())
            self.assertEqual(result.base_body, model.base_body)
            self.assertEqual(result.k, model.k)
            self.assertFalse(any(result.pair_weights))


if __name__ == '__main__':
    unittest.main()
