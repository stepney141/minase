"""MNUE版4の公開バイト配置、許容範囲、および鏡映共有条件を検証する。"""

import hashlib
from pathlib import Path
import struct
import tempfile
import unittest

import numpy as np

from minase_train.data.mnpt import initial_piece_values
from minase_train.nnue.format import read_mnue, write_mnue


def network_arrays(width=2):
    """仕様の左右6筋と隠れユニットの半分から、共有を満たす配列を構成する。"""
    rng = np.random.default_rng(74)
    linear_half = rng.integers(-4000, 4001, size=(95, 12, 6, 2), dtype=np.int16)
    hidden_half = rng.integers(-1016, 1017, size=(95, 12, 6, width), dtype=np.int16)
    output_half = rng.integers(-8192, 8193, size=(2, width // 2), dtype=np.int16)
    bias_half = rng.integers(-1016, 1017, size=width // 2, dtype=np.int32)
    return dict(
        linear=np.concatenate((linear_half, linear_half[:, :, ::-1]), axis=2).reshape(13680, 2),
        piece_values=initial_piece_values(), k=1072.6529541015625,
        weights=np.concatenate((hidden_half, np.roll(hidden_half[:, :, ::-1], width // 2, axis=3)),
                               axis=2).reshape(13680, width),
        biases=np.tile(bias_half, 2), output_weights=np.tile(output_half, (1, 2)), output_bias=-100_000,
    )


class FormatTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = Path(directory.name) / "net.bin"

    def test_round_trip_and_byte_layout_for_different_widths(self):
        for width in (2, 4, 512):
            with self.subTest(width=width):
                arrays = network_arrays(width)
                write_mnue(self.path, **arrays)
                net = read_mnue(self.path)
                for name, expected in arrays.items():
                    np.testing.assert_array_equal(getattr(net, name), expected)
                self.assertEqual(net.width, width)
                raw = self.path.read_bytes()
                self.assertEqual(len(raw), 96 + 13680 * 4 + 47 * 4 + 13680 * 2 * width + 8 * width + 4)
                self.assertEqual(struct.unpack_from("<4sIIIfI", raw), (b"MNUE", 4, 1, width, arrays["k"], 0))
                self.assertEqual(raw[24:56], b"L0,P0,R1,E0".ljust(32, b"\0"))
                self.assertEqual(raw[56:88], hashlib.sha256(raw[96:]).digest())
                self.assertEqual(raw[88:96], bytes(8))
                expected_body = b"".join(arrays[name].astype(dtype).tobytes() for name, dtype in (
                    ("linear", "<i2"), ("piece_values", "<i4"), ("weights", "<i2"),
                    ("biases", "<i4"), ("output_weights", "<i2"))) + struct.pack("<i", arrays["output_bias"])
                self.assertEqual(raw[96:], expected_body)

    def test_header_length_and_checksum_violations_are_rejected(self):
        for width in (2, 4):
            write_mnue(self.path, **network_arrays(width))
            good = self.path.read_bytes()
            cases = [(0, "<4s", b"MNPT"), (4, "<I", 0), (4, "<I", 1), (4, "<I", 2),
                     (4, "<I", 3), (4, "<I", 5), (8, "<I", 0), (8, "<I", 2),
                     *[(12, "<I", h) for h in (0, 1, 3, 514)],
                     *[(16, "<f", k) for k in (0, -1, float("nan"), float("inf"), 100001)],
                     (20, "<I", 1), (24, "B", 255), (40, "B", 1), (88, "B", 1), (95, "B", 1)]
            for offset, kind, value in cases:
                with self.subTest(width=width, offset=offset, value=value):
                    bad = bytearray(good)
                    struct.pack_into(kind, bad, offset, value)
                    self.path.write_bytes(bad)
                    with self.assertRaisesRegex(ValueError, str(self.path)):
                        read_mnue(self.path)
            for bad in (good[:95], good[:-1], good + b"\0", good[:96] + bytes(len(good) - 96)):
                self.path.write_bytes(bad)
                with self.assertRaises(ValueError):
                    read_mnue(self.path)

    def test_reader_rejects_body_ranges_and_sharing_with_valid_checksum(self):
        width = 4
        write_mnue(self.path, **network_arrays(width))
        good = self.path.read_bytes()
        pieces = 96 + 13680 * 4
        weights = pieces + 47 * 4
        biases = weights + 13680 * 2 * width
        output = biases + 4 * width
        cases = [(96, "<h", 123), (pieces + 29 * 4, "<i", 0),
                 (pieces + 11 * 4, "<i", 100), (pieces, "<i", -1), (pieces, "<i", 29000),
                 (weights, "<h", 1017), (weights, "<h", -1017), (weights, "<h", 0),
                 (biases, "<i", 1017), (biases, "<i", -1017), (biases, "<i", 0),
                 (output, "<h", 8193), (output, "<h", -8193), (output, "<h", 0),
                 (output + 4 * width, "<i", 4161537), (output + 4 * width, "<i", -4161537)]
        for offset, kind, value in cases:
            with self.subTest(offset=offset, value=value):
                bad = bytearray(good)
                struct.pack_into(kind, bad, offset, value)
                bad[56:88] = hashlib.sha256(bad[96:]).digest()
                self.path.write_bytes(bad)
                with self.assertRaisesRegex(ValueError, str(self.path)):
                    read_mnue(self.path)

    def test_writer_rejects_invalid_arrays_without_creating_file(self):
        cases = [
            ("linear", np.zeros((13680, 2), dtype=np.float32)),
            ("linear", np.full((13680, 2), 32768, dtype=np.int32)),
            ("linear", np.full((13680, 2), -32769, dtype=np.int32)),
            ("weights", np.zeros((13680, 3), dtype=np.int16)),
            ("weights", np.zeros((13679, 2), dtype=np.int16)),
            ("biases", np.full(2, 1017)), ("weights", np.full((13680, 2), -1017)),
            ("output_weights", np.full((2, 2), 8193)), ("output_bias", 4161537),
            ("output_bias", -4161537), ("output_bias", 0.5),
            *[("k", k) for k in (0, -1, float("nan"), float("inf"), 100001, 1e-100)],
        ]
        for key, value in cases:
            with self.subTest(key=key, shape=np.shape(value)):
                arrays = network_arrays()
                arrays[key] = value
                with self.assertRaisesRegex(ValueError, str(self.path)):
                    write_mnue(self.path, **arrays)
                self.assertFalse(self.path.exists())
        for name in ("linear", "weights", "biases", "output_weights"):
            arrays = network_arrays(4)
            arrays[name].flat[0] = 0
            with self.subTest(sharing=name), self.assertRaisesRegex(ValueError, "sharing"):
                write_mnue(self.path, **arrays)
            self.assertFalse(self.path.exists())
        arrays = network_arrays()
        arrays["piece_values"][29] = 0
        with self.assertRaises(ValueError):
            write_mnue(self.path, **arrays)

    def test_inclusive_limits_and_f32_k(self):
        for sign in (-1, 1):
            arrays = network_arrays(2)
            arrays.update(linear=np.full((13680, 2), -32768 if sign < 0 else 32767, dtype=np.int16),
                          weights=np.full((13680, 2), sign * 1016, dtype=np.int16),
                          biases=np.full(2, sign * 1016, dtype=np.int32),
                          output_weights=np.full((2, 2), sign * 8192, dtype=np.int16),
                          output_bias=sign * 4161536, k=100000)
            write_mnue(self.path, **arrays)
            self.assertEqual(read_mnue(self.path).k, 100000)
        arrays["k"] = 1072.65
        write_mnue(self.path, **arrays)
        self.assertEqual(read_mnue(self.path).k, float(np.float32(1072.65)))


if __name__ == "__main__":
    unittest.main()
