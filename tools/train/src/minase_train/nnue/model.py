"""線形経路、特徴変換部、および直接内積の出力部を組み合わせる。"""

from __future__ import annotations

import math
from pathlib import Path

import numpy as np
from numpy.typing import NDArray
import torch
from torch import Tensor, nn
from torch.nn import functional as F

from minase_train.data.features import (
    FEATURE_COUNT, MIRRORED_FEATURE_COUNT, PADDING_INDEX, canonical_feature_indices,
)
from minase_train.data.taper import phase_ratios
from minase_train.nnue.feature_sets import FeatureSetSpec
from minase_train.nnue.format import validate_width
from minase_train.nnue.reference import REFERENCE_BATCH
from minase_train.pst.model import MirroredEmbedding, expanded_model_weights
from minase_train.pst.removal import pst_removal_deltas


def load_initial(path: str | Path) -> NDArray[np.float32]:
    """npzの2端点の型、範囲、有限性、および鏡映対の一致を検査する。"""
    with np.load(path, allow_pickle=False) as archive:
        if not all(name in archive for name in ("middlegame", "endgame")):
            raise ValueError(f"{path}: middlegame and endgame are required")
        columns = [archive[name] for name in ("middlegame", "endgame")]
    if any(column.shape != (FEATURE_COUNT,) or column.dtype != np.float32 for column in columns):
        raise ValueError(f"{path}: PST endpoints must be float32 vectors of length 13680")
    initial = np.stack(columns, axis=1)
    _validate_initial(initial, str(path))
    return initial


def _validate_initial(initial: NDArray, source: str) -> None:
    if (initial.shape != (FEATURE_COUNT, 2) or not np.isfinite(initial).all()
            or np.any(initial < -4096) or np.any(initial > 4095.875)):
        raise ValueError(f"{source}: initial PST must have shape (13680, 2) and lie in -4096..4095.875")
    squares = initial.reshape(95, 12, 12, 2)
    if not np.array_equal(squares, squares[:, :, ::-1]):
        raise ValueError(f"{source}: initial PST mirror sharing violated")


def _feature_sum(rows: Tensor) -> Tensor:
    """左右の対応升を先に足し、鏡映で浮動小数点の加算順序が変わらない和を作る。"""
    board = rows[:, :144].reshape(len(rows), 12, 12, rows.shape[-1])
    pairs = board[:, :, :6] + board[:, :, 6:].flip(2)
    return pairs.sum(dim=(1, 2)) + rows[:, 144]


class LinearPath(MirroredEmbedding):
    """基底特徴の鏡映対を共有し、未観測行には移植値を保つ。"""

    def __init__(self, initial: NDArray, observed: NDArray, device: torch.device) -> None:
        super().__init__(torch.as_tensor(initial, device=device), device)
        self.register_buffer("observed", torch.as_tensor(observed, device=device))
        self.register_buffer("canonical_observed", torch.as_tensor(
            np.append(observed.reshape(95, 12, 12)[:, :, :6].reshape(-1), False), device=device))
        self.register_buffer("initial", self.weight.detach().clone())

    @torch.no_grad()
    def fix_unobserved(self) -> None:
        self.weight[~self.canonical_observed] = self.initial[~self.canonical_observed]


class SharedTransformer(nn.Module):
    """P型の正準行を持ち、鏡映側ではユニットの前半と後半を交換する。"""

    def __init__(self, observed: NDArray, width: int, device: torch.device, seed: int) -> None:
        super().__init__()
        self.width = width
        self.embedding = nn.Embedding(MIRRORED_FEATURE_COUNT + 1, width,
                                      padding_idx=MIRRORED_FEATURE_COUNT, device=device)
        self.register_buffer("canonical_indices", torch.as_tensor(
            canonical_feature_indices(np.arange(FEATURE_COUNT + 1)), dtype=torch.long, device=device))
        self.register_buffer("reflected", torch.as_tensor(
            np.arange(FEATURE_COUNT + 1) % 12 >= 6, device=device))
        self.register_buffer("observed", torch.as_tensor(
            np.append(observed.reshape(95, 12, 12)[:, :, :6].reshape(-1), False), device=device))
        generator = torch.Generator(device=device).manual_seed(seed)
        with torch.no_grad():
            self.embedding.weight.zero_()
            count = int(self.observed.sum())
            self.embedding.weight[self.observed] = torch.randn(
                (count, width), generator=generator, device=device) * 0.05

    def expanded_weights(self) -> Tensor:
        rows = self.embedding.weight[self.canonical_indices]
        return torch.where(self.reflected[:, None], rows.roll(self.width // 2, dims=1), rows)

    def forward(self, features: Tensor) -> Tensor:
        return F.embedding(features.long(), self.expanded_weights(), padding_idx=PADDING_INDEX)

    @torch.no_grad()
    def fix_unobserved(self) -> None:
        self.embedding.weight[~self.observed] = 0

    @torch.no_grad()
    def constrain(self) -> None:
        self.embedding.weight.clamp_(-8, 8)


class FactorizedTransformer(nn.Module):
    """特徴の行と基底特徴の仮想行を足し、保存時だけ全行を畳み込む。"""

    def __init__(self, spec: FeatureSetSpec, observed: NDArray, base_observed: NDArray,
                 width: int, device: torch.device, seed: int) -> None:
        super().__init__()
        self.embedding = nn.Embedding(spec.feature_count + 1, width,
                                      padding_idx=spec.feature_count, device=device)
        self.virtual = nn.Embedding(FEATURE_COUNT + 1, width, padding_idx=FEATURE_COUNT, device=device)
        self.register_buffer("observed", torch.as_tensor(np.append(observed, False), device=device))
        self.register_buffer("virtual_observed", torch.as_tensor(
            np.append(observed.reshape(-1, FEATURE_COUNT).any(axis=0), False), device=device))
        self.register_buffer("virtual_indices", torch.as_tensor(
            spec.base_indices(np.arange(spec.feature_count + 1)), dtype=torch.long, device=device))
        initial = SharedTransformer(base_observed, width, device, seed).expanded_weights().detach()
        self.register_buffer("initial_virtual", initial.clone())
        with torch.no_grad():
            self.embedding.weight.zero_()
            self.virtual.weight.copy_(initial)

    def forward(self, features: Tensor) -> Tensor:
        features = features.long()
        return self.embedding(features) + self.virtual(self.virtual_indices[features])

    def expanded_weights(self) -> Tensor:
        return self.embedding.weight + self.virtual.weight[self.virtual_indices]

    @torch.no_grad()
    def fix_unobserved(self) -> None:
        self.embedding.weight[~self.observed] = 0
        self.virtual.weight[~self.virtual_observed] = self.initial_virtual[~self.virtual_observed]

    @torch.no_grad()
    def constrain(self) -> None:
        self.virtual.weight.clamp_(-8, 8)
        virtual = self.virtual.weight[self.virtual_indices]
        self.embedding.weight.clamp_(min=-8 - virtual, max=8 - virtual)


class DirectHead(nn.Module):
    """第1層のバイアスと、両視点のSCReLUへの直接内積を持つ。"""

    def __init__(self, width: int, shared: bool, device: torch.device) -> None:
        super().__init__()
        self.shared = shared
        units = width // 2 if shared else width
        self.biases = nn.Parameter(torch.zeros(units, device=device))
        self.output_weights = nn.Parameter(torch.zeros(2, units, device=device))
        self.output_bias = nn.Parameter(torch.zeros((), device=device))

    def expanded_biases(self) -> Tensor:
        return self.biases.repeat(2) if self.shared else self.biases

    def expanded_output_weights(self) -> Tensor:
        return self.output_weights.repeat(1, 2) if self.shared else self.output_weights

    def forward(self, hidden: Tensor, perspective: int) -> Tensor:
        activation = hidden.clamp(0, 1).square()
        if self.shared:
            left, right = activation.chunk(2, dim=-1)
            activation = left + right
        return (activation * self.output_weights[perspective]).sum(dim=-1)

    @torch.no_grad()
    def constrain(self) -> None:
        self.biases.clamp_(-8, 8)
        self.output_weights.clamp_(-1.98, 1.98)
        self.output_bias.clamp_(-8, 8)


# 各視点の基底番号と入力特徴番号は、どちらも元の升順の145列を持つ。
FeatureInputs = tuple[Tensor, Tensor]


class NNUE(nn.Module):
    """入力特徴集合に従って線形経路、特徴変換部、出力部を組み立てる。"""

    def __init__(self, initial: NDArray, observed: NDArray, width: int, k: float,
                 device: torch.device, seed: int, spec: FeatureSetSpec,
                 feature_observed: NDArray) -> None:
        super().__init__()
        validate_width(width, "NNUE model")
        _validate_initial(np.asarray(initial), "NNUE model")
        observed = np.asarray(observed)
        if observed.shape != (FEATURE_COUNT,) or observed.dtype != np.bool_:
            raise ValueError("NNUE model: observed mask must be bool[13680]")
        squares = observed.reshape(95, 12, 12)
        if not np.array_equal(squares, squares[:, :, ::-1]):
            raise ValueError("NNUE model: observed mask must share mirrors")
        feature_observed = np.asarray(feature_observed)
        if feature_observed.shape != (spec.feature_count,) or feature_observed.dtype != np.bool_:
            raise ValueError(f"NNUE model: feature mask must be bool[{spec.feature_count}]")
        if not math.isfinite(k) or not 0 < k <= 100_000:
            raise ValueError("NNUE model: K must be finite and in (0, 100000]")
        self.width, self.k, self.spec = width, k, spec
        self.linear = LinearPath(initial, observed, device)
        self.ft = (FactorizedTransformer(spec, feature_observed, observed, width, device, seed)
                   if spec.factorized else SharedTransformer(observed, width, device, seed))
        self.head = DirectHead(width, spec.mirror == "shared", device)

    def components(self, stm: FeatureInputs, nstm: FeatureInputs,
                   ratios: Tensor) -> tuple[Tensor, Tensor, Tensor]:
        """線形のセンチポーン、非線形ロジット、および両視点の累算器を返す。"""
        sums = _feature_sum(self.linear(stm[0]))
        linear_cp = ratios * sums[:, 0] + (1 - ratios) * sums[:, 1]
        bias = self.head.expanded_biases()
        hidden = torch.stack([_feature_sum(self.ft(features)) + bias
                              for _, features in (stm, nstm)], dim=1)
        nl = self.head(hidden[:, 0], 0) + self.head(hidden[:, 1], 1) + self.head.output_bias
        return linear_cp, nl, hidden

    def forward(self, stm: FeatureInputs, nstm: FeatureInputs, ratios: Tensor) -> Tensor:
        """固定した出力用Kで線形評価を割り、非線形ロジットを足す。"""
        linear_cp, nl, _ = self.components(stm, nstm, ratios)
        return linear_cp / self.k + nl

    def removal_deltas(self, stm: FeatureInputs, nstm: FeatureInputs, counts: Tensor) -> Tensor:
        """文脈を固定し、先獅子を残して各升の行を両視点から引く。"""
        delta = pst_removal_deltas(self.linear(stm[0]), counts)
        for perspective, (_, features) in enumerate((stm, nstm)):
            rows = self.ft(features)
            hidden = _feature_sum(rows) + self.head.expanded_biases()
            before = self.head(hidden, perspective)
            after = self.head(hidden[:, None, :] - rows[:, :144], perspective)
            # 大きな(B, 144, H)配列はモデルの精度のまま、集約後だけ倍精度へ広げる。
            delta = delta + self.k * (after - before[:, None]).to(torch.float64)
        return delta

    @torch.no_grad()
    def constrain(self) -> None:
        """更新後に保存範囲へ切り詰め、未観測行とpaddingを復元する。"""
        if any(not bool(torch.isfinite(value).all()) for value in self.parameters()):
            raise ValueError("NNUE model: non-finite trained weights")
        self.linear.weight.clamp_(-4096, 4095.875)
        self.ft.constrain()
        self.head.constrain()
        self.linear.fix_unobserved()
        self.ft.fix_unobserved()

    @torch.no_grad()
    def floating_weights(self) -> dict[str, NDArray]:
        """特徴分解を畳み込み、保存用の単精度配列を取り出す。"""
        linear = expanded_model_weights(self.linear).cpu().numpy().astype(np.float32)
        return {
            "middlegame": linear[:, 0], "endgame": linear[:, 1],
            "weights": self.ft.expanded_weights()[:self.spec.feature_count].cpu().numpy().astype(np.float32),
            "biases": self.head.expanded_biases().cpu().numpy().astype(np.float32),
            "output_weights": self.head.expanded_output_weights().cpu().numpy().astype(np.float32),
            "output_bias": self.head.output_bias.cpu().numpy().astype(np.float32),
            "mask": self.linear.observed.cpu().numpy().copy(),
        }


def float_evaluate(weights: dict[str, NDArray], k: float, board: NDArray,
                   stm: NDArray, lion: NDArray, spec: FeatureSetSpec) -> NDArray[np.float64]:
    """量子化検査用に、線形評価と非線形出力を倍精度で再計算する。"""
    width = weights["weights"].shape[1]
    linear = np.zeros((FEATURE_COUNT + 1, 2), dtype=np.float64)
    linear[:FEATURE_COUNT] = np.stack((weights["middlegame"], weights["endgame"]), axis=1)
    table = np.zeros((spec.feature_count + 1, width), dtype=np.float64)
    table[:spec.feature_count] = weights["weights"]
    result = np.empty(len(board), dtype=np.float64)
    for start in range(0, len(board), REFERENCE_BATCH):
        chunk = slice(start, start + REFERENCE_BATCH)
        features = [spec.indices(board[chunk], lion[chunk], side[chunk])
                    for side in (stm, 1 - np.asarray(stm))]
        sums = linear[features[0][0]].sum(axis=1)
        ratios = phase_ratios(board[chunk])
        cp = ratios * sums[:, 0] + (1 - ratios) * sums[:, 1]
        nl = np.full(len(cp), float(weights["output_bias"]), dtype=np.float64)
        for perspective, (_, indices) in enumerate(features):
            hidden = table[indices].sum(axis=1) + weights["biases"]
            nl += (np.clip(hidden, 0, 1)**2 * weights["output_weights"][perspective]).sum(axis=1)
        result[chunk] = cp + k * nl
    return result
