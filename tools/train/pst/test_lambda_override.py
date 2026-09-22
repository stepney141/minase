"""混合比の設計書に基づき、適用範囲、教師値、校正、共通診断を検証する。"""

from contextlib import redirect_stdout, redirect_stderr
import hashlib
import io
import json
import math
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

from features import FEATURE_COUNT
from mnsd import Dataset, provenance_path
from pst_diagnostics import Weights, outcome_metrics
from test_phase4 import write_rescore
from test_train_pst import write_mnsd, write_provenance
from train_pst import build_targets, estimate_generation_ks, initial_piece_values, write_mnpt


class LambdaOverrideTest(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / 'selfplay.bin'
        write_mnsd(self.source, seed=11, checksum=b'a' * 32,
                   games=np.repeat(np.arange(1, 81), 3).tolist(),
                   scores=[100, 200, -400] * 80, results=[2, 0, 2] * 80)

    def test_only_selfplay_classes_change_and_provenance_is_unchanged(self):
        human = self.root / 'human.bin'
        started = self.root / 'human-start.bin'
        for path, origin, seed in ((human, 'human', 12), (started, 'selfplay', 13)):
            write_mnsd(path, seed=seed, checksum=b'b' * 32,
                       games=[1, 2], scores=[300, 400], results=[0, 2])
            write_provenance(path, result_origin=origin, start_origin='human-game', **{
                'lambda': 0 if origin == 'human' else .25,
                'games': [{'game': 1, 'id': 'first', 'ply': 0},
                          {'game': 2, 'id': 'second', 'ply': 0}]})
        paths = [self.source, human, started]
        before = [provenance_path(path).read_bytes() for path in paths]
        for override in (0, .4, 1):
            dataset = Dataset(paths, lambda_override=override)
            np.testing.assert_array_equal(dataset.teacher_lambdas, [override, 0, override])
            self.assertEqual([c.lambda_override for c in dataset.teacher_classes],
                             [override, None, override])
            rows = dataset.gather(np.array([240, 241]))
            targets = build_targets(rows, np.full(3, 100.), dataset.generations(np.array([240, 241])),
                                    dataset.teacher_lambdas)
            np.testing.assert_array_equal(targets, [0, 1])
        self.assertEqual(before, [provenance_path(path).read_bytes() for path in paths])

    def test_omission_explicit_zero_and_same_value_have_distinct_classes(self):
        original = Dataset([self.source])
        zero = Dataset([self.source], lambda_override=0)
        same = Dataset([self.source], lambda_override=.75)
        self.assertIsNone(original.lambda_override)
        self.assertIsNone(original.class_metadata()[0]['lambda_override'])
        self.assertEqual(original.class_metadata()[0]['lambda'], .75)
        self.assertEqual(zero.class_metadata()[0]['lambda_override'], 0)
        self.assertEqual(zero.class_metadata()[0]['lambda'], 0)
        self.assertNotEqual(original.teacher_classes, zero.teacher_classes)
        self.assertNotEqual(original.teacher_classes, same.teacher_classes)
        ks, _ = estimate_generation_ks(zero, indices=zero.training_indices)
        self.assertTrue(np.isnan(ks[0]))
        rows = zero.gather(zero.training_indices)
        np.testing.assert_array_equal(build_targets(rows, ks, zero.generations(zero.training_indices),
                                                   zero.teacher_lambdas), rows['result'] / 2)

    def test_k_uses_outcomes_at_one_and_targets_use_effective_lambda_with_lookahead(self):
        for lookahead, scores in ((None, [100, 200, -400]),
                                 ({'gamma': .9, 'plies': 3}, [-560 / 1.9, 400, -400])):
            reference = Dataset([self.source], lookahead=lookahead)
            original_ks, _ = estimate_generation_ks(reference, indices=reference.training_indices)
            for mix in (.4, 1):
                dataset = Dataset([self.source], lookahead=lookahead, lambda_override=mix)
                ks, _ = estimate_generation_ks(dataset, indices=dataset.training_indices)
                # 校正は混合式とは独立なので、正のλ同士では同じKになる。
                np.testing.assert_array_equal(ks, original_ks)
                rows = dataset.gather(np.arange(3))
                targets = build_targets(rows, [100], dataset.generations(np.arange(3)),
                                        dataset.teacher_lambdas, scores=dataset.teacher_scores(np.arange(3)))
                expected = [mix / (1 + math.exp(-v / 100)) + (1 - mix) * r
                            for v, r in zip(scores, [1, 0, 1])]
                np.testing.assert_allclose(targets, expected, rtol=1e-7)
                self.assertEqual(dataset.class_metadata()[0]['lambda_override'], mix)
                self.assertEqual(dataset.class_metadata()[0]['lookahead_gamma'],
                                 None if lookahead is None else .9)
        # 来歴がλ=0でも上書き後のλ=1ならKを推定する。
        write_provenance(self.source, **{'lambda': 0})
        dataset = Dataset([self.source], lambda_override=1)
        with patch('train_pst.estimate_k', return_value=321.) as estimate:
            ks, _ = estimate_generation_ks(dataset, indices=dataset.training_indices)
        np.testing.assert_array_equal(ks, [321.])
        np.testing.assert_array_equal(estimate.call_args.args[1],
                                      dataset.gather(dataset.training_indices)['result'])

    def test_override_applies_to_rescored_selfplay_classes(self):
        sidecar = self.root / 'rescore.bin'
        write_rescore(sidecar, self.source, [(1, 0, 120, 1, 100), (0, 0, 0, 0, 0)] * 120)
        dataset = Dataset([self.source], rescore=[sidecar], lambda_override=1)
        self.assertEqual(dataset.generation_count, 2)
        np.testing.assert_array_equal(dataset.teacher_lambdas, [1, 1])
        self.assertEqual([c.lambda_override for c in dataset.teacher_classes], [1, 1])

    def test_cli_options_reach_dataset_and_reject_invalid_values(self):
        import pst_diagnostics
        from train_pst import build_parser, main
        parser = build_parser()
        for mix in (None, '0', '1'):
            option = [] if mix is None else ['--lambda-override', mix]
            arguments = ['estimate-k', '--data', str(self.source), *option]
            self.assertEqual(parser.parse_args(arguments).lambda_override,
                             None if mix is None else float(mix))
            with redirect_stdout(io.StringIO()):
                main(arguments)
            output = self.root / f'diagnostic-{mix}'
            argv = ['pst_diagnostics.py', '--data', str(self.source), '--base', 'base.bin',
                    '--candidate', 'candidate.bin', '--probe', 'probe',
                    '--output-dir', str(output), *option]
            # 外部探査の手前で、CLIから構築されたデータセットを確認する。
            with patch('sys.argv', argv), patch.object(pst_diagnostics, 'diagnose', return_value={}) as diagnose:
                pst_diagnostics.main()
            dataset = diagnose.call_args.args[0]
            self.assertEqual(dataset.lambda_override, None if mix is None else float(mix))
            self.assertEqual(dataset.teacher_lambdas[0], .75 if mix is None else float(mix))
        for value in ('-0.01', '1.01', 'nan', 'inf', '-inf'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                main(['estimate-k', '--data', str(self.source), '--lambda-override=' + value])
            argv = ['pst_diagnostics.py', '--data', str(self.source), '--base', 'base.bin',
                    '--candidate', 'candidate.bin', '--probe', 'probe',
                    '--output-dir', str(self.root / 'invalid'), '--lambda-override=' + value]
            with patch('sys.argv', argv), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                pst_diagnostics.main()
            self.assertNotEqual(error.exception.code, 0)
            self.assertFalse((self.root / 'invalid').exists())

    def test_rejects_out_of_range_nonfinite_and_nonnumeric_values(self):
        for value in (-.01, 1.01, math.nan, math.inf, -math.inf, True, False, '0'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                Dataset([self.source], lambda_override=value)


class OutcomeMetricsTest(unittest.TestCase):
    def test_hand_computed_means_signs_and_cross_file_game_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            ids = [str(i) for i in range(200)
                   if int.from_bytes(hashlib.sha256(str(i).encode()).digest()[:8], 'little') % 20 == 0]
            train_id = next(str(i) for i in range(200)
                            if int.from_bytes(hashlib.sha256(str(i).encode()).digest()[:8], 'little') % 20 != 0)
            paths = []
            for file, (squares, results, games) in enumerate((
                    ([0, 1, 4, 0], [2, 0, 2, 0], [1, 1, 2, 3]),
                    ([2, 3, 5], [0, 1, 1], [1, 1, 2]))):
                boards = np.zeros((len(squares), 144), dtype='u1')
                boards[np.arange(len(squares)), squares] = 1
                path = root / f'{file}.bin'
                write_mnsd(path, seed=file, checksum=b'a' * 32, games=games,
                           results=results, board=boards)
                write_provenance(path, result_origin='human', **{'lambda': 0, 'games': [
                    {'game': 1, 'id': ids[0]}, {'game': 2, 'id': ids[1]},
                    {'game': 3, 'id': train_id}]})
                paths.append(path)
            weights = np.zeros(FEATURE_COUNT, dtype='i2')
            # 未成の歩は駒状態29。1枚で±100cpまたは0cpにする。
            weights[29 * 144:29 * 144 + 6] = [800, -800, 800, 0, 0, -800]
            mnpt = root / 'pst.bin'
            write_mnpt(mnpt, weights, weights, initial_piece_values(), 100)
            dataset = Dataset(paths, lambda_override=1)
            # バッチ境界とファイル境界を越えて同じ対局にまとめる。
            with patch('pst_diagnostics.BATCH', 2):
                report = outcome_metrics(dataset, {'base': Weights(mnpt)})['base']
            a = math.log(1 + math.exp(-1))
            b = math.log(1 + math.exp(1))
            c = (a + b) / 2
            self.assertAlmostEqual(report['bce_position_mean'], (2*a+b+2*math.log(2)+c)/6)
            self.assertAlmostEqual(report['bce_game_mean'], ((2*a+b+math.log(2))/4 + (math.log(2)+c)/2)/2)
            self.assertEqual(report['records'], 6)
            self.assertEqual(report['games'], 2)
            self.assertEqual(report['sign_agreement'], 2/3)
            self.assertEqual(report['sign_matches'], 2)
            self.assertEqual(report['sign_records'], 3)
            self.assertEqual(report['sign_excluded'], 3)
            self.assertEqual(report['draw_records'], 2)
            self.assertEqual(report['zero_score_records'], 2)
            self.assertEqual(report['output_k'], 100)
            json.dumps(report, allow_nan=False)
            unchanged = outcome_metrics(Dataset(paths), {'base': Weights(mnpt)})['base']
            for key in report:
                if isinstance(report[key], float):
                    self.assertAlmostEqual(report[key], unchanged[key])
                else:
                    self.assertEqual(report[key], unchanged[key])
            weights[:] = 0
            write_mnpt(root / 'zero.bin', weights, weights, initial_piece_values(), 100)
            empty = outcome_metrics(dataset, {'zero': Weights(root / 'zero.bin')})['zero']
            self.assertIsNone(empty['sign_agreement'])
            self.assertEqual(empty['sign_excluded'], 6)
            self.assertAlmostEqual(empty['bce_position_mean'], math.log(2))


if __name__ == '__main__':
    unittest.main()
