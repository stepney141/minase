"""設計書の整数演算と丸めで、MNUE版5の局面バッチを参照評価する。"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np
from numpy.typing import NDArray

from minase_train.data.features import FEATURE_COUNT
from minase_train.data.mnpt import EVALUATION_LIMIT
from minase_train.data.taper import PHASE_DIVISOR, phase_numerators
from minase_train.nnue.format import Network
from minase_train.nnue.feature_sets import get_feature_set


REFERENCE_BATCH = 1024


@dataclass(frozen=True)
class Evaluation:
    """両視点の累算器、活性化、丸め前後の出力、および最終評価を返す。"""

    linear_accumulators: NDArray[np.int32]
    accumulators: NDArray[np.int32]
    activations: NDArray[np.int32]
    linear_cp: NDArray[np.int64]
    output: NDArray[np.int64]
    cp_nl: NDArray[np.int64]
    evaluation: NDArray[np.int32]


def accumulators(net: Network, board: NDArray, stm: NDArray, lion: NDArray) -> tuple[NDArray, NDArray]:
    """手番側、相手側の順に、paddingが0の線形・非線形累算器を作る。"""
    linear_table = np.zeros((FEATURE_COUNT + 1, 2), dtype=np.int32)
    linear_table[:FEATURE_COUNT] = net.linear
    spec = get_feature_set(net.feature_set)
    table = np.zeros((spec.feature_count + 1, net.width), dtype=np.int32)
    table[:spec.feature_count] = net.weights
    linear = np.empty((len(board), 2, 2), dtype=np.int32)
    hidden = np.empty((len(board), 2, net.width), dtype=np.int32)
    for perspective, side in enumerate((stm, 1 - np.asarray(stm))):
        base, features = spec.indices(board, lion, side)
        for start in range(0, len(board), REFERENCE_BATCH):
            chunk = slice(start, start + REFERENCE_BATCH)
            linear[chunk, perspective] = linear_table[base[chunk]].sum(axis=1, dtype=np.int32)
            hidden[chunk, perspective] = table[features[chunk]].sum(axis=1, dtype=np.int32) + net.biases
    return linear, hidden


def activate(acc: NDArray) -> NDArray[np.int32]:
    """SCReLUの整数近似を0..126の範囲で計算する。"""
    clipped = np.clip(acc, 0, 127).astype(np.int32)
    return (clipped * clipped) >> 7


def output_scale(k: float) -> int:
    """ヘッダのf32を倍精度へ広げてから偶数丸めで尺度を求める。"""
    return round(float(np.float32(k)) * 65_536 / (127 * 4096))


def nonlinear_cp(output: NDArray, k: float) -> NDArray[np.int64]:
    """負数にも算術シフトを使い、中間値を正方向へ丸める。"""
    return (np.asarray(output, dtype=np.int64) * output_scale(k) + 32_768) >> 16


def evaluate_details(net: Network, board: NDArray, stm: NDArray, lion: NDArray) -> Evaluation:
    """整数推論の手順1〜6を実行し、診断用の中間値も返す。"""
    linear, hidden = accumulators(net, board, stm, lion)
    q = phase_numerators(board)
    sums = linear[:, 0].astype(np.int64)
    numerator = q * sums[:, 0] + (PHASE_DIVISOR - q) * sums[:, 1]
    linear_cp = np.sign(numerator) * (np.abs(numerator) // (PHASE_DIVISOR * 8))
    activations = activate(hidden)
    output = (activations.astype(np.int64) * net.output_weights).sum(axis=(1, 2)) + net.output_bias
    if np.any(output < np.iinfo(np.int32).min) or np.any(output > np.iinfo(np.int32).max):
        raise ValueError("NNUE output does not fit i32")
    correction = nonlinear_cp(output, net.k)
    values = np.clip(linear_cp + correction, -EVALUATION_LIMIT, EVALUATION_LIMIT).astype(np.int32)
    return Evaluation(linear, hidden, activations, linear_cp, output, correction, values)


def integer_evaluate(net: Network, board: NDArray, stm: NDArray, lion: NDArray) -> NDArray[np.int32]:
    """最終段だけを±28,999へ制限した整数評価を返す。"""
    return evaluate_details(net, board, stm, lion).evaluation
