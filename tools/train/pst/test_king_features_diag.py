"""MNKFの全列読み込みとMNSDとの対応を検証する。"""

import hashlib
from pathlib import Path
import struct
import tempfile
import unittest

import numpy as np

from test_train_pst import write_provenance
from mnsd import HEADER, KingFeatures
from mnsd import Dataset, RECORD_DTYPE, hash64, write_mnsd


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
        write_mnsd(self.data, self.records, seed=17, network_checksum=bytes(32))
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
        write_mnsd(self.data, changed, seed=17, network_checksum=bytes(32))
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


if __name__ == "__main__":
    unittest.main()
