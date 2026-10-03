"""先読み教師の事前診断を独立した参照値で検証する。"""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import numpy as np

import minase_train.data.mnsd as mnsd
import minase_train.diagnostics.lookahead_teacher as diag
from helpers import write_provenance
from minase_train.checksum import sha256_file
from minase_train.data.features import FEATURE_COUNT
from minase_train.data.mnpt import initial_piece_values, write_mnpt
from minase_train.data.mnsd import Dataset, HEADER, RECORD_DTYPE, hash64


class LookaheadIntegrationTest(unittest.TestCase):

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.path = self.root / 'source.bin'
        candidates = np.arange(1, 100, dtype='u4')
        self.game = int(candidates[hash64(11, candidates) % 20 != 0][0])
        self.rows = np.zeros(4, dtype=RECORD_DTYPE)
        self.rows['game'] = self.game
        self.rows['ply'] = [0, 1, 3, 10]
        self.rows['score'] = [100, 200, -400, 50]
        self.rows['result'] = [2, 0, 0, 2]
        self.rows['lion'] = 255
        for row, count in zip(self.rows, [78, 77, 61, 46]):
            row['board'][:count] = 1
        mnsd.write_mnsd(self.path, self.rows, seed=11, network_checksum=b'a' * 32)
        write_provenance(self.path)
        self.current = Dataset([self.path])
        self.future = Dataset([self.path], lookahead={'gamma': .9, 'plies': 3})
        self.expected = np.array([124 / 1.81, -400, -400, 50])

    def diagnostic_files(self):
        feature_path = self.root / 'features.mnkf'
        values = np.zeros((4, 118), dtype='u1')
        values[:, 0] = [0, 1, 2, 3]
        values[:, 117] = [3, 2, 1, 0]
        feature_path.write_bytes(HEADER.pack(b'MNKF', 1, 2, 118, 4, sha256_file(self.path)) + values.tobytes())
        pst = self.root / 'pst.bin'
        zero = np.zeros(FEATURE_COUNT, dtype='i2')
        write_mnpt(pst, zero, zero, initial_piece_values(), 100)
        sample = self.root / 'sample.json'
        sample.write_text(json.dumps({'format': 'depth-sensitivity-sample', 'version': 1,
            'files': [{'file': str(self.path), 'sha256': sha256_file(self.path).hex()}],
            'positions': [{'file': str(self.path), 'index': i, 'group': 'exposed' if i < 2 else 'control'} for i in range(4)]}))
        return feature_path, pst, sample

    def test_diagnostic_metrics_against_hand_values(self):
        features, pst, sample = self.diagnostic_files()
        # Kを固定し、分布・混合勝率・残差の数値を推定器と独立に確認する。
        with patch('minase_train.diagnostics.lookahead_teacher.estimate_generation_ks', side_effect=[(np.array([100.]), [4]), (np.array([200.]), [4])]):
            report = diag.diagnose([self.path], [features], pst, 100, [.9, .7, .95], 3, sample, batch=2)
        delta = np.array([124 / 1.81 - 100, -600, 0, 0])
        entry = report['gammas']['0.9']
        self.assertAlmostEqual(entry['difference_cp']['mean'], delta.sum() / 4)
        self.assertAlmostEqual(entry['difference_cp']['std'], np.sqrt(np.sum((delta - delta.mean()) ** 2) / 4))
        np.testing.assert_allclose(list(entry['difference_cp']['quantiles'].values()),
                                   [-514.7237569060774, -173.61878453038673, -15.74585635359116, 0, 0])
        self.assertEqual(entry['difference_cp']['sharp_drop_fraction'], .25)
        for name, value in zip(('78以上', '62〜77', '47〜61', '46以下'), delta):
            self.assertAlmostEqual(entry['piece_bands'][name]['mean'], value)
        self.assertEqual(entry['windows']['fallback_fraction'], .5)
        self.assertEqual(entry['windows']['record_count']['mean'], .75)
        self.assertEqual(entry['windows']['first_record_plies']['mean'], 1.5)
        old = (.75 / (1 + np.exp(-self.rows['score'].astype(float) / 100)) + .25 * self.rows['result'] / 2).astype('f4')
        new = (.75 / (1 + np.exp(-self.expected / 200)) + .25 * self.rows['result'] / 2).astype('f4')
        for name, sl in (('exposed', slice(0, 2)), ('control', slice(2, 4))):
            self.assertAlmostEqual(report['exposed_sample']['groups'][name]['mean'], np.mean(new[sl].astype(float) - old[sl]))
        columns = report['residual_correlations']['columns']
        for field, target in (('current', old), ('lookahead', new)):
            correlation = np.corrcoef(np.arange(4), target.astype(float) - .5)[0, 1]
            self.assertAlmostEqual(columns[0][field], correlation)
            self.assertAlmostEqual(columns[117][field], -correlation)
            self.assertIsNone(columns[1][field])
        self.assertEqual(report['teacher_k_comparison'][0]['current_k'], 100)
        self.assertEqual(report['teacher_k_comparison'][0]['lookahead_k'], 200)
        json.dumps(report, allow_nan=False)

    def test_empty_distribution_and_invalid_sample(self):
        self.assertIsNone(diag.distribution([])['mean'])
        _, _, sample = self.diagnostic_files()
        data = json.loads(sample.read_text())
        data['positions'][0]['index'] = 4
        sample.write_text(json.dumps(data))
        with self.assertRaises(ValueError):
            diag.sample_indices(self.current, sample)


if __name__ == "__main__":
    unittest.main()
