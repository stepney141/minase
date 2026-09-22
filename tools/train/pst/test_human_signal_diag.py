"""指示書の診断B/C契約を、合成ファイルと独立した統計の参照値で検証する。"""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

import human_signal_diag as diag
from features import FEATURE_COUNT
from mnsd import RECORD_DTYPE, hash64, write_mnsd
from train_pst import initial_piece_values, write_mnpt


class HumanSignalTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.pst = self.root / 'pst.bin'
        zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
        write_mnpt(self.pst, zeros, zeros, initial_piece_values(), 1000.)

    def fixture(self, name='human', origin='human-game', games=80, signal=True, repeats=8):
        # 各対局は同一の先手勝敗、交互の手番を持つ。8局面中6局面の特徴が
        # 勝敗を正しく予測する。無信号では各勝敗で発火率を同じにする。
        n = games * repeats
        records = np.zeros(n, dtype=RECORD_DTYPE)
        records['board'][:, :60] = 1
        records['lion'] = 255
        records['game'] = np.repeat(np.arange(1, games + 1), repeats)
        records['stm'] = np.tile(np.arange(repeats) % 2, games)
        records['ply'] = np.tile(np.arange(repeats), games)
        wins = (records['game'] % 2) != records['stm']
        records['result'] = wins * 2
        features = np.zeros((n, 118), dtype=np.uint8)
        local = np.tile(np.arange(repeats), games)
        features[:, 6] = np.where(local % 4 == 0, ~wins, wins) if signal else local % 4 < 2
        data = self.root / (name + '.bin')
        write_mnsd(data, records, seed=31, network_checksum=bytes(32))
        digest = hashlib.sha256(data.read_bytes()).digest()
        mapped = [{'game': g, 'id': f'棋譜-{g}', 'ply': 50} for g in range(1, games + 1)]
        provenance = {'format': 'minase-provenance', 'version': 1, 'mnsd_sha256': digest.hex(),
                      'teacher': {'generation_commit': '0' * 40, 'network_checksum': '00' * 32,
                                  'nodes': 0, 'rule_set': 'L0,P0,R1,E0', 'search_condition': 'in-game'},
                      'result_origin': 'human' if origin == 'human-game' else 'selfplay',
                      'start_origin': 'human-game' if origin == 'human-start-selfplay' else 'random',
                      'lambda': 0. if origin == 'human-game' else .75,
                      'games': None if origin == 'random-selfplay' else mapped}
        Path(str(data) + '.provenance.json').write_text(json.dumps(provenance))
        mnkf = self.root / (name + '.mnkf')
        mnkf.write_bytes(struct.pack('<4sIIIQ32s', b'MNKF', 1, 2, 118, n, digest) + features.tobytes())
        table = {row['id']: {'game': row['game'], 'sente_rating': 1600, 'gote_rating': 1600,
                            'speed': 'correspondence', 'clock': None} for row in mapped}
        table_path = self.root / (name + '.games.json')
        table_path.write_text(json.dumps(table))
        return data, mnkf, table_path

    def sample(self, signal=True, origin='human-game', name='human', games=80):
        data, features, table = self.fixture(name, origin, games=games, signal=signal)
        return diag.load_sample(data, features, self.pst, 1000., origin,
                                table if origin == 'human-game' else None)

    def test_sha_split_and_folds_are_reproducible_and_game_disjoint(self):
        sample = self.sample()
        for row in sample.assignments:
            expected = int.from_bytes(hashlib.sha256(row['id'].encode()).digest()[:8], 'little') % 2
            self.assertEqual(row['split'], 'validation' if expected == 0 else 'diagnostic')
        first = diag.cross_validation_folds(sample.games, 7)
        np.testing.assert_array_equal(first, diag.cross_validation_folds(sample.games, 7))
        for game in np.unique(sample.games):
            self.assertEqual(len(np.unique(first[sample.games == game])), 1)
        reordered = np.arange(len(first))[::-1]
        np.testing.assert_array_equal(first[reordered], diag.cross_validation_folds(sample.games[reordered], 7))
        random = self.sample(origin='random-selfplay', name='random')
        np.testing.assert_array_equal(random.validation, hash64(31, random.games) % 2 == 0)

    def test_exact_ten_columns_and_reject_wrong_mnkf(self):
        data, features, _ = self.fixture()
        raw = bytearray(features.read_bytes())
        raw[56:] = np.tile(np.arange(118, dtype=np.uint8), (640, 1)).tobytes()
        features.write_bytes(raw)
        selected = diag.read_features(features, hashlib.sha256(data.read_bytes()).digest(), 640)
        np.testing.assert_array_equal(selected[0], [6, 7, 8, 9, 10, 11, 26, 29, 34, 37])
        for offset in (8, 12, 16, 24):
            corrupt = raw.copy()
            corrupt[offset] ^= 1
            features.write_bytes(corrupt)
            with self.assertRaises(ValueError):
                diag.read_features(features, hashlib.sha256(data.read_bytes()).digest(), 640)
        features.write_bytes(raw[:-1])
        with self.assertRaises(ValueError):
            diag.read_features(features, hashlib.sha256(data.read_bytes()).digest(), 640)

    def test_signal_improves_and_balanced_null_stays_near_zero(self):
        for signal in (True, False):
            sample = self.sample(signal=signal, name=str(signal))
            result, _ = diag.diagnose_group(sample, 100, 1, np.random.default_rng(9))
            self.assertEqual(result['insufficient_reasons'], [])
            if signal:
                self.assertGreater(result['g']['estimate'], .10)
                self.assertGreater(result['g']['lower_95_one_sided'], 0)
                self.assertGreater(result['models']['candidate']['coefficients'][
                    'shelter.stm.one_slider.distance0'], 0)
            else:
                self.assertLess(abs(result['g']['estimate']), 1e-7)
            json.dumps(result, allow_nan=False)

    def test_validation_targets_do_not_change_fitted_coefficients_or_alpha(self):
        sample = self.sample()
        before, _ = diag.diagnose_group(sample, 20, 1, np.random.default_rng(1))
        sample.y[sample.validation] = 1 - sample.y[sample.validation]
        after, _ = diag.diagnose_group(sample, 20, 1, np.random.default_rng(1))
        self.assertEqual(before['models'], after['models'])
        self.assertLess(after['g']['estimate'], 0)

    def test_bootstrap_equal_game_weight_and_seed(self):
        means = diag.game_means(np.array([1., 3., 10.]), np.array([1, 1, 2]))
        np.testing.assert_array_equal(means, [2., 10.])
        first = diag.bootstrap_means(means, 2000, np.random.default_rng(13))
        second = diag.bootstrap_means(means, 2000, np.random.default_rng(13))
        np.testing.assert_array_equal(first, second)
        self.assertEqual(set(first), {2., 6., 10.})
        report = diag.interval(6., first)
        self.assertEqual(report['lower_95_one_sided'], 2.)
        self.assertAlmostEqual(report['standard_error'], np.sqrt(8), delta=.15)

    def test_all_insufficient_conditions_and_rejection(self):
        metric = {'estimate': .1, 'lower_95_one_sided': .02, 'upper_95_one_sided': .2,
                  'standard_error': .03, 'invalid_fraction': 0.}
        groups = {name: {'g': metric.copy(), 'counts': {'validation': {'games': 300}},
                         'insufficient_reasons': []} for name in ('human-start', 'random-start')}
        self.assertEqual(diag.decision(groups, metric)[0], '基準を満たす')
        for name in groups:
            altered = copy.deepcopy(groups)
            altered[name]['counts']['validation']['games'] = 299
            self.assertIn(f'{name}:fewer_than_300_validation_games', diag.decision(altered, metric)[1])
            altered = copy.deepcopy(groups)
            altered[name]['insufficient_reasons'] = ['candidate:regression_not_converged']
            self.assertEqual(diag.decision(altered, metric)[0], '判断材料不足')
            altered = copy.deepcopy(groups)
            altered[name]['g']['invalid_fraction'] = .05
            self.assertIn(f'{name}:bootstrap_invalid_at_least_5_percent', diag.decision(altered, metric)[1])
        for name in ('human-start', 'G'):
            altered = copy.deepcopy(groups)
            contrast = metric.copy()
            target = contrast if name == 'G' else altered[name]['g']
            target['lower_95_one_sided'] = -.01
            self.assertEqual(diag.decision(altered, contrast)[0], '判断材料不足')
        negative = dict(metric, lower_95_one_sided=-.2, upper_95_one_sided=-.01)
        self.assertEqual(diag.decision(groups, negative)[0], '基準を満たさない')
        invalid = diag.interval(np.nan, np.full(20, np.nan))
        self.assertEqual(diag.decision(groups, invalid)[0], '判断材料不足')

    def test_optimizer_failure_and_fixed_offset(self):
        x = np.zeros((4, 1))
        y = np.array([0., 0., 0., 1.])
        fit = diag.fit_logistic(x, np.zeros(4), y, 0, max_iterations=0)
        self.assertFalse(fit.converged)
        fit = diag.fit_logistic(x, np.full(4, 2.), y, 0)
        self.assertTrue(fit.converged)
        self.assertAlmostEqual(fit.coefficients[0], np.log(1 / 3) - 2, places=5)
        self.assertFalse(diag.fit_logistic(x, np.zeros(4), np.ones(4), 0).converged)
        short = self.sample(games=4)
        result, _ = diag.diagnose_group(short, 10, 1, np.random.default_rng(1))
        self.assertTrue(result['insufficient_reasons'])

    def test_failed_cv_is_reported_even_if_other_alphas_converge(self):
        sample = self.sample()
        original = diag.fit_logistic

        def fail_unregularized(x, offset, y, alpha):
            fit = original(x, offset, y, alpha)
            if alpha == 0:
                fit.converged = False
            return fit

        with patch.object(diag, 'fit_logistic', side_effect=fail_unregularized):
            report, _ = diag.diagnose_group(sample, 20, 1, np.random.default_rng(1))
        self.assertEqual(diag.decision({'human-games': report})[0], '判断材料不足')
        self.assertEqual(len(report['models']['candidate']['cv_failures']), 5)
        self.assertGreater(report['models']['candidate']['alpha'], 0)

    def test_pst_uses_output_k_taper_and_ignores_extra_weights(self):
        records = np.zeros(3, dtype=RECORD_DTYPE)
        records['lion'] = 255
        for row, count in zip(records, [2, 47, 92]):
            row['board'][:count] = 1
        # 全升mg=1cp、eg=3cp、φ=0,1/2,1より、評価値は6,94,92cp。
        mg = np.full(FEATURE_COUNT + 1, 8, dtype=np.int16)
        eg = np.full(FEATURE_COUNT + 1, 24, dtype=np.int16)
        mg[-1] = eg[-1] = 30000
        write_mnpt(self.pst, mg, eg, initial_piece_values(), 1000., feature_count=len(mg))
        np.testing.assert_allclose(diag.pst_logits(records, self.pst, 10.), [.6, 9.4, 9.2], rtol=1e-6)

    def test_empty_validation_reports_insufficient_without_nonfinite_json(self):
        sample = self.sample()
        sample.validation[:] = False
        report, _ = diag.diagnose_group(sample, 20, 1, np.random.default_rng(1))
        verdict, reasons = diag.decision({'human-games': report})
        self.assertEqual(verdict, '判断材料不足')
        self.assertIn('human-games:bootstrap_invalid_at_least_5_percent', reasons)
        json.dumps(report, allow_nan=False)

    def test_piece_clock_boundaries_ratings_and_draw_midgame_exclusions(self):
        data, features, table_path = self.fixture(games=1, repeats=8)
        records = np.zeros(8, dtype=RECORD_DTYPE)
        for row, count in zip(records, [78, 77, 62, 61, 47, 46, 92, 1]):
            row['board'][:count] = 1
        records['game'] = np.arange(1, 9)
        records['stm'] = np.arange(8) % 2
        table = {str(i + 1): {'game': i + 1, 'sente_rating': 1800, 'gote_rating': 1600,
                             'speed': 'correspondence' if i == 7 else 'rapid',
                             'clock': None if value is None else {'initial': value}}
                 for i, value in enumerate([599, 600, 1799, 1800, 0, 3600, None, None])}
        x, names = diag.covariates(records, {i: str(i) for i in range(1, 9)}, table)
        np.testing.assert_array_equal(x[:, 1:4], [[0,0,0], [1,0,0], [1,0,0], [0,1,0],
                                                [0,1,0], [0,0,1], [0,0,0], [0,0,1]])
        np.testing.assert_array_equal(x[:, names.index('rating_difference')], [200,-200]*4)
        np.testing.assert_array_equal(x[:, -3:], [[0,0,0], [1,0,0], [1,0,0], [0,1,0],
                                                [0,0,0], [0,1,0], [0,0,1], [0,0,1]])
        data, features, _ = self.fixture('start', 'human-start-selfplay', games=4)
        # ヘッダと来歴とMNKFの検査和を更新して、境界の盤面を作る。
        from mnsd import map_records
        records = np.array(map_records(data))
        records['board'][records['game'] == 1, 46:] = 0
        records['board'][records['game'] == 2, 47:] = 0
        records['result'][records['game'] == 3] = 1
        write_mnsd(data, records, seed=31, network_checksum=bytes(32))
        digest = hashlib.sha256(data.read_bytes()).digest()
        provenance_path = Path(str(data) + '.provenance.json')
        provenance = json.loads(provenance_path.read_text())
        provenance['mnsd_sha256'] = digest.hex()
        provenance_path.write_text(json.dumps(provenance))
        raw = bytearray(features.read_bytes())
        raw[24:56] = digest
        features.write_bytes(raw)
        sample = diag.load_sample(data, features, self.pst, 1000, 'human-start-selfplay')
        self.assertEqual(set(sample.games), {2, 4})
        self.assertEqual(sample.exclusions['draw_positions'], 8)
        self.assertEqual(sample.exclusions['non_midgame_positions'], 8)
        self.assertEqual(sample.exclusions['games_without_midgame_records'], 1)

    def test_cli_and_independent_contrast_bootstrap(self):
        data, features, table = self.fixture()
        output = self.root / 'out.json'
        diag.main(['human-games', '--data', str(data), '--king-features', str(features),
                   '--games-table', str(table), '--pst', str(self.pst), '--output-k', '1000',
                   '--output', str(output), '--bootstrap', '20'])
        self.assertEqual(json.loads(output.read_text())['decision'], '基準を満たす')
        human, hf, _ = self.fixture('h', 'human-start-selfplay', games=640)
        random, rf, _ = self.fixture('r', 'random-selfplay', games=640, signal=False)
        args = argparse.Namespace(command='human-start', human_start=human, random_start=random,
                                  human_start_features=hf, random_start_features=rf,
                                  pst=self.pst, output_k=1000, bootstrap=100, seed=3)
        report = diag.run(args)
        self.assertEqual(report['decision'], '基準を満たす')
        self.assertGreater(report['G']['lower_95_one_sided'], 0)
        self.assertAlmostEqual(report['G']['estimate'], report['groups']['human-start']['g']['estimate'] -
                               report['groups']['random-start']['g']['estimate'])
        self.assertEqual(report, diag.run(args))

    def test_contrast_variance_is_sum_for_independent_group_resamples(self):
        human = self.sample(name='h', origin='human-start-selfplay', games=640)
        random = self.sample(name='r', origin='random-selfplay', games=640)
        # 同じ系列で再標本化すると差の分散が打ち消される対照を作る。
        random.validation = human.validation.copy()
        for sample in (human, random):
            sample.extra[sample.games % 3 == 0] = 0
        args = argparse.Namespace(command='human-start', human_start=Path('h'), random_start=Path('r'),
                                  human_start_features=Path('hf'), random_start_features=Path('rf'),
                                  pst=self.pst, output_k=1000, bootstrap=2000, seed=3)
        with patch.object(diag, 'load_sample', side_effect=[human, random]):
            report = diag.run(args)
        groups = report['groups']
        variance_sum = sum(g['g']['standard_error'] ** 2 for g in groups.values())
        self.assertGreater(variance_sum, 0)
        self.assertAlmostEqual(report['G']['standard_error'] ** 2 / variance_sum, 1., delta=.12)


if __name__ == '__main__':
    unittest.main()
