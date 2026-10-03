"""整数と実数の重みで局面を参照評価する。"""

from __future__ import annotations

import numpy as np
from numpy.typing import NDArray

from minase_train.data.features import FEATURE_COUNT, INITIAL_BOARD, NO_LION_SQUARE, feature_indices
from minase_train.data.mnpt import EVALUATION_LIMIT
from minase_train.data.taper import PHASE_DIVISOR, phase_numerators


# 設計書「量子化と整数推論」の合格条件: 浮動小数点評価との平均絶対誤差2センチポーン以下。
QUANTIZATION_ERROR_LIMIT = 2.0


def integer_evaluate(
    middlegame: NDArray[np.int16],
    endgame: NDArray[np.int16],
    features: NDArray[np.int32],
    numerators: NDArray[np.int64],
) -> NDArray[np.int32]:
    """設計書「整数評価と差分更新」の式で量子化重みにより局面バッチを評価する。

    numeratorsは局面ごとの q = min(90, max(0, N − 2)) である。
    """
    numerators = np.asarray(numerators, dtype=np.int64)
    if numerators.shape != (features.shape[0],):
        raise ValueError("numerators must have one value per position")
    if np.any(numerators < 0) or np.any(numerators > PHASE_DIVISOR):
        raise ValueError("phase numerator is outside 0..90")
    sums = np.empty((features.shape[0], 2), dtype=np.int64)
    for column, weights in enumerate((middlegame, endgame)):
        extended = np.zeros(FEATURE_COUNT + 1, dtype=np.int64)
        extended[:FEATURE_COUNT] = weights.astype(np.int64)
        sums[:, column] = extended[features].sum(axis=1, dtype=np.int64)
    blended = numerators * sums[:, 0] + (PHASE_DIVISOR - numerators) * sums[:, 1]
    values = np.sign(blended) * (np.abs(blended) // (PHASE_DIVISOR * 8))
    return np.clip(values, -EVALUATION_LIMIT, EVALUATION_LIMIT).astype(np.int32)


def float_evaluate(
    middlegame: NDArray[np.float32],
    endgame: NDArray[np.float32],
    features: NDArray[np.int32],
    phi: NDArray[np.float64],
) -> NDArray[np.float32]:
    """浮動小数点重みで局面バッチを補間評価する。"""
    sums = np.empty((features.shape[0], 2), dtype=np.float32)
    for column, weights in enumerate((middlegame, endgame)):
        extended = np.zeros(FEATURE_COUNT + 1, dtype=np.float32)
        extended[:FEATURE_COUNT] = weights
        sums[:, column] = extended[features].sum(axis=1, dtype=np.float32)
    phi = np.asarray(phi, dtype=np.float32)
    return phi * sums[:, 0] + (1.0 - phi) * sums[:, 1]


def initial_position_score(middlegame: NDArray[np.int16], endgame: NDArray[np.int16]) -> int:
    """量子化重みで中将棋初期局面を先手視点から評価する。"""
    board = INITIAL_BOARD[None, :]
    features = feature_indices(
        board,
        np.array([0], dtype=np.uint8),
        np.array([NO_LION_SQUARE], dtype=np.uint8),
    )
    return int(integer_evaluate(middlegame, endgame, features, phase_numerators(board))[0])
