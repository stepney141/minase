"""MNPTの形式、駒価値、および重みの量子化を検証する。"""

from __future__ import annotations

import hashlib
import struct
import tempfile
import unittest
from pathlib import Path

import numpy as np

from helpers import PIECE_VALUES
from minase_train.data.features import FEATURE_COUNT
from minase_train.data.mnpt import (
    HEADER_LENGTH as MNPT_HEADER_LENGTH,
    quantize,
    read_mnpt,
    read_mnpt_v3,
    validate_fixed_base,
    write_mnpt,
    write_mnpt_v3,
)


class TaperedFormatTest(unittest.TestCase):

    """MNPT版2・版3の配置、完全検証、およびPST部分の読み込みを検証する。"""

    def test_mnpt_corruption_and_inconsistent_piece_values_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "weights.bin"
            weights = np.arange(FEATURE_COUNT, dtype=np.int16)
            write_mnpt(path, weights, -weights, PIECE_VALUES, 300.0)
            self.assertEqual(path.stat().st_size, 80 + 13_680 * 2 * 2 + 47 * 4)
            middlegame, endgame, values, k = read_mnpt(path)
            np.testing.assert_array_equal(middlegame, weights)
            np.testing.assert_array_equal(endgame, -weights)
            np.testing.assert_array_equal(values, PIECE_VALUES)
            self.assertEqual(k, 300.0)
            original = path.read_bytes()
            corruptions = {
                "header": original[:79],
                "length": original[:-1],
                "extra byte": original + b"x",
                "magic": b"MNPX" + original[4:],
                "version": original[:4] + struct.pack("<I", 1) + original[8:],
                "future version": original[:4] + struct.pack("<I", 4) + original[8:],
                "features": original[:8] + struct.pack("<I", 42) + original[12:],
                "checksum": original[:MNPT_HEADER_LENGTH] + bytes([original[MNPT_HEADER_LENGTH] ^ 1]) + original[MNPT_HEADER_LENGTH + 1:],
            }
            for name, content in corruptions.items():
                if name in ("length", "extra byte"):
                    content = content[:48] + hashlib.sha256(content[80:]).digest() + content[80:]
                with self.subTest(name=name):
                    path.write_bytes(content)
                    with self.assertRaises(ValueError) as error:
                        read_mnpt(path)
                    self.assertIn(str(path), str(error.exception))
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

    def test_v3_readers_preserve_pst_and_validate_rank_dependent_length(self) -> None:
        """階数が異なっても両APIの戻り値を保ち、過不足のある本体は拒否する。"""
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fm.bin"
            weights = np.arange(FEATURE_COUNT, dtype=np.int16)
            fixed_length = 80 + 13_680 * 2 * 2 + 47 * 4
            for rank in (1, 3):
                with self.subTest(rank=rank):
                    embeddings = np.tile(np.arange(rank, dtype=np.int16) - 2,
                                         (FEATURE_COUNT, 1))
                    signs = np.array([1 if i % 2 == 0 else -1 for i in range(rank)], dtype=np.int8)
                    write_mnpt_v3(path, weights, -weights, PIECE_VALUES, 1000.125,
                                  embeddings, signs, 7)
                    self.assertEqual(path.stat().st_size, fixed_length + 8 + rank + 13_680 * rank * 2)
                    expected = (weights, -weights, PIECE_VALUES, 1000.125, embeddings, signs, 7)
                    for reader, count in ((read_mnpt, 4), (read_mnpt_v3, 7)):
                        decoded = reader(path)
                        self.assertEqual(len(decoded), count)
                        for actual, value in zip(decoded, expected[:count]):
                            np.testing.assert_array_equal(actual, value)
                    original = path.read_bytes()
                    corruptions = {
                        "short header": original[:79],
                        "short PST": original[:fixed_length - 1],
                        "missing rank and exponent": original[:fixed_length],
                        "short rank": original[:fixed_length + 3],
                        "missing exponent": original[:fixed_length + 4],
                        "short exponent": original[:fixed_length + 7],
                        "short embeddings": original[:-1],
                        "extra byte": original + b"x",
                    }
                    for name, content in corruptions.items():
                        # 長さの違反を、検査和の不一致とは独立に検証する。
                        if len(content) >= 80:
                            content = content[:48] + hashlib.sha256(content[80:]).digest() + content[80:]
                        path.write_bytes(content)
                        for reader in (read_mnpt, read_mnpt_v3):
                            with self.subTest(name=name, reader=reader.__name__), self.assertRaises(ValueError) as error:
                                reader(path)
                            self.assertIn(str(path), str(error.exception))

    def test_v3_reader_rejects_valid_v2(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "pst.bin"
            weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
            write_mnpt(path, weights, weights, PIECE_VALUES, 300.0)
            with self.assertRaises(ValueError) as error:
                read_mnpt_v3(path)
            self.assertIn(str(path), str(error.exception))

    def test_fixed_base_accepts_v3_and_compares_only_pst_fields(self) -> None:
        """基準が版3でもFMの変更を許し、PSTの両端点・駒価値・Kは固定する。"""
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory) / "base.bin"
            candidate = Path(directory) / "candidate.bin"
            weights = np.arange(FEATURE_COUNT, dtype=np.int16)
            embeddings = np.zeros((FEATURE_COUNT, 1), dtype=np.int16)
            write_mnpt_v3(base, weights, -weights, PIECE_VALUES, 300.0,
                          embeddings, np.array([1]), 0)
            write_mnpt_v3(candidate, weights, -weights, PIECE_VALUES, 300.0,
                          embeddings + 1, np.array([-1]), 1)
            validate_fixed_base(base, candidate)
            values = PIECE_VALUES.copy()
            values[0] += 1
            for name, mg, eg, piece_values, k in (
                ("middlegame", weights + 1, -weights, PIECE_VALUES, 300.0),
                ("endgame", weights, -weights + 1, PIECE_VALUES, 300.0),
                ("piece values", weights, -weights, values, 300.0),
                ("K", weights, -weights, PIECE_VALUES, 301.0),
            ):
                write_mnpt_v3(candidate, mg, eg, piece_values, k, embeddings, np.array([1]), 0)
                with self.subTest(name=name), self.assertRaises(ValueError):
                    validate_fixed_base(base, candidate)


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
