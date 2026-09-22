"""MNKF診断の列選択とファイル間の対応を検証する。"""

import hashlib
from pathlib import Path
import tempfile
import unittest

import numpy as np

from king_features_diag import parse_ranges
from mnsd import HEADER, Dataset, KingFeatures, RECORD_DTYPE, write_mnsd
from test_train_pst import write_provenance


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
        write_mnsd(self.data, self.records, seed=0, network_checksum=bytes(32))
        write_provenance(self.data)
        self.values = np.tile(np.arange(68, dtype=np.uint8), (200, 1)) % 3 + 1
        self.write_features()

    def write_features(self, definition=1):
        self.mnkf.write_bytes(HEADER.pack(
            b'MNKF', 1, definition, self.values.shape[1], len(self.records), hashlib.sha256(self.data.read_bytes()).digest()
        ) + self.values.tobytes())

    def dataset(self, columns=(0, 1, 62)):
        return KingFeatures(Dataset([self.data]), [self.mnkf], columns)

    def test_noncontiguous_columns_keep_requested_order_across_files(self):
        other = self.root / 'other.bin'
        other_features = self.root / 'other.mnkf'
        write_mnsd(other, self.records[:2], seed=5, network_checksum=b'a' * 32)
        write_provenance(other)
        other_values = np.full((2, 68), 7, dtype=np.uint8)
        other_features.write_bytes(HEADER.pack(
            b'MNKF', 1, 1, 68, 2, hashlib.sha256(other.read_bytes()).digest()
        ) + other_values.tobytes())
        columns = parse_ranges('62:64,0:2', 68)
        self.assertEqual(columns, [62, 63, 0, 1])
        dataset = KingFeatures(Dataset([self.data, other]), [self.mnkf, other_features], columns)
        expected = np.array([[7, 7, 7, 7], [3, 1, 1, 2], [7, 7, 7, 7], [3, 1, 1, 2]])
        np.testing.assert_array_equal(dataset.gather(np.array([201, 0, 200, 1])), expected)

    def test_range_rejections(self):
        for value in ('', '0:0', '-1:3', '0:69', '0:2,1:3', '0', '1:2:3', '2:1'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                parse_ranges(value, 68)

    def test_mixed_definitions_and_widths_are_rejected(self):
        # 選択した列が共通でも、入力ファイルの定義と列数は一致しなければならない。
        other = self.root / 'shelter.bin'
        other_features = self.root / 'shelter.mnkf'
        write_mnsd(other, self.records[:2], seed=5, network_checksum=b'a' * 32)
        write_provenance(other)
        for definition, width in ((1, 24), (2, 68), (2, 118)):
            rows = np.full((2, width), 7, dtype=np.uint8)
            other_features.write_bytes(HEADER.pack(
                b'MNKF', 1, definition, width, 2, hashlib.sha256(other.read_bytes()).digest()
            ) + rows.tobytes())
            with self.subTest(definition=definition, width=width), self.assertRaises(ValueError):
                KingFeatures(Dataset([self.data, other]), [self.mnkf, other_features], range(24))

    def test_selected_columns_use_file_width(self):
        for definition, width in ((1, 24), (1, 68), (2, 118)):
            self.values = np.tile(np.arange(width, dtype=np.uint8), (200, 1))
            self.write_features(definition)
            dataset = self.dataset([width - 1, 0])
            np.testing.assert_array_equal(dataset.gather(np.array([0])), [[width - 1, 0]])
            for columns in ([width], [-1], range(width + 1)):
                with self.subTest(width=width, columns=columns), self.assertRaises(ValueError):
                    self.dataset(columns)


if __name__ == "__main__":
    unittest.main()
