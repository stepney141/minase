"""設計書の幾何加重平均と実数教師の受け渡しを独立の参照値で検査する。"""

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

from features import FEATURE_COUNT
from lookahead import compute_lookahead, lookahead_options, window_statistics
from mnsd import Dataset, HEADER, RECORD_DTYPE, hash64, sha256_file, write_mnsd
from test_train_pst import write_provenance
from train_pst import (build_targets, estimate_generation_ks, estimate_k, make_model,
                       validation_loss, train_epoch, initial_piece_values, write_mnpt)
import lookahead_diag as diag


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
        write_mnsd(self.path, self.rows, seed=11, network_checksum=b'a' * 32)
        write_provenance(self.path)
        self.current = Dataset([self.path])
        self.future = Dataset([self.path], lookahead={'gamma': .9, 'plies': 3})
        self.expected = np.array([124 / 1.81, -400, -400, 50])

    def test_teacher_k_classification_float_targets_and_gather(self):
        np.testing.assert_allclose(self.future.teacher_scores(np.array([3, 0, 2, 1])), self.expected[[3, 0, 2, 1]])
        np.testing.assert_array_equal(self.future.gather(np.arange(4))['score'], self.rows['score'])
        np.testing.assert_array_equal(self.current.training_indices, self.future.training_indices)
        self.assertNotEqual(self.current.teacher_classes, self.future.teacher_classes)
        self.assertEqual(self.future.class_metadata()[0]['lookahead_gamma'], .9)
        self.assertEqual(self.future.class_metadata()[0]['lookahead_plies'], 3)
        with patch('train_pst.estimate_k', wraps=estimate_k) as estimate:
            ks, _ = estimate_generation_ks(self.future, indices=self.future.training_indices)
            np.testing.assert_allclose(estimate.call_args.args[0], self.expected)
        original_k, _ = estimate_generation_ks(self.current, indices=self.current.training_indices)
        self.assertNotAlmostEqual(ks[0], original_k[0], places=3)
        targets = build_targets(self.rows, [100], np.zeros(4, dtype=int), [.75], scores=self.future.teacher_scores(np.arange(4)))
        expected = .75 / (1 + np.exp(-self.expected / 100)) + .25 * self.rows['result'] / 2
        np.testing.assert_array_equal(targets, expected.astype('f4'))
        rounded = .75 / (1 + np.exp(-self.expected.astype('i2') / 100)) + .25 * self.rows['result'] / 2
        self.assertNotEqual(targets[0], np.float32(rounded[0]))
        with self.assertRaisesRegex(ValueError, 'lookahead.*rescore'):
            Dataset([self.path], lookahead={'gamma': .9, 'plies': 3}, rescore=['missing.bin'])
        Dataset([self.path], lookahead={'gamma': .9, 'plies': 3}, rescore=['-'])

    def test_multiple_files_keep_local_windows_and_global_indices(self):
        other = self.root / 'other.bin'
        rows = self.rows.copy()
        rows['score'] = [200, 50, -75, 99]
        write_mnsd(other, rows, seed=13, network_checksum=b'a' * 32)
        write_provenance(other)
        dataset = Dataset([self.path, other], lookahead={'gamma': .9, 'plies': 3})
        np.testing.assert_allclose(dataset.teacher_scores(np.array([4, 0, 5, 1, 7, 3])),
                                   [10.75 / 1.81, 124 / 1.81, -75, -400, 99, 50])
        self.assertEqual(dataset.generation_count, 1)
        for options in ({'gamma': .7, 'plies': 3}, {'gamma': .9, 'plies': 40}):
            self.assertNotEqual(self.future.teacher_classes, Dataset([self.path], lookahead=options).teacher_classes)

    def test_validation_loss_uses_real_teacher_values(self):
        model = make_model(torch.ones((FEATURE_COUNT, 2)), torch.device('cpu'), 'tapered')
        loss, _ = validation_loss(model, self.future, np.array([100.]), 100, 2,
                                  torch.device('cpu'), indices=np.arange(4))
        logits = np.array([78, 77, 61, 46]) / 100
        target = (.75 / (1 + np.exp(-self.expected / 100)) + .25 * self.rows['result'] / 2).astype('f4')
        expected = np.mean(np.logaddexp(0, logits) - target * logits)
        self.assertAlmostEqual(loss, expected, places=7)
        epoch = train_epoch(model, torch.optim.SGD(model.parameters(), lr=0), self.future,
                            np.array([100.]), 100, 2, torch.Generator().manual_seed(1),
                            torch.device('cpu'), indices=np.arange(4), removal_penalty=0,
                            removal_reference=None)
        self.assertAlmostEqual(epoch.bce_loss, expected, places=7)

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
        with patch('lookahead_diag.estimate_generation_ks', side_effect=[(np.array([100.]), [4]), (np.array([200.]), [4])]):
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


if __name__ == '__main__':
    unittest.main()
