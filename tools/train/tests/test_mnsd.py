"""教師データの形式、来歴、分割、および追加特徴の対応を検証する。"""

from __future__ import annotations

import hashlib
import json
import math
import struct
import tempfile
import unittest
from pathlib import Path

import numpy as np
import torch

import minase_train.data.mnsd as mnsd
from helpers import ProvenanceFixtures, write_mnsd, write_provenance, write_rescore
from minase_train.data.features import FEATURE_COUNT, INITIAL_BOARD
from minase_train.data.mnsd import (
    Dataset,
    HEADER,
    KingFeatures,
    RECORD_DTYPE,
    hash64,
    provenance_path,
)
from minase_train.pst.model import make_model
from minase_train.pst.teacher import build_targets, estimate_generation_ks
from minase_train.pst.train import validation_loss


class KingFeatureSelectionTests(unittest.TestCase):

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.data = self.root / 'data.bin'
        self.mnkf = self.root / 'features.bin'
        self.records = np.zeros(200, dtype=RECORD_DTYPE)
        self.records['board'][:, :47] = 1
        self.records['board'][:, :2] = [12, 76]
        self.records['board'][::2, 2] = 51
        self.records['lion'] = 255
        self.records['game'] = np.arange(200)
        self.records['score'] = 1000
        self.records['result'] = 2
        mnsd.write_mnsd(self.data, self.records, seed=0, network_checksum=bytes(32))
        write_provenance(self.data)
        self.values = np.tile(np.arange(68, dtype=np.uint8), (200, 1)) % 3 + 1
        self.write_features()

    def write_features(self):
        self.mnkf.write_bytes(HEADER.pack(
            b'MNKF', 1, 1, self.values.shape[1], len(self.records), hashlib.sha256(self.data.read_bytes()).digest()
        ) + self.values.tobytes())

    def test_mixed_definitions_and_widths_are_rejected(self):
        # 入力ファイルの定義と列数は一致しなければならない。
        other = self.root / 'shelter.bin'
        other_features = self.root / 'shelter.mnkf'
        mnsd.write_mnsd(other, self.records[:2], seed=5, network_checksum=b'a' * 32)
        write_provenance(other)
        for definition, width in ((1, 24), (2, 68), (2, 118)):
            rows = np.full((2, width), 7, dtype=np.uint8)
            other_features.write_bytes(HEADER.pack(
                b'MNKF', 1, definition, width, 2, hashlib.sha256(other.read_bytes()).digest()
            ) + rows.tobytes())
            with self.subTest(definition=definition, width=width), self.assertRaises(ValueError):
                KingFeatures(Dataset([self.data, other]), [self.mnkf, other_features])


class KingFeaturesTests(unittest.TestCase):

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.data = self.directory / "data.bin"
        self.features = self.directory / "features.bin"
        games = np.arange(1, 200, dtype=np.uint32)
        validation = hash64(17, games) % np.uint64(20) == 0
        self.records = np.zeros(4, dtype=RECORD_DTYPE)
        self.records["lion"] = 255
        self.records["game"][:3] = games[~validation][:3]
        self.records["game"][3] = games[validation][0]
        self.records["result"] = [0, 1, 2, 2]
        # 補間仕様φ=(N−2)/90から、訓練の3局面はφ=0, 1/2, 1。
        for row, count in enumerate([2, 47, 92, 92]):
            self.records["board"][row, :count] = 1
            self.records["board"][row, :2] = [12, 76]  # 先手王と後手玉。
        self.records["board"][1, 2] = 51  # 先手の太子。
        self.records["board"][2, 2:4] = [51, 115]  # 双方の太子。
        self.records["stm"][2] = 1
        mnsd.write_mnsd(self.data, self.records, seed=17, network_checksum=bytes(32))
        write_provenance(self.data)
        self.values = np.zeros((4, 68), dtype=np.uint8)
        self.values[:, 0] = [0, 1, 2, 255]
        self.values[:, 1] = [2, 1, 0, 255]
        self.values[:, 2] = 1
        self.write_features()
        self.dataset = Dataset([self.data])

    def write_features(self):
        self.features.write_bytes(HEADER.pack(
            b"MNKF", 1, 1, self.values.shape[1], len(self.values), hashlib.sha256(self.data.read_bytes()).digest(),
        ) + self.values.tobytes())

    def test_reads_all_columns_from_header(self):
        for definition, width in ((1, 24), (1, 68), (2, 118)):
            values = np.tile(np.arange(width, dtype=np.uint8), (4, 1))
            self.features.write_bytes(HEADER.pack(
                b"MNKF", 1, definition, width, 4, hashlib.sha256(self.data.read_bytes()).digest()
            ) + values.tobytes())
            with self.subTest(definition=definition, width=width):
                mapped = KingFeatures(self.dataset, [self.features])
                np.testing.assert_array_equal(mapped.gather(np.array([3, 0])), values[[3, 0]])
                self.assertEqual(mapped.definition_id, definition)
                self.assertEqual(mapped.column_count, width)

    def test_rejects_input_sha256_mismatch(self):
        # 交換形式の契約: 入力全体に結び付くので、盤面以外の教師値の変更も拒否する。
        changed = self.records.copy()
        changed["score"][0] = 1
        mnsd.write_mnsd(self.data, changed, seed=17, network_checksum=bytes(32))
        write_provenance(self.data)
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            KingFeatures(Dataset([self.data]), [self.features])

    def test_rejects_each_header_mismatch_and_body_length(self):
        # magic・版・列数・局面数を、それ以外を維持したまま1つずつ変える。
        original = self.features.read_bytes()
        for offset, fmt, value in [(0, "4s", b"BAD!"), (4, "I", 2),
                                   (12, "I", 67), (16, "Q", 3)]:
            with self.subTest(offset=offset):
                changed = bytearray(original)
                struct.pack_into("<" + fmt, changed, offset, value)
                self.features.write_bytes(changed)
                with self.assertRaises(ValueError):
                    KingFeatures(self.dataset, [self.features])
        for changed in (original[:20], original[:-1], original + b"\0"):
            self.features.write_bytes(changed)
            with self.assertRaises(ValueError):
                KingFeatures(self.dataset, [self.features])

    def test_global_indices_preserve_order_across_files(self):
        # Datasetと同じ通し番号。ファイル境界を往復し、重複した番号も保存する。
        second_data = self.directory / "second.bin"
        second_features = self.directory / "second-features.bin"
        mnsd.write_mnsd(second_data, self.records[:2], seed=18, network_checksum=bytes(32))
        write_provenance(second_data)
        rows = np.full((2, 68), 7, dtype=np.uint8)
        rows[1] = 8
        second_features.write_bytes(HEADER.pack(
            b"MNKF", 1, 1, 68, 2, hashlib.sha256(second_data.read_bytes()).digest(),
        ) + rows.tobytes())
        dataset = Dataset([self.data, second_data])
        mapped = KingFeatures(dataset, [self.features, second_features])
        np.testing.assert_array_equal(mapped.gather(np.array([5, 0, 4, 2, 5])),
                                      np.vstack((rows[1], self.values[0], rows[0], self.values[2], rows[1])))
        for invalid in (np.array([-1]), np.array([6])):
            with self.assertRaises(IndexError):
                mapped.gather(invalid)


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

    def test_override_applies_to_rescored_selfplay_classes(self):
        sidecar = self.root / 'rescore.bin'
        write_rescore(sidecar, self.source, [(1, 0, 120, 1, 100), (0, 0, 0, 0, 0)] * 120)
        dataset = Dataset([self.source], rescore=[sidecar], lambda_override=1)
        self.assertEqual(dataset.generation_count, 2)
        np.testing.assert_array_equal(dataset.teacher_lambdas, [1, 1])
        self.assertEqual([c.lambda_override for c in dataset.teacher_classes], [1, 1])

    def test_rejects_out_of_range_nonfinite_and_nonnumeric_values(self):
        for value in (-.01, 1.01, math.nan, math.inf, -math.inf, True, False, '0'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                Dataset([self.source], lambda_override=value)


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

    def test_multiple_files_keep_local_windows_and_global_indices(self):
        other = self.root / 'other.bin'
        rows = self.rows.copy()
        rows['score'] = [200, 50, -75, 99]
        mnsd.write_mnsd(other, rows, seed=13, network_checksum=b'a' * 32)
        write_provenance(other)
        dataset = Dataset([self.path, other], lookahead={'gamma': .9, 'plies': 3})
        np.testing.assert_allclose(dataset.teacher_scores(np.array([4, 0, 5, 1, 7, 3])),
                                   [10.75 / 1.81, 124 / 1.81, -75, -400, 99, 50])
        self.assertEqual(dataset.generation_count, 1)
        for options in ({'gamma': .7, 'plies': 3}, {'gamma': .9, 'plies': 40}):
            self.assertNotEqual(self.future.teacher_classes, Dataset([self.path], lookahead=options).teacher_classes)


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


    def test_required_provenance_and_checksum(self):
        provenance_path(self.source).unlink()
        with self.assertRaises(OSError):
            Dataset([self.source])
        self.metadata(mnsd_sha256="0" * 64)
        with self.assertRaises(ValueError):
            Dataset([self.source])

    def test_provenance_domains(self):
        cases = [
            {"format": "wrong"}, {"version": 2}, {"version": True},
            {"result_origin": "unknown"}, {"start_origin": "unknown"},
            {"lambda": -0.01}, {"lambda": 1.01}, {"lambda": float("nan")}, {"lambda": True},
            {"games": []}, {"teacher": {}},
        ]
        for key, value in (("generation_commit", "bad"), ("network_checksum", "bad"),
                           ("nodes", -1), ("nodes", True), ("nodes", 2**32),
                           ("rule_set", ""), ("search_condition", "unknown")):
            cases.append({"teacher": dict(self.provenance["teacher"], **{key: value})})
        for change in cases:
            with self.subTest(change=change):
                self.metadata(**change)
                with self.assertRaises(ValueError):
                    Dataset([self.source])
        for key in self.provenance:
            value = dict(self.provenance)
            del value[key]
            provenance_path(self.source).write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                Dataset([self.source])

    def test_game_mapping_requirements(self):
        for games in (None, [{"game": 1, "id": "abc"}],
                      [{"game": 1, "id": "abc", "ply": -1}],
                      [{"game": 0, "id": "abc", "ply": 0}],
                      [{"game": 1, "id": "", "ply": 0}],
                      [{"game": 1, "id": "abc", "ply": 0}] * 2):
            with self.subTest(games=games):
                self.metadata(start_origin="human-game", games=games)
                with self.assertRaises(ValueError):
                    Dataset([self.source])


    def test_id_split_agrees_across_origins_files_and_seeds(self):
        games = self.human_games()
        self.metadata(result_origin="human", start_origin="human-game", games=games, **{"lambda": 0})
        other = self.root / "other.bin"
        mnsd.write_mnsd(other, self.rows, seed=11, network_checksum=b"a" * 32)
        write_provenance(other, start_origin="human-game", games=games)
        dataset = Dataset([self.source, other])
        expected = [int.from_bytes(hashlib.sha256(g["id"].encode()).digest()[:8], "little") % 20 == 0 for g in games]
        np.testing.assert_array_equal(dataset.validation_masks[0], expected)
        np.testing.assert_array_equal(dataset.validation_masks[1], expected)
        self.assertEqual(dataset.game_keys(np.array([0, 100])), [("human", "game-1")] * 2)
        self.assertEqual(dataset.generation_count, 2)
        # 対局結果だけを教師とする実戦棋譜の平均に、実戦開始の自己対局を混ぜない。
        groups = {}
        model = make_model(torch.zeros((FEATURE_COUNT, 1)), torch.device("cpu"), "single")
        validation_loss(model, dataset, np.array([np.nan, 1000.]), 1., 100, torch.device("cpu"),
                        indices=np.arange(100, 200), breakdown=groups)
        self.assertEqual(groups["origins"]["human-start-selfplay"]["positions"], 100)
        self.assertIsNone(groups["human_game_mean"]["loss"])

    def test_all_seven_fields_define_classes(self):
        variants = [("generation_commit", "a" * 40), ("network_checksum", "b" * 64),
                    ("nodes", 200000), ("rule_set", "another-rule"), ("search_condition", "standalone"),
                    ("result_origin", "human"), ("start_origin", "human-game")]
        for field, value in variants:
            with self.subTest(field=field):
                other = self.root / "other.bin"
                mnsd.write_mnsd(other, self.rows, seed=22, network_checksum=b"a" * 32)
                changes = {"teacher": dict(self.provenance["teacher"])}
                if field in changes["teacher"]:
                    changes["teacher"][field] = value
                else:
                    changes[field] = value
                    changes["games"] = self.human_games()
                write_provenance(other, **changes)
                self.assertEqual(Dataset([self.source, other]).generation_count, 2)

    def test_rejects_duplicate_content_and_conflicting_mix(self):
        other = self.root / "other.bin"
        other.write_bytes(self.source.read_bytes())
        write_provenance(other)
        with self.assertRaises(ValueError):
            Dataset([self.source, other])
        mnsd.write_mnsd(other, self.rows, seed=22, network_checksum=b"a" * 32, teacher_nodes=100000)
        write_provenance(other, **{"lambda": 0.5})
        with self.assertRaises(ValueError):
            Dataset([self.source, other])

    def test_rescore_rejects_partial_and_invalid_contract_fields(self):
        rows = [(1, 0, 42, 1, 100)] * 100
        write_rescore(self.sidecar, self.source, rows)
        original = self.sidecar.read_bytes()
        mutations = [(0, b"WRNG"), (4, struct.pack("<I", 2)), (8, bytes(32)),
                     (40, struct.pack("<Q", 99)), (48, bytes(32)), (80, struct.pack("<Q", 99)),
                     (160, b"\1"), (161, b"\1"), (236, b"\1"), (164, b"z"),
                     (240, b"\3"), (241, b"\2"), (244, bytes(4))]
        for offset, value in mutations:
            changed = bytearray(original)
            changed[offset:offset + len(value)] = value
            self.sidecar.write_bytes(changed)
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                Dataset([self.source], rescore=[self.sidecar])
        for value in (original[:-1], original[:-16], original + bytes(16)):
            self.sidecar.write_bytes(value)
            with self.assertRaises(ValueError):
                Dataset([self.source], rescore=[self.sidecar])
        with self.assertRaises(ValueError):
            Dataset([self.source], rescore=[])

    def test_rescore_direct_input_equivalence_and_original_feature_rows(self):
        # 共有契約の境界: ±29000、捕獲/成り、未完了。重なる理由も含む。
        rows = [(1, 0, 100 if i % 2 else -100, 3, 200) for i in range(100)]
        rows[:7] = [(0, 0, 0, 0, 0), (1, 0, 28999, 1, 50), (1, 0, 29000, 1, 50),
                    (1, 0, -29000, 1, 50), (1, 1, 10, 1, 50), (2, 0, 0, 0, 20),
                    (1, 1, -32768, 1, 50)]
        before = self.source.read_bytes()
        write_rescore(self.sidecar, self.source, rows)
        features = self.root / "features.bin"
        features.write_bytes(HEADER.pack(b"MNKF", 1, 1, 1, 100, hashlib.sha256(before).digest()) + bytes(range(100)))
        replaced = Dataset([self.source], rescore=[self.sidecar])
        mapped_features = KingFeatures(replaced, [features])
        self.assertEqual(mapped_features.column_count, 1)
        expected_kept = np.array([0, 1, *range(7, 100)])
        self.assertEqual(replaced.exclusions, {"mate_band": 3, "tactical": 2, "depth_incomplete": 1, "total": 5})
        kept = np.sort(np.concatenate((replaced.training_indices, replaced.validation_indices)))
        np.testing.assert_array_equal(kept, expected_kept)
        np.testing.assert_array_equal(mapped_features.gather(kept)[:, 0], expected_kept)
        direct_rows = self.rows[expected_kept].copy()
        direct_rows["score"][1:] = [rows[i][2] for i in expected_kept[1:]]
        direct = self.root / "direct.bin"
        mnsd.write_mnsd(direct, direct_rows, seed=11, network_checksum=b"a" * 32)
        write_provenance(direct)
        dataset = Dataset([direct])
        for split in ("training_indices", "validation_indices"):
            left_indices, right_indices = getattr(replaced, split), getattr(dataset, split)
            left, right = replaced.gather(left_indices), dataset.gather(right_indices)
            np.testing.assert_array_equal(left, right)
            targets = build_targets(left, np.full(replaced.generation_count, 400.), replaced.generations(left_indices), replaced.teacher_lambdas)
            expected = 0.75 / (1 + np.exp(-right["score"].astype(float) / 400)) + 0.25 * right["result"] / 2
            np.testing.assert_allclose(targets, expected, rtol=1e-6)
        self.assertEqual(self.source.read_bytes(), before)
        classes = replaced.teacher_classes
        self.assertEqual(classes[1].generation_commit, "a" * 40)
        self.assertEqual(classes[1].nodes, 200000)
        self.assertEqual(classes[1].search_condition, "standalone")
        self.assertEqual(classes[1].result_origin, "selfplay")

        # 全行を付け直した場合は、直接保存した教師と分類・推定Kも一致する。
        rows = [(1, 0, -300 if i % 2 == 0 else 700, 2, 500) for i in range(100)]
        rows[10] = (1, 0, 29000, 2, 500)
        rows[20] = (1, 1, 0, 2, 500)
        rows[30] = (2, 0, 0, 0, 500)
        write_rescore(self.sidecar, self.source, rows)
        replaced = Dataset([self.source], rescore=[self.sidecar])
        kept = np.array([i for i in range(100) if i not in (10, 20, 30)])
        direct_rows = self.rows[kept].copy()
        direct_rows["score"] = [rows[i][2] for i in kept]
        path = self.root / "direct-complete.bin"
        mnsd.write_mnsd(path, direct_rows, seed=11, network_checksum=b"a" * 32,
                   teacher_nodes=200000, generation_commit="a" * 40)
        metadata = write_provenance(path)
        metadata["teacher"]["search_condition"] = "standalone"
        provenance_path(path).write_text(json.dumps(metadata))
        direct = Dataset([path])
        self.assertEqual(replaced.class_metadata(), direct.class_metadata())
        left_k, _ = estimate_generation_ks(replaced, indices=replaced.training_indices)
        right_k, _ = estimate_generation_ks(direct, indices=direct.training_indices)
        np.testing.assert_array_equal(left_k, right_k)


class TrainHalfTest(unittest.TestCase):

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / 'selfplay.bin'
        board = np.zeros((240, 144), dtype='u1')
        board[:, 0] = 1
        board[1::3, 17] = 2
        board[2::3, 29] = 65
        write_mnsd(self.source, seed=11, checksum=b'a' * 32,
                   games=np.repeat(np.arange(1, 81), 3).tolist(),
                   scores=[100, 200, -400] * 80, results=[2, 0, 2] * 80,
                   board=board)

    def assert_partition(self, dataset):
        halves = dataset.training_halves()
        self.assertTrue(all(indices.size for indices in halves))
        self.assertEqual(np.intersect1d(*halves).size, 0)
        np.testing.assert_array_equal(np.sort(np.concatenate(halves)), dataset.training_indices)
        keys = [set(dataset.game_keys(indices)) for indices in halves]
        self.assertTrue(keys[0].isdisjoint(keys[1]))
        for indices, group in zip(halves, keys):
            self.assertEqual(np.intersect1d(indices, dataset.validation_indices).size, 0)
            self.assertTrue(group.isdisjoint(dataset.game_keys(dataset.validation_indices)))
            self.assertTrue(np.all(np.diff(indices) > 0))
        return keys

    def test_partition_and_identity_survive_file_order_and_record_order(self):
        other = self.root / 'other.bin'
        write_mnsd(other, seed=2**64 - 1, checksum=b'b' * 32,
                   games=np.repeat(np.arange(80), 2).tolist())
        human_paths = []
        for file in range(2):
            path = self.root / f'human-{file}.bin'
            # 同じ棋譜IDに別のファイルシード・ローカル対局番号を与える。
            numbers = np.arange(1, 81) + file * 100
            if file:
                numbers = numbers[::-1]
            write_mnsd(path, seed=20 + file, checksum=b'c' * 32,
                       games=np.repeat(numbers, 2).tolist())
            write_provenance(path, result_origin='human' if file == 0 else 'selfplay',
                             start_origin='human-game', **{
                'lambda': 0 if file == 0 else .75,
                'games': [{'game': int(n), 'id': f'棋譜-{n - file * 100}', 'ply': 0}
                          for n in numbers]})
            human_paths.append(path)
        paths = [self.source, other, *human_paths]
        expected = self.assert_partition(Dataset(paths))
        self.assertEqual(expected, self.assert_partition(Dataset(paths[::-1])))
        first = self.assert_partition(Dataset([human_paths[0]]))
        self.assertEqual(first, self.assert_partition(Dataset([human_paths[1]])))
        # 検証ハッシュの下位ビットを群番号に流用していないことも確かめる。
        dataset = Dataset([self.source])
        sizes = [len(indices) for indices in dataset.training_halves()]
        self.assertTrue(all(.3 < size / sum(sizes) < .7 for size in sizes))
        for indices in dataset.training_halves():
            validation_bits = hash64(11, dataset.gather(indices)['game']) % 2
            self.assertEqual(set(validation_bits), {0, 1})

    def test_rescore_exclusions_remain_outside_both_halves(self):
        sidecar = self.root / 'rescore.bin'
        write_rescore(sidecar, self.source,
                      [(1, 1, 120, 1, 100), (2, 0, 0, 0, 100), (0, 0, 0, 0, 0)] * 80)
        dataset = Dataset([self.source], rescore=[sidecar])
        self.assert_partition(dataset)
        for indices in dataset.training_halves():
            self.assertTrue(np.all(indices % 3 == 2))


class DatasetTest(unittest.TestCase):

    """Datasetの来歴検証、分割、世代、収集順を検証する。"""

    def test_rejects_duplicate_path_and_seed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first.bin"
            second = root / "second.bin"
            write_mnsd(first, seed=7, checksum=b"a" * 32, games=[0])
            write_mnsd(second, seed=7, checksum=b"b" * 32, games=[1])

            with self.assertRaises(ValueError):
                Dataset([first, first.resolve()])
            with self.assertRaises(ValueError):
                Dataset([first, second])

    def test_hash_split_is_stable_when_file_order_changes(self) -> None:
        games = list(range(80))
        # 既存データの検証分割を保つ回帰値。厳密なハッシュ式は規範文書に未定義。
        # 0、シード差、u64の桁あふれを含む既知の参照値を固定する。
        for seed, game, expected in (
            (0, 0, 0),
            (1, 0, 0x5692161D100B05E5),
            (0, 1, 0xE220A8397B1DCDAF),
            (0xFFFFFFFFFFFFFFFF, 0xFFFFFFFF, 0x3A9B57C277B22E0A),
        ):
            with self.subTest(seed=seed, game=game):
                self.assertEqual(int(hash64(seed, np.array([game], dtype=np.uint32))[0]), expected)
        validation_games = {
            11: {4, 15, 22, 41, 48, 59, 70},
            22: {6, 8, 35, 44, 58, 63, 74},
        }

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first.bin"
            second = root / "second.bin"
            write_mnsd(first, seed=11, checksum=b"a" * 32, games=games)
            write_mnsd(second, seed=22, checksum=b"a" * 32, games=games)

            memberships: list[dict[tuple[int, int], bool]] = []
            for paths in ([first, second], [second, first]):
                dataset = Dataset(paths)
                membership: dict[tuple[int, int], bool] = {}
                for header, records, validation in zip(
                    dataset.headers, dataset.records, dataset.validation_masks
                ):
                    for game, selected in zip(records["game"], validation):
                        key = (header.seed, int(game))
                        membership[key] = bool(selected)
                        self.assertEqual(
                            bool(selected), int(game) in validation_games[header.seed]
                        )
                memberships.append(membership)
            self.assertEqual(memberships[0], memberships[1])

    def test_gather_preserves_arbitrary_order_and_duplicates(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first.bin"
            second = root / "second.bin"
            write_mnsd(
                first, seed=1, checksum=b"a" * 32, games=[10, 11, 12], scores=[1, 2, 3]
            )
            write_mnsd(
                second, seed=2, checksum=b"a" * 32, games=[20, 21], scores=[4, 5]
            )
            dataset = Dataset([first, second])

            gathered = dataset.gather(np.array([4, 0, 3, 1, 4], dtype=np.int64))
            self.assertEqual(gathered.dtype, RECORD_DTYPE)
            self.assertEqual(gathered["score"].tolist(), [5, 1, 4, 2, 5])
            self.assertEqual(dataset.gather(np.array([], dtype=np.int64)).shape, (0,))


if __name__ == "__main__":
    unittest.main()
