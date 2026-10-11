"""MNUE版5の公開バイト配置、許容範囲、および鏡映共有条件を検証する。"""

import hashlib
from pathlib import Path
import struct
import tempfile
import unittest

import numpy as np

from minase_train.data.mnpt import initial_piece_values
from minase_train.nnue.format import architecture_string, read_mnue, write_mnue


def network_arrays(width=2, feature_set="P"):
    """仕様の左右6筋と隠れユニットの半分から、共有を満たす配列を構成する。"""
    rng = np.random.default_rng(74)
    linear_half = rng.integers(-4000, 4001, size=(95, 12, 6, 2), dtype=np.int16)
    hidden_half = rng.integers(-1016, 1017, size=(95, 12, 6, width), dtype=np.int16)
    output_half = rng.integers(-8192, 8193, size=(2, width // 2), dtype=np.int16)
    bias_half = rng.integers(-1016, 1017, size=width // 2, dtype=np.int32)
    arrays = dict(
        feature_set=feature_set, linear=np.concatenate((linear_half, linear_half[:, :, ::-1]), axis=2).reshape(13680, 2),
        piece_values=initial_piece_values(), k=1072.6529541015625,
        weights=np.concatenate((hidden_half, np.roll(hidden_half[:, :, ::-1], width // 2, axis=3)),
                               axis=2).reshape(13680, width),
        biases=np.tile(bias_half, 2), output_weights=np.tile(output_half, (1, 2)), output_bias=-100_000,
    )

    if feature_set == "HalfKP-hm-b8":
        arrays.update(weights=rng.integers(-1016, 1017, (109440, width), dtype=np.int16),
                      biases=rng.integers(-1016, 1017, width, dtype=np.int32),
                      output_weights=rng.integers(-8192, 8193, (2, width), dtype=np.int16))
    return arrays


class FormatTest(unittest.TestCase):
    feature_set = "P"
    count = 13680
    widths = (2, 4, 512)

    def arrays(self, width=2):
        return network_arrays(width, self.feature_set)

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / "net.bin"

    def test_round_trip_and_byte_layout_for_different_widths(self):
        for width in self.widths:
            with self.subTest(width=width):
                arrays = self.arrays(width)
                write_mnue(self.path, **arrays)
                net = read_mnue(self.path)
                for name, expected in arrays.items():
                    np.testing.assert_array_equal(getattr(net, name), expected)
                self.assertEqual(net.width, width)
                raw = self.path.read_bytes()
                self.assertEqual(len(raw), 192 + 13680 * 4 + 47 * 4 + self.count * 2 * width + 8 * width + 4)
                self.assertEqual(struct.unpack_from("<4sI", raw), (b"MNUE", 5))
                description = f"features={self.feature_set};ft={width};linear=tapered2;head=direct-screlu"
                self.assertEqual(architecture_string(self.feature_set, width), description)
                self.assertEqual(raw[8:104], description.encode("ascii").ljust(96, b"\0"))
                self.assertEqual(struct.unpack_from("<If", raw, 104), (width, arrays["k"]))
                self.assertEqual(raw[112:144], b"L0,P0,R1,E0".ljust(32, b"\0"))
                self.assertEqual(raw[144:176], hashlib.sha256(raw[192:]).digest())
                self.assertEqual(raw[176:192], bytes(16))
                expected_body = b"".join(arrays[name].astype(dtype).tobytes() for name, dtype in (
                    ("linear", "<i2"), ("piece_values", "<i4"), ("weights", "<i2"),
                    ("biases", "<i4"), ("output_weights", "<i2"))) + struct.pack("<i", arrays["output_bias"])
                self.assertEqual(raw[192:], expected_body)

    def test_header_length_and_checksum_violations_are_rejected(self):
        for width in (2, 4):
            write_mnue(self.path, **self.arrays(width))
            good = self.path.read_bytes()
            cases = [(0, "<4s", b"MNPT"),
                     *[(4, "<I", version) for version in (0, 1, 2, 3, 4, 6)],
                     *[(104, "<I", h) for h in (0, 1, 3, 514, width + 2)],
                     *[(108, "<f", k) for k in (0, -1, float("nan"), float("inf"), 100001)],
                     (8, "B", 255), (8, "B", 0), (103, "B", 1),
                     (112, "B", 255), (143, "B", 1), (176, "B", 1), (191, "B", 1)]
            for offset, kind, value in cases:
                with self.subTest(width=width, offset=offset, value=value):
                    bad = bytearray(good)
                    struct.pack_into(kind, bad, offset, value)
                    self.path.write_bytes(bad)
                    with self.assertRaisesRegex(ValueError, str(self.path)):
                        read_mnue(self.path)
            for bad in (good[:191], good[:-1], good + b"\0", good[:192] + bytes(len(good) - 192)):
                self.path.write_bytes(bad)
                with self.assertRaises(ValueError):
                    read_mnue(self.path)

    def test_reader_rejects_body_ranges_and_sharing_with_valid_checksum(self):
        width = 4
        write_mnue(self.path, **self.arrays(width))
        good = self.path.read_bytes()
        pieces = 192 + 13680 * 4
        weights = pieces + 47 * 4
        biases = weights + self.count * 2 * width
        output = biases + 4 * width
        cases = [(192, "<h", 123), (pieces + 29 * 4, "<i", 0),
                 (pieces + 11 * 4, "<i", 100), (pieces, "<i", -1), (pieces, "<i", 29000),
                 (weights, "<h", 1017), (weights, "<h", -1017),
                 (biases, "<i", 1017), (biases, "<i", -1017),
                 (output, "<h", 8193), (output, "<h", -8193),
                 (output + 4 * width, "<i", 4161537), (output + 4 * width, "<i", -4161537)]
        if self.feature_set == "P":
            cases.extend([(weights, "<h", 0), (biases, "<i", 0), (output, "<h", 0)])
        for offset, kind, value in cases:
            with self.subTest(offset=offset, value=value):
                bad = bytearray(good)
                struct.pack_into(kind, bad, offset, value)
                bad[144:176] = hashlib.sha256(bad[192:]).digest()
                self.path.write_bytes(bad)
                with self.assertRaisesRegex(ValueError, str(self.path)):
                    read_mnue(self.path)

    def test_writer_rejects_invalid_arrays_without_creating_file(self):
        cases = [
            ("linear", np.zeros((13680, 2), dtype=np.float32)),
            ("linear", np.full((13680, 2), 32768, dtype=np.int32)),
            ("linear", np.full((13680, 2), -32769, dtype=np.int32)),
            ("weights", np.zeros((self.count, 3), dtype=np.int16)),
            ("weights", np.zeros((self.count - 1, 2), dtype=np.int16)),
            ("biases", np.full(2, 1017)), ("weights", np.full((self.count, 2), -1017)),
            ("output_weights", np.full((2, 2), 8193)), ("output_bias", 4161537),
            ("output_bias", -4161537), ("output_bias", 0.5),
            *[("k", k) for k in (0, -1, float("nan"), float("inf"), 100001, 1e-100)],
        ]
        for key, value in cases:
            with self.subTest(key=key, shape=np.shape(value)):
                arrays = self.arrays()
                arrays[key] = value
                with self.assertRaisesRegex(ValueError, str(self.path)):
                    write_mnue(self.path, **arrays)
                self.assertFalse(self.path.exists())
        shared = ("linear", "weights", "biases", "output_weights") if self.feature_set == "P" else ("linear",)
        for name in shared:
            arrays = self.arrays(4)
            arrays[name].flat[0] = 0
            with self.subTest(sharing=name), self.assertRaisesRegex(ValueError, "sharing"):
                write_mnue(self.path, **arrays)
            self.assertFalse(self.path.exists())
        arrays = self.arrays()
        arrays["piece_values"][29] = 0
        with self.assertRaises(ValueError):
            write_mnue(self.path, **arrays)

    def test_inclusive_limits_and_f32_k(self):
        for sign in (-1, 1):
            arrays = self.arrays(2)
            arrays.update(linear=np.full((13680, 2), -32768 if sign < 0 else 32767, dtype=np.int16),
                          weights=np.full((self.count, 2), sign * 1016, dtype=np.int16),
                          biases=np.full(2, sign * 1016, dtype=np.int32),
                          output_weights=np.full((2, 2), sign * 8192, dtype=np.int16),
                          output_bias=sign * 4161536, k=100000)
            write_mnue(self.path, **arrays)
            self.assertEqual(read_mnue(self.path).k, 100000)
        arrays["k"] = 1072.65
        write_mnue(self.path, **arrays)
        self.assertEqual(read_mnue(self.path).k, float(np.float32(1072.65)))

    def test_structure_requires_exact_name_width_head_and_padding(self):
        write_mnue(self.path, **self.arrays())
        good = self.path.read_bytes()
        description = good[8:104].rstrip(b"\0")
        for changed in (description.replace(self.feature_set.encode(), b"unknown"),
                        description.replace(b"ft=2", b"ft=02"),
                        description.replace(b"tapered2", b"tapered3"),
                        description.replace(b"direct-screlu", b"other"),
                        description + b"\0x", description + b" "):
            with self.subTest(description=changed):
                bad = bytearray(good)
                bad[8:104] = changed.ljust(96, b"\0")
                self.path.write_bytes(bad)
                with self.assertRaisesRegex(ValueError, "architecture"):
                    read_mnue(self.path)

    def test_version_four_is_explicitly_unsupported(self):
        self.path.write_bytes(b"MNUE" + struct.pack("<I", 4) + bytes(88))
        with self.assertRaisesRegex(ValueError, "unsupported version 4"):
            read_mnue(self.path)


class HalfKPFormatTest(FormatTest):
    feature_set = "HalfKP-hm-b8"
    count = 109440
    widths = (2, 4, 8)


class EmbeddedNetworkTest(unittest.TestCase):
    def test_embedded_version_five_p_width_64(self):
        path = Path(__file__).resolve().parents[3] / "crates/minase/nets/nnue.bin"
        net = read_mnue(path)
        self.assertEqual(net.feature_set, "P")
        self.assertEqual(net.width, 64)
        self.assertEqual(net.weights.shape, (13680, 64))


if __name__ == "__main__":
    unittest.main()
