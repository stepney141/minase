"""フェーズ1の式・除外条件・USI契約を手計算と合成入力で検査する。"""

import io
from contextlib import redirect_stdout
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

import leaf_sample as sample
from minase_train.data.features import FEATURE_COUNT, feature_indices
from minase_train.data.mnsd import RECORD_DTYPE, write_mnsd
from minase_train.pst.model import make_model, model_logits
from minase_train.data.taper import phase_ratios


def records(count):
    rows = np.zeros(count, dtype=RECORD_DTYPE)
    rows['lion'] = 255
    rows['game'] = 1
    rows['ply'] = np.arange(count)
    rows['result'] = 1
    return rows


def leaf(kind='static', k=0, value=0, leaf_eval=0, nodes=1):
    return dict(kind=kind, k=k, moves=['1a1b'] * k, value=value, leaf_eval=leaf_eval, nodes=nodes)


class CalculationTest(unittest.TestCase):
    def test_even_and_odd_sign_for_features_values_and_gradients(self):
        # log(3)のロジットは予測3/4。教師1/4なら勾配係数は1/2。
        value = np.log(3)
        result = sample.regression_changes(
            np.array([[1., 0.], [1., 0.]]), np.array([[0., 2.], [0., 2.]]),
            np.array([value, value]), np.array([value, -value]),
            [2, 1], np.array([.25, .25]), scale=1)
        np.testing.assert_allclose(result['loss_difference'], [0, 0], atol=1e-15)
        np.testing.assert_allclose(result['static_gradient'], [[.5, 0], [.5, 0]])
        np.testing.assert_allclose(result['q_gradient'], [[0, 1], [0, -1]])
        np.testing.assert_allclose(result['gradient_ratio'], [np.sqrt(5), np.sqrt(5)])
        np.testing.assert_array_equal(result['feature_changed'], [True, True])

    def test_k_zero_loss_and_gradient_are_identical(self):
        phi = np.array([[1., 2.], [0., 1.]])
        result = sample.regression_changes(phi, phi, [30, -70], [30, -70], [0, 0], np.array([.8, .2]))
        np.testing.assert_array_equal(result['loss_difference'], [0, 0])
        np.testing.assert_array_equal(result['gradient_ratio'], [0, 0])
        np.testing.assert_array_equal(result['gradient_changed'], [False, False])
        np.testing.assert_array_equal(result['q_gradient'], result['static_gradient'])

    def test_equal_value_different_feature_changes_gradient(self):
        # 値0なら予測1/2。教師3/4、K=2なので各有効特徴の勾配は-1/8。
        result = sample.regression_changes(np.array([[1., 0.]]), np.array([[0., 1.]]),
                                            [0], [0], [2], np.array([.75]), scale=2)
        np.testing.assert_array_equal(result['loss_difference'], [0])
        np.testing.assert_array_equal(result['static_gradient'], [[-.125, 0]])
        np.testing.assert_array_equal(result['q_gradient'], [[0, -.125]])
        self.assertTrue(result['gradient_changed'][0])
        self.assertAlmostEqual(result['gradient_ratio'][0], np.sqrt(2))

    def test_nonzero_loss_difference_and_zero_gradient_denominator(self):
        result = sample.regression_changes(np.array([[1.]]), np.array([[1.]]),
                                            [0], [np.log(3)], [0], np.array([.5]), scale=1)
        self.assertAlmostEqual(result['loss_difference'][0], np.log(4) - .5 * np.log(3) - np.log(2))
        self.assertTrue(np.isnan(result['gradient_ratio'][0]))
        np.testing.assert_allclose(result['q_gradient'], [[.25]])

    def test_feature_vector_phase_mirror_and_side_to_move(self):
        rows = records(3)
        rows['board'][:, [0, 11, 24]] = 1  # 同じ鏡映特徴の歩2枚と、別の歩1枚。
        rows['stm'] = [0, 1, 0]
        rows['lion'][2] = 5
        phi = sample.feature_vectors(rows).reshape(3, -1, 2)
        # 未成歩の状態は29。各状態72特徴、後手視点は段が反転する。
        np.testing.assert_allclose(phi[0, 29 * 72], [2 / 90, 178 / 90])
        np.testing.assert_allclose(phi[0, 29 * 72 + 12], [1 / 90, 89 / 90])
        np.testing.assert_allclose(phi[1, (47 + 29) * 72 + 66], [2 / 90, 178 / 90])
        np.testing.assert_allclose(phi[2, 94 * 72 + 5], [1 / 90, 89 / 90])
        np.testing.assert_allclose(phi.sum(axis=(1, 2)), [3, 3, 4])

    def test_gradient_matches_training_model_at_zero_weights(self):
        # 47駒なので両端点の係数は1/2。整数値と学習時の連続値はともに0。
        rows = records(1)
        rows['board'][0, :47] = 1
        model = make_model(torch.zeros((FEATURE_COUNT, 2)), torch.device('cpu'), 'mirrored')
        logits = model_logits(model, torch.as_tensor(feature_indices(
            rows['board'], rows['stm'], rows['lion'])),
            torch.as_tensor(phase_ratios(rows['board']), dtype=torch.float32), 2)
        loss = torch.nn.functional.binary_cross_entropy_with_logits(logits, torch.tensor([.75]))
        loss.backward()
        phi = sample.feature_vectors(rows)
        changes = sample.regression_changes(phi, phi, [0], [0], [0], np.array([.75]), scale=2)
        np.testing.assert_array_equal(changes['static_gradient'][0], model.weight.grad[:-1].numpy().reshape(-1))

    def test_cost_arithmetic(self):
        result = sample.estimate_cost(['quiet', 'quiet', 'capture', 'promotion'],
                                       [100, 100, 300, 500], 10, 200)
        self.assertEqual(result['cpu_seconds_per_node'], .01)
        self.assertEqual(result['by_kind']['quiet']['cpu_seconds'], 2)
        self.assertEqual(result['by_kind']['capture']['cpu_seconds'], 3)
        self.assertEqual(result['by_kind']['quiet']['estimated_training_records'], 142.5)
        self.assertAlmostEqual(result['minutes']['Q'], 1.484375)
        self.assertAlmostEqual(result['minutes']['Qc'], 3.7109375)
        self.assertAlmostEqual(result['minutes']['combined'], 5.1953125)
        self.assertTrue(result['budget_pass'])
        self.assertFalse(sample.estimate_cost(['quiet'], [1], 10000, 200)['budget_pass'])

    def test_distribution_reports_empty_and_absolute_percentile(self):
        result = sample.distribution([-10, 0, 2])
        self.assertEqual(result['mean'], -8 / 3)
        self.assertEqual(result['median'], 0)
        self.assertAlmostEqual(result['abs_p95'], 9.2)
        self.assertEqual(result['nonzero_fraction'], 2 / 3)
        self.assertIsNone(sample.distribution([])['mean'])

    def test_sampling_is_seeded_unique_and_eligible(self):
        kinds = ['quiet', 'capture'] * 200 + ['promotion', 'quiet', 'capture']
        info = [leaf(k=1)] * 400 + [leaf(k=1), leaf(k=0), leaf('mate', k=1, leaf_eval=None)]
        first = sample.select_roots(kinds, info)
        np.testing.assert_array_equal(first, sample.select_roots(kinds, info))
        self.assertEqual(len(first), 256)
        self.assertEqual(len(set(first)), 256)
        self.assertTrue(np.all(first < 400))
        # シードの変更を検出する。具体的な抽出アルゴリズムには依存しない。
        with patch('leaf_sample.np.random.default_rng', wraps=np.random.default_rng) as rng:
            sample.select_roots(kinds, info)
            rng.assert_called_once_with(1)
        np.testing.assert_array_equal(sample.select_roots(
            ['quiet', 'capture', 'promotion', 'quiet'],
            [leaf(k=1), leaf('max_ply', k=2), leaf(k=1), leaf(k=0)]), [0, 1])
        self.assertEqual(len(sample.select_roots(['quiet'], [leaf()])), 0)

    def test_score_mate_conversion_including_negative_zero(self):
        for kind, text, expected in [('cp', '-42', -42), ('mate', '3', 29997),
                                      ('mate', '-3', -29997), ('mate', '0', 30000), ('mate', '-0', -30000)]:
            self.assertEqual(sample.score_value(kind, text), expected)
        with self.assertRaises(ValueError):
            sample.score_value('unknown', '2')

    def test_search_parse_and_bounds(self):
        result = sample.parse_search(['info depth 9 score mate -0 nodes 500 pv 1a1b', 'bestmove 1a1b'])
        self.assertEqual(result['value'], -30000)
        self.assertTrue(result['mate'])
        with self.assertRaises(ValueError):
            sample.parse_search(['info depth 9 score cp 10 lowerbound nodes 500 pv 1a1b', 'bestmove 1a1b'])

    def test_search_comparison_preserves_history_and_parity(self):
        rows = records(3)
        rows['ply'] = [1, 2, 3]
        histories = {1: ['1a1b', '2a2b', '3a3b']}
        info = [leaf(k=1), leaf(k=2), leaf(k=0)]
        values = [{'value': 10, 'mate': False}, {'value': 20, 'mate': False},
                  {'value': 29998, 'mate': True}, {'value': 30, 'mate': False}]
        with patch('leaf_sample.run_search', side_effect=values) as search:
            report = sample.independent_comparison(rows, ['quiet', 'capture', 'quiet'], info,
                                                    histories, Path('/unused'), 1)
        self.assertEqual([call.args[1] for call in search.call_args_list],
                         [['1a1b'], ['1a1b', '1a1b'], ['1a1b', '2a2b'], ['1a1b', '2a2b', '1a1b', '1a1b']])
        self.assertEqual(report['roots'][0]['difference_cp'], 30)
        self.assertEqual(report['roots'][1]['difference_cp'], 29968)
        self.assertEqual(report['by_kind']['all']['mate_roots'], 1)
        self.assertEqual(report['by_kind']['all']['difference_cp']['mean'], 14999)

    def test_empty_search_does_not_launch_engine(self):
        with patch('leaf_sample.run_search') as search:
            report = sample.independent_comparison(records(1), ['quiet'], [leaf()], {1: []}, Path('/unused'), 1)
        search.assert_not_called()
        self.assertFalse(report['comparable'])
        self.assertEqual(report['count'], 0)

    def test_new_process_per_search_and_fixed_usi_conditions(self):
        class Process:
            def __init__(self):
                self.stdin = io.StringIO()
                self.stdout = io.StringIO('option name USI_Hash type spin\noption name Threads type spin\nusiok\nreadyok\ninfo depth 1 score cp 12 nodes 10000000 pv 1a1b\nbestmove 1a1b\n')
                self.returncode = 0
            def __enter__(self):
                return self
            def __exit__(self, *args):
                return False
        processes = [Process(), Process()]
        with patch('leaf_sample.subprocess.Popen', side_effect=processes) as launch:
            for _ in range(2):
                self.assertEqual(sample.run_search(Path('/engine'), ['1a1b'])['value'], 12)
        self.assertEqual(launch.call_count, 2)
        self.assertEqual(launch.call_args.args[0], ['/engine', '--protocol', 'usi', '--rules', 'engine-default'])
        for process in processes:
            self.assertEqual(process.stdin.getvalue(), 'usi\nsetoption name USI_Hash value 64\nsetoption name Threads value 1\nisready\nusinewgame\nposition startpos moves 1a1b\ngo nodes 10000000\nquit\n')


class InputAndAnalysisTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.rows = records(4)
        self.rows['board'][:, 0] = 1
        self.rows['score'] = [100, -200, 300, -400]
        self.rows['result'] = [2, 0, 2, 0]
        self.kinds = np.array(['quiet', 'capture', 'promotion', 'quiet'])
        self.leaves = self.rows.copy()
        self.info = [leaf(), leaf(k=1), leaf('max_ply', k=2), leaf('mate', k=1, value=29999, leaf_eval=None)]
        self.leaves['stm'][1] = 1
        self.zero = np.zeros(FEATURE_COUNT, dtype='i2')
        self.paths = [self.root / name for name in ('sample.bin', 'sample.kinds', 'sample.history.jsonl', 'leaves.bin', 'leaves.jsonl')]
        self.write_inputs()

    def write_inputs(self):
        source, kinds, history, end, info = self.paths
        for path, rows in ((source, self.rows), (end, self.leaves)):
            write_mnsd(path, rows, seed=1, network_checksum=b'a' * 32)
        kinds.write_text('\n'.join(self.kinds) + '\n')
        history.write_text(json.dumps({'game': 1, 'moves': ['1a1b'] * 4}) + '\n')
        info.write_text(''.join(json.dumps(row) + '\n' for row in self.info))

    def test_load_aggregate_teacher_scale_and_mate_exclusion(self):
        rows, kinds, histories, leaves, info = sample.read_inputs(*self.paths, 200)
        with patch('leaf_sample.estimate_k', return_value=100) as estimate:
            report = sample.analyze(rows, kinds, leaves, info, self.zero, self.zero, 10, 200)
        # 静かな0手目は3手目の-400を反転した400。最後は元の-400へ退避。
        np.testing.assert_array_equal(estimate.call_args.args[0], [400, -400])
        np.testing.assert_array_equal(estimate.call_args.args[1], [2, 0])
        self.assertEqual(report['conditions']['teacher_k'], 100)
        self.assertEqual(report['by_kind']['quiet']['count'], 2)
        self.assertEqual(report['by_kind']['quiet']['regression_count'], 1)
        self.assertEqual(report['by_kind']['quiet']['leaf_kinds']['mate'], 1)
        self.assertEqual(report['by_kind']['quiet']['gradient_changed_fraction'], 0)
        self.assertEqual(report['by_kind']['capture']['gradient_changed_fraction'], 1)
        self.assertEqual(report['by_kind']['quiet']['output_difference_cp']['mean'], 14999.5)
        self.assertTrue(report['proceed_phase2'])
        report['independent_search'] = sample.independent_comparison(
            rows[:1], kinds[:1], info[:1], histories, Path('/unused'), 1)
        json.dumps(report, allow_nan=False)
        summary = sample.summary_markdown(report)
        for token in ('センチポーン', '分', '件', '根手番側', '末端の手番側', 'コサイン'):
            self.assertIn(token, summary)

    def test_untrainable_quiet_is_a_failed_phase2_condition(self):
        report = sample.analyze(self.rows, np.array(['capture'] * 4), self.leaves, self.info,
                                self.zero, self.zero, 10, 200)
        self.assertFalse(report['phase2_conditions']['trainable_quiet'])
        self.assertFalse(report['proceed_phase2'])
        self.assertIsNone(report['conditions']['teacher_k'])
        self.assertEqual(report['by_kind']['capture']['regression_count'], 0)
        json.dumps(report, allow_nan=False)

    def test_leaf_integer_mismatch_stops(self):
        self.info[1]['leaf_eval'] = 1
        self.info[1]['value'] = -1
        with self.assertRaisesRegex(AssertionError, 'Python leaf'):
            sample.analyze(self.rows, self.kinds, self.leaves, self.info, self.zero, self.zero, 10, 200)

    def test_return_value_mismatch_stops(self):
        self.info[1]['value'] = 1
        self.write_inputs()
        with self.assertRaisesRegex(AssertionError, 'leaf/value'):
            sample.read_inputs(*self.paths, 200)

    def test_count_history_and_metadata_mismatch_stop(self):
        self.paths[1].write_text('quiet\n')
        with self.assertRaises(ValueError):
            sample.read_inputs(*self.paths, 200)
        self.write_inputs()
        self.paths[2].write_text(json.dumps({'game': 1, 'moves': []}) + '\n')
        with self.assertRaises(ValueError):
            sample.read_inputs(*self.paths, 200)
        self.leaves['score'][0] = 1
        self.write_inputs()
        with self.assertRaises(ValueError):
            sample.read_inputs(*self.paths, 200)

    def test_cpu_log_checks_totals_and_rejects_duplicates(self):
        log = self.root / 'leaves.log'
        log.write_text('records: 4\nstatic: 2\nmax_ply: 1\nmate: 1\nnodes: 1000\ncpu_seconds: 2.5\nelapsed_seconds: 1.5\n')
        self.assertEqual(sample.read_cpu_log(log, 4, 1000), 2.5)
        with self.assertRaises(ValueError):
            sample.read_cpu_log(log, 4, 999)
        with log.open('a') as stream:
            stream.write('cpu_seconds: 3\n')
        with self.assertRaises(ValueError):
            sample.read_cpu_log(log, 4, 1000)

    def test_cli_writes_both_reports_and_three_hashes_without_search(self):
        # 同一の根・末端からなる入力では、どの重みでも差は0となる。
        self.leaves = self.rows.copy()
        mg, eg, _, _ = sample.read_mnpt(sample.WT / 'nets/pst.bin')
        values = sample.integer_evaluate(mg, eg, feature_indices(
            self.rows['board'], self.rows['stm'], self.rows['lion']),
            sample.phase_numerators(self.rows['board']))
        self.info = [leaf(value=int(v), leaf_eval=int(v)) for v in values]
        self.write_inputs()
        checksum = (sample.WT / 'nets/pst.bin').read_bytes()[48:80]
        for path in (self.paths[0], self.paths[3]):
            write_mnsd(path, self.rows, seed=1, network_checksum=checksum)
        log, engine = self.root / 'leaves.log', self.root / 'minase'
        log.write_text('records: 4\nnodes: 4\ncpu_seconds: 1\n')
        engine.write_bytes(b'no engine is launched for k=0')
        arguments = []
        for name, path in zip(('sample', 'kinds', 'history', 'leaves', 'info'), self.paths):
            arguments.extend(['--' + name, str(path)])
        arguments.extend(['--log', str(log), '--engine', str(engine), '--output-dir', str(self.root)])
        output = io.StringIO()
        with redirect_stdout(output), patch('leaf_sample.run_search') as search:
            sample.main(arguments)
        search.assert_not_called()
        report = json.loads((self.root / 'result.json').read_text())
        self.assertEqual(report['by_kind']['quiet']['loss_difference']['max'], 0)
        self.assertEqual(report['by_kind']['capture']['gradient_changed_fraction'], 0)
        self.assertEqual(report['independent_search']['count'], 0)
        for path in (self.root / 'result.json', self.root / 'summary.md', Path(sample.__file__)):
            self.assertIn(sample.sha256_file(path).hex(), output.getvalue())
        self.assertEqual(len(output.getvalue().splitlines()), 3)


if __name__ == '__main__':
    unittest.main()
