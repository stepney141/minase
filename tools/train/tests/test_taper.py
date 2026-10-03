"""駒数による補間係数と端点の識別性を検証する。"""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import numpy as np

from helpers import write_mnsd
from minase_train.data.features import INITIAL_BOARD, PADDING_INDEX, feature_indices
from minase_train.data.mnsd import Dataset
from minase_train.data.taper import (
    band_indices,
    feature_identifiability,
    phase_numerators,
    phase_ratios,
    piece_counts,
)
from minase_train.diagnostics.taper_report import state_summary


class MirroredIdentifiabilityTest(unittest.TestCase):

    def test_mirror_pairs_pool_phase_observations_before_computing_ssd(self) -> None:
        # strength-stage7.md: 正準特徴の単位で出現回数とφの偏差平方和を集計する。
        # 同じ段の両端の歩をφ=0とφ=1で1回ずつ観測する。
        boards = np.zeros((2, 144), dtype=np.uint8)
        boards[0, :2] = [1, 12]
        boards[1, :92] = 11
        boards[1, 11] = 1
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "observations.bin"
            write_mnsd(path, seed=0, checksum=b"a" * 32, games=[1, 2], board=boards)
            dataset = Dataset([path])
            indices = np.arange(2, dtype=np.int64)
            full = feature_identifiability(dataset, indices, mirrored=False, batch=1)
            pooled = feature_identifiability(dataset, indices, mirrored=True, batch=1)
        self.assertEqual(full["count"].size, 13680)
        self.assertEqual(pooled["count"].size, 6840)
        self.assertEqual(int(pooled["count"].sum()), 94)
        pawn_full = 29 * 144
        pawn_pooled = 29 * 72
        self.assertEqual(full["ssd"][pawn_full], 0)
        self.assertEqual(full["ssd"][pawn_full + 11], 0)
        self.assertEqual(pooled["count"][pawn_pooled], 2)
        self.assertEqual(pooled["mean_phi"][pawn_pooled], 0.5)
        self.assertEqual(pooled["ssd"][pawn_pooled], 0.5)
        pawn = state_summary(pooled, 72)[29]
        self.assertEqual(pawn["observed_squares"], 1)
        self.assertEqual(pawn["occurrences"], 2)
        self.assertEqual(pawn["mean_phi"], 0.5)


class TaperedFormatTest(unittest.TestCase):

    """補間係数、整数評価の式、MNPTバージョン2の検査、およびモデル種別の契約を検証する。"""

    def test_phase_numerator_counts_board_pieces_only(self) -> None:
        boards = np.zeros((4, 144), dtype=np.uint8)
        boards[0] = INITIAL_BOARD  # 92枚
        boards[1, :2] = 12  # 王2枚
        boards[2, :47] = 1  # 47枚 → q=45
        boards[3, :] = 1  # 144枚 → 上限
        np.testing.assert_array_equal(piece_counts(boards), [92, 2, 47, 144])
        np.testing.assert_array_equal(phase_numerators(boards), [90, 0, 45, 90])
        np.testing.assert_array_equal(phase_ratios(boards), [1.0, 0.0, 0.5, 1.0])
        # 先獅子の対象升は特徴だが駒ではないので、局面の駒数に影響しない。
        features = feature_indices(boards[:1], np.zeros(1, dtype=np.uint8), np.array([5], dtype=np.uint8))
        self.assertEqual(int((features != PADDING_INDEX).sum()), 93)
        np.testing.assert_array_equal(
            band_indices(np.array([0.0, 0.19, 0.2, 0.4, 0.6, 0.8, 0.99, 1.0])), [0, 0, 1, 2, 3, 4, 4, 4]
        )


if __name__ == "__main__":
    unittest.main()
