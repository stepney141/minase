"""教師値の混合と勝率尺度Kの推定を検証する。"""

from __future__ import annotations

import math
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import numpy as np
import torch

import minase_train.data.mnsd as mnsd
import minase_train.pst.teacher as teacher
from helpers import ProvenanceFixtures, write_mnsd, write_provenance, write_rescore
from minase_train.data.features import INITIAL_BOARD
from minase_train.data.mnsd import Dataset, RECORD_DTYPE, hash64
from minase_train.pst.model import model_logits
from minase_train.pst.teacher import build_targets, estimate_generation_ks, estimate_k


class LambdaOverrideTest(unittest.TestCase):

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / 'selfplay.bin'
        write_mnsd(self.source, seed=11, checksum=b'a' * 32,
                   games=np.repeat(np.arange(1, 81), 3).tolist(),
                   scores=[100, 200, -400] * 80, results=[2, 0, 2] * 80)

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
        with patch('minase_train.pst.teacher.estimate_k', return_value=321.) as estimate:
            ks, _ = estimate_generation_ks(dataset, indices=dataset.training_indices)
        np.testing.assert_array_equal(ks, [321.])
        np.testing.assert_array_equal(estimate.call_args.args[1],
                                      dataset.gather(dataset.training_indices)['result'])


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

    def test_teacher_k_classification_float_targets_and_gather(self):
        np.testing.assert_allclose(self.future.teacher_scores(np.array([3, 0, 2, 1])), self.expected[[3, 0, 2, 1]])
        np.testing.assert_array_equal(self.future.gather(np.arange(4))['score'], self.rows['score'])
        np.testing.assert_array_equal(self.current.training_indices, self.future.training_indices)
        self.assertNotEqual(self.current.teacher_classes, self.future.teacher_classes)
        self.assertEqual(self.future.class_metadata()[0]['lookahead_gamma'], .9)
        self.assertEqual(self.future.class_metadata()[0]['lookahead_plies'], 3)
        with patch('minase_train.pst.teacher.estimate_k', wraps=estimate_k) as estimate:
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


class Phase4Test(ProvenanceFixtures, unittest.TestCase):

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / "source.bin"
        self.sidecar = self.root / "replacement.any"
        self.rows = np.zeros(100, dtype=RECORD_DTYPE)
        self.rows["board"] = INITIAL_BOARD
        self.rows["lion"] = 255
        self.rows["game"] = np.arange(1, 101)
        self.rows["score"] = [-500, 500] * 50
        self.rows["result"] = [0, 2] * 50
        mnsd.write_mnsd(self.source, self.rows, seed=11, network_checksum=b"a" * 32, teacher_nodes=100000)
        self.provenance = write_provenance(self.source)



    def test_independent_k_with_same_checksum_and_training_only(self):
        rows = [(1, 0, 700, 1, 20) if i % 2 else (0, 0, 0, 0, 0) for i in range(100)]
        write_rescore(self.sidecar, self.source, rows)
        dataset = Dataset([self.source], rescore=[self.sidecar])
        calls = []
        def fit(scores, results):
            calls.append((scores.copy(), results.copy()))
            return [100., 200.][len(calls)-1]
        with patch('minase_train.pst.teacher.estimate_k', side_effect=fit):
            ks, counts = estimate_generation_ks(dataset, indices=dataset.training_indices)
        np.testing.assert_array_equal(ks, [100, 200])
        self.assertEqual(sum(counts), len(dataset.training_indices))
        for c, (scores, results) in enumerate(calls):
            records = dataset.gather(dataset.generation_training_indices(c))
            np.testing.assert_array_equal(scores, records["score"])
            np.testing.assert_array_equal(results, records["result"])
        np.testing.assert_array_equal(calls[1][0], 700)

    def test_zero_lambda_never_estimates_or_uses_teacher_k(self):
        self.metadata(result_origin="human", start_origin="human-game", games=self.human_games(), **{"lambda": 0})
        dataset = Dataset([self.source])
        with patch('minase_train.pst.teacher.estimate_k', side_effect=AssertionError("K must not be estimated")):
            ks, _ = estimate_generation_ks(dataset, indices=dataset.training_indices)
        self.assertTrue(np.isnan(ks[0]))
        records = dataset.gather(dataset.training_indices)
        np.testing.assert_array_equal(build_targets(records, ks, dataset.generations(dataset.training_indices), dataset.teacher_lambdas), records["result"] / 2)
        records["score"] = -32768
        np.testing.assert_array_equal(build_targets(records, ks, np.zeros(len(records), dtype=int), dataset.teacher_lambdas), records["result"] / 2)

    def test_class_specific_mixing(self):
        records = self.rows[:3].copy()
        records["score"] = 400
        records["result"] = [2, 0, 1]
        actual = build_targets(records, np.array([np.nan, 400., 800.]), np.arange(3), np.array([0., 0.75, 1.]))
        np.testing.assert_allclose(actual, [1., 0.75 / (1 + np.exp(-1)), 1 / (1 + np.exp(-0.5))], rtol=1e-6)


class TeacherScaleTest(unittest.TestCase):

    """世代別教師Kとモデル出力Kの責務分離を検証する。"""

    def test_generations_have_independent_teacher_scales(self) -> None:
        games = list(range(40))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first.bin"
            second = root / "second.bin"
            write_mnsd(
                first,
                seed=101,
                checksum=b"a" * 32,
                games=games,
                scores=[-1000, 1000] * 20,
                results=[0, 2] * 20,
            )
            write_mnsd(
                second,
                seed=202,
                checksum=b"b" * 32,
                games=games,
                scores=[-1000, 1000] * 20,
                results=[2, 0] * 20,
            )
            dataset = Dataset([first, second])
            self.assertEqual(dataset.file_generations.tolist(), [0, 1])

            generation_ks, generation_counts = estimate_generation_ks(dataset, indices=dataset.training_indices)
            expected_ks = []
            expected_counts = []
            for generation in range(dataset.generation_count):
                records = dataset.gather(
                    dataset.generation_training_indices(generation)
                )
                expected_ks.append(estimate_k(records["score"], records["result"]))
                expected_counts.append(records.size)
            np.testing.assert_allclose(generation_ks, expected_ks)
            self.assertEqual(generation_counts, expected_counts)
            self.assertNotAlmostEqual(generation_ks[0], generation_ks[1])

            records = np.zeros(2, dtype=RECORD_DTYPE)
            records["score"] = 1000
            records["result"] = 1
            targets = build_targets(
                records,
                np.array(generation_ks, dtype=np.float64),
                np.array([0, 1], dtype=np.int64),
                np.ones(2),
            )
            expected = 1.0 / (
                1.0 + np.exp(-1000.0 / np.array(generation_ks))
            )
            np.testing.assert_allclose(targets, expected, rtol=1e-6)

            single = torch.nn.Embedding(2, 1)
            with torch.no_grad():
                single.weight[:, 0] = torch.tensor([30.0, 10.0])
            phi = torch.tensor([0.25])
            logits = model_logits(single, torch.tensor([[0, 1]]), phi, 200.0)
            self.assertAlmostEqual(float(logits.item()), 0.2)
            tapered = torch.nn.Embedding(2, 2)
            with torch.no_grad():
                tapered.weight.copy_(torch.tensor([[30.0, 100.0], [10.0, 300.0]]))
            # φ=0.25: 0.25×40 + 0.75×400 = 310 → 310/200。
            logits = model_logits(tapered, torch.tensor([[0, 1]]), phi, 200.0)
            self.assertAlmostEqual(float(logits.item()), 1.55)


class RemovalApiTest(unittest.TestCase):

    def test_teacher_scales_use_only_explicit_fit_indices(self):
        with tempfile.TemporaryDirectory() as directory:
            a = Path(directory) / "a.bin"
            b = Path(directory) / "b.bin"
            scores = [100, -200, 300, -400, 1000, -1000, 2000, -2000]
            results = [0, 2, 2, 0, 2, 0, 2, 0]
            changed = scores[:4] + [-1000, 1000, -2000, 2000]
            write_mnsd(a, seed=5, checksum=b"a" * 32, games=list(range(8)), scores=scores, results=results)
            write_mnsd(b, seed=5, checksum=b"a" * 32, games=list(range(8)), scores=changed, results=results)
            first, second = Dataset([a]), Dataset([b])
            fit = np.array([0, 1, 2, 3], dtype=np.int64)
            ka, counts = teacher.estimate_generation_ks(first, indices=fit)
            kb, _ = teacher.estimate_generation_ks(second, indices=fit)
            np.testing.assert_array_equal(ka, kb)
            np.testing.assert_array_equal(counts, [4])
            self.assertEqual(teacher.estimate_mixed_k(first, indices=fit), teacher.estimate_mixed_k(second, indices=fit))
            self.assertNotEqual(teacher.estimate_mixed_k(first, indices=np.arange(8)), teacher.estimate_mixed_k(second, indices=np.arange(8)))


if __name__ == "__main__":
    unittest.main()
