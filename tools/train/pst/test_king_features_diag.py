"""段階9の交換形式と統計を、手計算できる人工局面で検証する。"""

import hashlib
import json
import math
from pathlib import Path
import struct
import tempfile
import unittest

import numpy as np

from king_features_diag import Statistics, diagnose
from test_train_pst import write_provenance
from mnsd import COLUMN_COUNT, HEADER, KingFeatures
from mnsd import Dataset, RECORD_DTYPE, hash64, write_mnsd
from train_pst import FEATURE_COUNT, initial_piece_values, write_mnpt


class KingFeaturesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.data = self.directory / "data.bin"
        self.features = self.directory / "features.bin"
        self.pst = self.directory / "pst.bin"
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
        write_mnsd(self.data, self.records, seed=17, network_checksum=bytes(32))
        write_provenance(self.data)
        self.values = np.zeros((4, COLUMN_COUNT), dtype=np.uint8)
        self.values[:, 0] = [0, 1, 2, 255]
        self.values[:, 1] = [2, 1, 0, 255]
        self.values[:, 2] = 1  # 分散0の列の相関は未定義。
        self.write_features()
        zero = np.zeros(FEATURE_COUNT, dtype=np.int16)
        write_mnpt(self.pst, zero, zero, initial_piece_values(), 10.0)
        self.dataset = Dataset([self.data])

    def write_features(self):
        self.features.write_bytes(HEADER.pack(
            b"MNKF", 1, 1, self.values.shape[1], len(self.values), hashlib.sha256(self.data.read_bytes()).digest(),
        ) + self.values.tobytes())

    def test_rejects_input_sha256_mismatch(self):
        # 交換形式の契約: 入力全体に結び付くので、盤面以外の教師値の変更も拒否する。
        changed = self.records.copy()
        changed["score"][0] = 1
        write_mnsd(self.data, changed, seed=17, network_checksum=bytes(32))
        write_provenance(self.data)
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            KingFeatures(Dataset([self.data]), [self.features])

    def test_rejects_each_header_mismatch_and_body_length(self):
        # magic・版・定義・列数・局面数を、それ以外を維持したまま1つずつ変える。
        original = self.features.read_bytes()
        for offset, fmt, value in [(0, "4s", b"BAD!"), (4, "I", 2), (8, "I", 2),
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

    def test_training_statistics_match_hand_calculation(self):
        # 訓練のみx=(0,1,2)。発火率2/3、x²加重φ平均=(.5+4)/5=.9、
        # Σx²(φ−.9)²=.16+.04=.20。逆順列との相関は−1。
        mapped = KingFeatures(self.dataset, [self.features])
        result = diagnose(self.dataset, mapped, self.pst, 10.0, lambda_value=0, batch=2)
        self.assertEqual(result["training_positions"], 3)
        self.assertEqual(result["generation_training_counts"], [3])
        column = result["columns"][0]
        self.assertAlmostEqual(column["activation_rate"], 2 / 3)
        self.assertEqual(column["histogram"], {"0": 1, "1": 1, "2": 1})
        self.assertAlmostEqual(column["phase_weighted_mean"], .9)
        self.assertAlmostEqual(column["identifiability_ssd"], .2)
        self.assertAlmostEqual(result["correlation_matrix"][0][1], -1)
        # 基準重み0だからp=1/2。λ=0ならy=(0,1/2,1)でxとの相関は1。
        self.assertAlmostEqual(column["residual_correlation"], 1)
        self.assertIsNone(result["correlation_matrix"][0][2])
        self.assertIsNone(result["columns"][3]["phase_weighted_mean"])
        self.assertEqual(result["columns"][3]["identifiability_ssd"], 0)
        self.assertEqual(result["two_royals_rates"], {"either": 2 / 3, "stm": 2 / 3, "opponent": 1 / 3})
        json.dumps(result, allow_nan=False)
        single = diagnose(self.dataset, mapped, self.pst, 10.0, lambda_value=0, batch=1)
        self.assertAlmostEqual(single["columns"][0]["identifiability_ssd"], .2)
        self.assertAlmostEqual(single["columns"][0]["residual_correlation"], 1)

    def test_global_indices_preserve_order_across_files(self):
        # Datasetと同じ通し番号。ファイル境界を往復し、重複した番号も保存する。
        second_data = self.directory / "second.bin"
        second_features = self.directory / "second-features.bin"
        write_mnsd(second_data, self.records[:2], seed=18, network_checksum=bytes(32))
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

    def test_prediction_uses_both_endpoints_and_the_requested_output_k(self):
        # 王だけに序中盤10cp・終盤0cpを与える。φ=(0,.5,1)、出力K=20なら
        # p=(sigmoid(0),sigmoid(.25),sigmoid(.5))。教師探索値0とλ=.75から
        # y=(.375,.5,.625)。残差の相関をこの式だけから求める。
        mg = np.zeros(FEATURE_COUNT, dtype=np.int16)
        eg = np.zeros(FEATURE_COUNT, dtype=np.int16)
        mg[11 * 144:12 * 144] = 80
        write_mnpt(self.pst, mg, eg, initial_piece_values(), 10.0)
        mapped = KingFeatures(self.dataset, [self.features])
        result = diagnose(self.dataset, mapped, self.pst, 20.0, batch=2)
        probabilities = np.array([1 / (1 + math.exp(-v)) for v in (0, .25, .5)])
        residuals = np.array([.375, .5, .625]) - probabilities
        expected = np.corrcoef([0, 1, 2], residuals)[0, 1]
        self.assertAlmostEqual(result["columns"][0]["residual_correlation"], expected, places=6)

    def test_empty_training_statistics_are_rejected(self):
        with self.assertRaises(ValueError):
            Statistics().report()

    def test_shelter_diagnostics_match_selected_and_compact_files(self):
        # 診断の統計と列名も、68列のうち0〜23を選んだ場合と24列の場合で一致する。
        selected = KingFeatures(self.dataset, [self.features], range(24))
        expected = diagnose(self.dataset, selected, self.pst, 10.0, lambda_value=0, batch=2)
        self.values = self.values[:, :24]
        self.write_features()
        for columns in (None, range(24)):
            with self.subTest(columns=columns):
                compact = KingFeatures(self.dataset, [self.features], columns)
                actual = diagnose(self.dataset, compact, self.pst, 10.0, lambda_value=0, batch=2)
                self.assertEqual(actual, expected)
                self.assertEqual(actual['column_count'], 24)
                self.assertEqual(len(actual['columns']), 24)
                self.assertTrue(all(column['name'].startswith('shelter.') for column in actual['columns']))
        # 列数が有効でも、定義の識別値が異なるファイルは拒否する。
        invalid = bytearray(self.features.read_bytes())
        struct.pack_into('<I', invalid, 8, 2)
        self.features.write_bytes(invalid)
        with self.assertRaisesRegex(ValueError, 'definition ID'):
            KingFeatures(self.dataset, [self.features], range(24))


if __name__ == "__main__":
    unittest.main()
