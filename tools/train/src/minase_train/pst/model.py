"""線形PSTのモデルと勝率ロジットを計算する。"""

from __future__ import annotations

import numpy as np
import torch
from torch import Tensor, nn

from minase_train.data.features import (
    FEATURE_COUNT,
    MIRRORED_FEATURE_COUNT,
    PADDING_INDEX,
    canonical_feature_indices,
)


MODEL_KINDS = ("single", "tapered", "mirrored")


class MirroredEmbedding(nn.Embedding):
    """左右の鏡映対が序中盤・終盤の2端点を共有するPSTモデル。"""

    def __init__(self, initial: Tensor, device: torch.device) -> None:
        super().__init__(
            MIRRORED_FEATURE_COUNT + 1, 2,
            padding_idx=MIRRORED_FEATURE_COUNT, device=device,
        )
        self.register_buffer(
            "canonical_indices",
            torch.as_tensor(
                canonical_feature_indices(np.arange(FEATURE_COUNT + 1)),
                dtype=torch.long, device=device,
            ),
        )
        tables = initial.reshape(-1, 12, 12, 2)
        shared = (tables[:, :, :6] + tables[:, :, 6:].flip(2)) / 2.0
        with torch.no_grad():
            self.weight.zero_()
            self.weight[:MIRRORED_FEATURE_COUNT].copy_(shared.reshape(-1, 2))

    def forward(self, indices: Tensor) -> Tensor:
        return super().forward(self.canonical_indices[indices.long()])


def make_model(initial: Tensor, device: torch.device, kind: str) -> nn.Module:
    """指定種別と初期値(特徴数×端点数)からpadding行付き線形PSTモデルを作る。"""
    if kind not in MODEL_KINDS:
        raise ValueError(f"unknown model kind: {kind}")
    columns = 1 if kind == "single" else 2
    if initial.shape != (FEATURE_COUNT, columns):
        raise ValueError(f"{kind} initial weights must have shape ({FEATURE_COUNT}, {columns})")
    if kind == "mirrored":
        model = MirroredEmbedding(initial, device)
    else:
        model = nn.Embedding(FEATURE_COUNT + 1, columns, padding_idx=PADDING_INDEX, device=device)
        with torch.no_grad():
            model.weight.zero_()
            model.weight[:FEATURE_COUNT].copy_(initial)
    return model


def expanded_model_weights(model: nn.Module) -> Tensor:
    """PSTを全13,680特徴へ展開する。"""
    if isinstance(model, MirroredEmbedding):
        return model.weight[model.canonical_indices[:FEATURE_COUNT]]
    return model.weight[:FEATURE_COUNT]


def phase_weights(phi: Tensor, columns: int) -> Tensor:
    """列ごとの補間係数を返す。単一PSTは1、2端点は(φ, 1−φ)。"""
    if columns == 1:
        return torch.ones((phi.shape[0], 1), dtype=phi.dtype, device=phi.device)
    if columns == 2:
        return torch.stack((phi, 1.0 - phi), dim=1)
    raise ValueError("model must have 1 or 2 columns")


def model_logits(model: nn.Module, features: Tensor, phi: Tensor, k: float) -> Tensor:
    """特徴番号バッチと補間係数から勝率ロジットを計算する。"""
    sums = model(features).sum(dim=1)
    return (sums * phase_weights(phi, sums.shape[1])).sum(dim=1) / k
