"""駒除去の参照評価と追加損失を計算する。"""

from __future__ import annotations

import math

import numpy as np
import torch
from numpy.typing import NDArray
from torch import Tensor
from torch.nn import functional as torch_functional

from minase_train.data.features import (
    BOARD_FEATURE_COUNT,
    FEATURE_COUNT,
    PIECE_STATE_COUNT,
    ROYAL_STATES,
)
from minase_train.data.mnpt import EVALUATION_LIMIT
from minase_train.data.taper import PHASE_DIVISOR


REMOVAL_MARGIN_CP = 0.25


def make_removal_reference(
    middlegame: NDArray[np.int16], endgame: NDArray[np.int16], device: torch.device
) -> Tensor:
    """完全鏡映対称の初期MNPTを、padding付きの整数基準表へ変換する。"""
    columns = (np.asarray(middlegame), np.asarray(endgame))
    if any(column.shape != (FEATURE_COUNT,) or column.dtype != np.dtype("int16")
           for column in columns):
        raise ValueError("removal reference endpoints must be i16 arrays of length 13680")
    original = np.stack(columns, axis=1)
    squares = original.reshape(95, 12, 12, 2)
    if not np.array_equal(squares, squares[:, :, ::-1, :]):
        raise ValueError("positive removal penalty requires exactly mirrored initial i16 weights")
    reference = torch.zeros((FEATURE_COUNT + 1, 2), dtype=torch.int64, device=device)
    reference[:FEATURE_COUNT] = torch.as_tensor(original.astype(np.int64), device=device)
    return reference


def removal_loss(
    feature_weights: Tensor, features: Tensor, reference: Tensor, counts: Tensor, k: float
) -> Tensor:
    """元局面等重み・対象除去等重みで、基準符号と悪化上限の片側絶対値損失を返す。"""
    if not math.isfinite(k) or k <= 0.0:
        raise ValueError("K must be finite and positive")
    if features.ndim != 2 or features.shape[1] != 145 or features.shape[0] == 0:
        raise ValueError("removal features must have shape (B, 145) with B > 0")
    if feature_weights.shape != (*features.shape, 2):
        raise ValueError("removal weights must have shape (B, 145, 2)")
    if reference.shape != (FEATURE_COUNT + 1, 2) or reference.dtype != torch.int64:
        raise ValueError("removal reference must be an i64 table of shape (13681, 2)")
    if counts.shape != (features.shape[0],) or counts.dtype != torch.int64:
        raise ValueError("removal counts must be an i64 vector with one value per position")
    if not bool(torch.isfinite(feature_weights).all()):
        raise ValueError("removal weights contain a non-finite value")

    q = (counts - 2).clamp(0, PHASE_DIVISOR)
    after_q = (counts - 3).clamp(0, PHASE_DIVISOR)

    def numerators(weights: Tensor) -> tuple[Tensor, Tensor]:
        sums = weights.sum(dim=1)
        before = q * sums[:, 0] + (PHASE_DIVISOR - q) * sums[:, 1]
        remaining = sums[:, None, :] - weights[:, :144, :]
        after = (after_q[:, None] * remaining[:, :, 0]
                 + (PHASE_DIVISOR - after_q[:, None]) * remaining[:, :, 1])
        return before, after

    # 先獅子と王駒も端点和に残す。基準はi16整数、候補は倍精度で差分を求める。
    base_before, base_after = numerators(reference[features.long()])
    divisor = PHASE_DIVISOR * 8
    integer_before = (base_before.sign() * (base_before.abs() // divisor)).clamp(
        -EVALUATION_LIMIT, EVALUATION_LIMIT
    )
    integer_after = (base_after.sign() * (base_after.abs() // divisor)).clamp(
        -EVALUATION_LIMIT, EVALUATION_LIMIT
    )
    base_sign = (integer_after - integer_before[:, None]).sign()
    base_delta = (base_after - base_before[:, None]).to(torch.float64) / divisor
    before, after = numerators(feature_weights.to(torch.float64))
    signed_delta = base_sign * ((after - before[:, None]) / PHASE_DIVISOR)

    board_features = features[:, :144]
    states = (board_features // 144) % PIECE_STATE_COUNT
    relative_color = board_features // (PIECE_STATE_COUNT * 144)
    eligible = ((board_features < BOARD_FEATURE_COUNT) & (states != ROYAL_STATES[0])
                & (states != ROYAL_STATES[1]) & (base_sign != 0))
    wrong_direction = (((relative_color == 0) & (base_sign > 0))
                       | ((relative_color == 1) & (base_sign < 0)))
    magnitude = base_delta.abs()
    lower = torch_functional.relu(magnitude.clamp(max=REMOVAL_MARGIN_CP) - signed_delta)
    upper = torch_functional.relu(signed_delta - magnitude)
    penalties = lower + wrong_direction * upper
    per_position = torch.where(eligible, penalties, 0.0).sum(dim=1) / eligible.sum(dim=1).clamp_min(1)
    loss = per_position.mean() / k
    if not bool(torch.isfinite(loss)):
        raise ValueError("removal loss is non-finite")
    return loss
