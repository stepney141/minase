"""MNKFの定義と列幅がファイル間で一致することを検証する。"""

import hashlib
from pathlib import Path
import tempfile
import unittest

import numpy as np

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

    def write_features(self):
        self.mnkf.write_bytes(HEADER.pack(
            b'MNKF', 1, 1, self.values.shape[1], len(self.records), hashlib.sha256(self.data.read_bytes()).digest()
        ) + self.values.tobytes())

    def test_mixed_definitions_and_widths_are_rejected(self):
        # 入力ファイルの定義と列数は一致しなければならない。
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
                KingFeatures(Dataset([self.data, other]), [self.mnkf, other_features])


if __name__ == "__main__":
    unittest.main()
