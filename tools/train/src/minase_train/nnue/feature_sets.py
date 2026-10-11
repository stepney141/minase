"""入力特徴集合の仕様と、元の升に列を対応させた一括の番号付けを持つ。"""

from __future__ import annotations

from dataclasses import dataclass
from types import MappingProxyType
from typing import Literal

import numpy as np
from numpy.typing import NDArray

from minase_train.data.features import (
    COLOR_BY_BYTE, FEATURE_COUNT, PIECE_STATE_BY_BYTE, SQUARE_BY_PERSPECTIVE,
    feature_indices,
)


@dataclass(frozen=True)
class FeatureSetSpec:
    """名前、特徴数、鏡映の扱い、および特徴分解の有無を定める。"""

    name: str
    feature_count: int
    mirror: Literal["shared", "canonical"]
    factorized: bool

    def indices(self, board: NDArray[np.uint8], lion: NDArray[np.uint8],
                side: NDArray[np.uint8]) -> tuple[NDArray[np.int32], NDArray[np.int32]]:
        """基底特徴と入力特徴を(B, 145)で返し、空の枠には各特徴数を入れる。"""
        base = feature_indices(board, side, lion)
        if self.mirror == "shared":
            return base, base.copy()
        board = np.asarray(board, dtype=np.uint8)
        side = np.asarray(side, dtype=np.uint8)
        states = PIECE_STATE_BY_BYTE[board]
        own = COLOR_BY_BYTE[board] == side[:, None]
        kings = own & (states == 11)
        princes = own & (states == 21)
        royals = np.where(kings.any(axis=1)[:, None], kings, princes)
        present = royals.any(axis=1)
        # argmaxは最初の真を選ぶため、後手でも絶対升の最小値になる。
        square = SQUARE_BY_PERSPECTIVE[side, royals.argmax(axis=1)]
        rank, file = square // 12, square % 12
        reflected = present & (file < 6)
        canonical_file = np.where(reflected, 11 - file, file)
        bucket = np.where(present, np.where(rank == 0, 11 - canonical_file,
                                           np.where(rank == 1, 6, 7)), 0)
        mapped = np.where(reflected[:, None], base // 12 * 12 + 11 - base % 12, base)
        features = np.where(base == FEATURE_COUNT, self.feature_count,
                            bucket[:, None] * FEATURE_COUNT + mapped)
        return base, features.astype(np.int32)

    def base_indices(self, features: NDArray[np.integer]) -> NDArray[np.int32]:
        """発火した特徴を、特徴分解で共有する基底特徴の行へ写す。"""
        features = np.asarray(features)
        return np.where(features == self.feature_count, FEATURE_COUNT,
                        features % FEATURE_COUNT).astype(np.int32)


FEATURE_SETS = MappingProxyType({
    "P": FeatureSetSpec("P", FEATURE_COUNT, "shared", False),
    "HalfKP-hm-b8": FeatureSetSpec("HalfKP-hm-b8", 8 * FEATURE_COUNT, "canonical", True),
})


def get_feature_set(name: str) -> FeatureSetSpec:
    """登録された名前だけを受理する。"""
    try:
        return FEATURE_SETS[name]
    except KeyError:
        raise ValueError(f"unsupported feature set {name!r}") from None
