"""MNPTの形式、駒価値、および重みの量子化を検証する。"""

from __future__ import annotations

import struct
import tempfile
import unittest
from pathlib import Path

import numpy as np

from helpers import PIECE_VALUES
from minase_train.data.features import FEATURE_COUNT
from minase_train.data.mnpt import (
    FILE_LENGTH,
    HEADER_LENGTH as MNPT_HEADER_LENGTH,
    quantize,
    read_mnpt,
    write_mnpt,
)


class TaperedFormatTest(unittest.TestCase):

    """補間係数、整数評価の式、MNPTバージョン2の検査、およびモデル種別の契約を検証する。"""

    def test_mnpt_corruption_and_inconsistent_piece_values_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "weights.bin"
            weights = np.arange(FEATURE_COUNT, dtype=np.int16)
            write_mnpt(path, weights, -weights, PIECE_VALUES, 300.0)
            self.assertEqual(path.stat().st_size, FILE_LENGTH)
            middlegame, endgame, values, k = read_mnpt(path)
            np.testing.assert_array_equal(middlegame, weights)
            np.testing.assert_array_equal(endgame, -weights)
            np.testing.assert_array_equal(values, PIECE_VALUES)
            self.assertEqual(k, 300.0)
            original = path.read_bytes()
            corruptions = {
                "length": original[:-1],
                "magic": b"MNPX" + original[4:],
                "version": original[:4] + struct.pack("<I", 1) + original[8:],
                "checksum": original[:MNPT_HEADER_LENGTH] + bytes([original[MNPT_HEADER_LENGTH] ^ 1]) + original[MNPT_HEADER_LENGTH + 1:],
            }
            for name, content in corruptions.items():
                with self.subTest(name=name):
                    path.write_bytes(content)
                    with self.assertRaises(ValueError):
                        read_mnpt(path)
            bad_values = [
                ("nonpositive", {29: 0}),
                ("royal", {11: 2601}),
                ("range", {4: 29_000, 11: 29_100, 21: 29_100}),
            ]
            for name, changes in bad_values:
                with self.subTest(name=name):
                    values = PIECE_VALUES.copy()
                    for state, value in changes.items():
                        values[state] = value
                    with self.assertRaises(ValueError):
                        write_mnpt(path, weights, weights, values, 300.0)


class WeightProjectionTest(unittest.TestCase):

    """各Adam更新後に、MNPTのi16/8範囲へ射影する承認済みの契約を検証する。"""

    LOWER_CP = -4096.0

    UPPER_CP = 4095.875

    def test_quantize_keeps_i16_boundaries_and_rejects_invalid_weights(self) -> None:
        # 補間計画の1/8cp単位のi16形式から直接得られる保存境界。
        weights = np.array([self.LOWER_CP, 0.0, self.UPPER_CP], dtype=np.float32)
        np.testing.assert_array_equal(quantize(weights), [-32768, 0, 32767])
        for value in (-4096.125, 4096.0):
            with self.subTest(value=value), self.assertRaises(OverflowError):
                quantize(np.array([value], dtype=np.float32))
        for value in (np.nan, np.inf, -np.inf):
            with self.subTest(value=value), self.assertRaises(ValueError):
                quantize(np.array([value], dtype=np.float32))


if __name__ == "__main__":
    unittest.main()
