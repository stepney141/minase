"""段階9のMNPT変換契約を、端点ごとの配置とバイト保存で検査する。"""

import hashlib
from pathlib import Path
import struct
import tempfile
import unittest

import numpy as np

from convert_mnpt import FEATURE_COUNT, PST_FEATURE_COUNT, convert_mnpt
from train_pst import initial_piece_values, read_mnpt, write_mnpt


class ConvertMnptTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.source = Path(self.temporary.name) / "source.bin"
        self.destination = Path(self.temporary.name) / "converted.bin"
        self.mg = (np.arange(PST_FEATURE_COUNT) * 3 - 22000).astype("<i2")
        self.eg = (19000 - np.arange(PST_FEATURE_COUNT) * 2).astype("<i2")
        self.values = initial_piece_values()
        write_mnpt(self.source, self.mg, self.eg, self.values, 1072.6529541015625)

    def test_conversion_inserts_zeros_per_endpoint_and_preserves_all_other_fields(self):
        # 異なる端点と規則名でも、ヘッダの特徴数と検査和以外はバイト単位で保存する。
        for rule_set in (b"L0,P0,R1,E0", b"custom-rules", b"r" * 32):
            with self.subTest(rule_set=rule_set):
                write_mnpt(self.source, self.mg, self.eg, self.values, 1072.6529541015625,
                           rule_set=rule_set)
                before = self.source.read_bytes()
                convert_mnpt(self.source, self.destination)
                after = self.destination.read_bytes()
                self.assertEqual(len(after), 80 + 2 * 13704 * 2 + 47 * 4)
                self.assertEqual(after[:8], before[:8])
                self.assertEqual(after[12:48], before[12:48])
                self.assertEqual(struct.unpack_from("<I", after, 8)[0], 13704)
                self.assertEqual(after[48:80], hashlib.sha256(after[80:]).digest())
                for endpoint in (0, 1):
                    old_start = 80 + endpoint * 13680 * 2
                    new_start = 80 + endpoint * 13704 * 2
                    self.assertEqual(after[new_start:new_start + 13680 * 2],
                                     before[old_start:old_start + 13680 * 2])
                    self.assertEqual(after[new_start + 13680 * 2:new_start + 13704 * 2],
                                     bytes(24 * 2))
                self.assertEqual(after[-47 * 4:], before[-47 * 4:])
                mg, eg, values, k = read_mnpt(self.destination, feature_count=FEATURE_COUNT,
                                             rule_set=rule_set)
                np.testing.assert_array_equal(mg[:PST_FEATURE_COUNT], self.mg)
                np.testing.assert_array_equal(eg[:PST_FEATURE_COUNT], self.eg)
                np.testing.assert_array_equal(values, self.values)
                self.assertEqual(k, 1072.6529541015625)

    def test_invalid_input_does_not_write_output(self):
        valid = self.source.read_bytes()
        corruptions = [valid[:-1]]
        for offset in (0, 4, 8, 12, 47, 48, 80, len(valid) - 1):
            damaged = bytearray(valid)
            damaged[offset] ^= 0x80
            # Kの変更は有限な正数になり得るので、明示的に0にする。
            if offset == 12:
                damaged[12:16] = bytes(4)
            corruptions.append(bytes(damaged))
        for damaged in corruptions:
            with self.subTest(header=damaged[:16]):
                self.source.write_bytes(damaged)
                with self.assertRaises(ValueError):
                    convert_mnpt(self.source, self.destination)
                self.assertFalse(self.destination.exists())

    def test_generalized_reader_still_checks_requested_feature_count(self):
        convert_mnpt(self.source, self.destination)
        damaged = bytearray(self.destination.read_bytes())
        struct.pack_into("<I", damaged, 8, PST_FEATURE_COUNT)
        self.destination.write_bytes(damaged)
        with self.assertRaises(ValueError):
            read_mnpt(self.destination, feature_count=FEATURE_COUNT)


if __name__ == "__main__":
    unittest.main()
