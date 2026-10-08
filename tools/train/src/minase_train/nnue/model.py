"""鏡映共有の線形PSTとSCReLUの1層ネットを同時に学習する。"""

from __future__ import annotations

import math
from pathlib import Path

import numpy as np
from numpy.typing import NDArray
import torch
from torch import Tensor, nn
from torch.nn import functional as F

from minase_train.data.features import (
    FEATURE_COUNT, MIRRORED_FEATURE_COUNT, PADDING_INDEX, feature_indices,
)
from minase_train.data.taper import phase_ratios
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


class NNUE(nn.Module):
    """正準特徴の重みと半数のバイアス・出力重みで、鏡映共有を表現する。"""

    def __init__(self, initial: NDArray, observed: NDArray, width: int, k: float,
                 device: torch.device, seed: int) -> None:
        super().__init__()
        validate_width(width, "NNUE model")
        _validate_initial(np.asarray(initial), "NNUE model")
        observed = np.asarray(observed)
        if observed.shape != (FEATURE_COUNT,) or observed.dtype != np.bool_:
            raise ValueError("NNUE model: observed mask must be bool[13680]")
        squares = observed.reshape(95, 12, 12)
        if not np.array_equal(squares, squares[:, :, ::-1]):
            raise ValueError("NNUE model: observed mask must share mirrors")
        if not math.isfinite(k) or not 0 < k <= 100_000:
            raise ValueError("NNUE model: K must be finite and in (0, 100000]")
        self.width, self.k = width, k
        self.linear = MirroredEmbedding(torch.as_tensor(initial, device=device), device)
        self.embedding = nn.Embedding(MIRRORED_FEATURE_COUNT + 1, width,
                                      padding_idx=MIRRORED_FEATURE_COUNT, device=device)
        self.biases = nn.Parameter(torch.zeros(width // 2, device=device))
        self.output_weights = nn.Parameter(torch.zeros(2, width // 2, device=device))
        self.output_bias = nn.Parameter(torch.zeros((), device=device))
        self.register_buffer("observed", torch.as_tensor(observed, device=device))
        self.register_buffer("canonical_observed", torch.as_tensor(
            np.append(squares[:, :, :6].reshape(-1), False), device=device))
        self.register_buffer("initial_linear", self.linear.weight.detach().clone())
        self.register_buffer("reflected", torch.as_tensor(
            np.arange(FEATURE_COUNT + 1) % 12 >= 6, device=device))
        generator = torch.Generator(device=device).manual_seed(seed)
        with torch.no_grad():
            self.embedding.weight.zero_()
            count = int(self.canonical_observed.sum())
            self.embedding.weight[self.canonical_observed] = torch.randn(
                (count, width), generator=generator, device=device) * 0.05
            self.fix_unobserved()

    def nonlinear_parameters(self) -> list[nn.Parameter]:
        """非線形経路を1つの最適化群として返す。"""
        return [self.embedding.weight, self.biases, self.output_weights, self.output_bias]

    def expanded_weights(self) -> Tensor:
        """鏡映側で隠れユニットの前半と後半を交換して全特徴へ展開する。"""
        rows = self.embedding.weight[self.linear.canonical_indices]
        return torch.where(self.reflected[:, None], rows.roll(self.width // 2, dims=1), rows)

    def feature_weights(self, features: Tensor) -> Tensor:
        """padding行を含む非線形の特徴重みを集める。"""
        return F.embedding(features.long(), self.expanded_weights(), padding_idx=PADDING_INDEX)

    def _output(self, hidden: Tensor, perspective: int) -> Tensor:
        activation = hidden.clamp(0, 1).square()
        left, right = activation.chunk(2, dim=-1)
        return ((left + right) * self.output_weights[perspective]).sum(dim=-1)

    def components(self, features_stm: Tensor, features_nstm: Tensor,
                   ratios: Tensor) -> tuple[Tensor, Tensor, Tensor]:
        """線形のセンチポーン、非線形ロジット、および両視点の累算器を返す。"""
        sums = _feature_sum(self.linear(features_stm))
        linear_cp = ratios * sums[:, 0] + (1 - ratios) * sums[:, 1]
        bias = self.biases.repeat(2)
        hidden = torch.stack([_feature_sum(self.feature_weights(features)) + bias
                              for features in (features_stm, features_nstm)], dim=1)
        nl = self._output(hidden[:, 0], 0) + self._output(hidden[:, 1], 1) + self.output_bias
        return linear_cp, nl, hidden

    def forward(self, features_stm: Tensor, features_nstm: Tensor, ratios: Tensor) -> Tensor:
        """固定した出力用Kで線形評価を割り、非線形ロジットを足す。"""
        linear_cp, nl, _ = self.components(features_stm, features_nstm, ratios)
        return linear_cp / self.k + nl

    def removal_deltas(self, features_stm: Tensor, features_nstm: Tensor, counts: Tensor) -> Tensor:
        """先獅子を残して各升の特徴だけを両視点から引いた除去差分を返す。"""
        delta = pst_removal_deltas(self.linear(features_stm), counts)
        for perspective, features in enumerate((features_stm, features_nstm)):
            rows = self.feature_weights(features)
            hidden = _feature_sum(rows) + self.biases.repeat(2)
            before = self._output(hidden, perspective)
            after = self._output(hidden[:, None, :] - rows[:, :144], perspective)
            # 大きな(B, 144, H)配列はモデルの精度のまま、集約後だけ倍精度へ広げる。
            delta = delta + self.k * (after - before[:, None]).to(torch.float64)
        return delta

    @torch.no_grad()
    def fix_unobserved(self) -> None:
        """未観測行の線形初期値と非線形の0、およびpaddingの0を厳密に保つ。"""
        self.embedding.weight[~self.canonical_observed] = 0
        self.linear.weight[~self.canonical_observed] = self.initial_linear[~self.canonical_observed]

    @torch.no_grad()
    def constrain(self) -> None:
        """更新後の重みを保存形式の範囲へ制限し、未観測行を復元する。"""
        if any(not bool(torch.isfinite(value).all()) for value in self.parameters()):
            raise ValueError("NNUE model: non-finite trained weights")
        self.linear.weight.clamp_(-4096, 4095.875)
        self.embedding.weight.clamp_(-8, 8)
        self.biases.clamp_(-8, 8)
        self.output_weights.clamp_(-1.98, 1.98)
        self.output_bias.clamp_(-8, 8)
        self.fix_unobserved()

    @torch.no_grad()
    def floating_weights(self) -> dict[str, NDArray]:
        """量子化前の全特徴の重みを、npz用の単精度配列として取り出す。"""
        linear = expanded_model_weights(self.linear).cpu().numpy().astype(np.float32)
        return {
            "middlegame": linear[:, 0], "endgame": linear[:, 1],
            "weights": self.expanded_weights()[:FEATURE_COUNT].cpu().numpy().astype(np.float32),
            "biases": self.biases.repeat(2).cpu().numpy().astype(np.float32),
            "output_weights": self.output_weights.repeat(1, 2).cpu().numpy().astype(np.float32),
            "output_bias": self.output_bias.cpu().numpy().astype(np.float32),
            "mask": self.observed.cpu().numpy().copy(),
        }


def float_evaluate(weights: dict[str, NDArray], k: float, board: NDArray,
                   stm: NDArray, lion: NDArray) -> NDArray[np.float64]:
    """量子化検査用に、線形評価と非線形出力を倍精度で再計算する。"""
    width = weights["weights"].shape[1]
    linear = np.zeros((FEATURE_COUNT + 1, 2), dtype=np.float64)
    linear[:FEATURE_COUNT] = np.stack((weights["middlegame"], weights["endgame"]), axis=1)
    table = np.zeros((FEATURE_COUNT + 1, width), dtype=np.float64)
    table[:FEATURE_COUNT] = weights["weights"]
    result = np.empty(len(board), dtype=np.float64)
    for start in range(0, len(board), REFERENCE_BATCH):
        chunk = slice(start, start + REFERENCE_BATCH)
        features = [feature_indices(board[chunk], side[chunk], lion[chunk])
                    for side in (stm, 1 - np.asarray(stm))]
        sums = linear[features[0]].sum(axis=1)
        ratios = phase_ratios(board[chunk])
        cp = ratios * sums[:, 0] + (1 - ratios) * sums[:, 1]
        nl = np.full(len(cp), float(weights["output_bias"]), dtype=np.float64)
        for perspective, indices in enumerate(features):
            hidden = table[indices].sum(axis=1) + weights["biases"]
            nl += (np.clip(hidden, 0, 1)**2 * weights["output_weights"][perspective]).sum(axis=1)
        result[chunk] = cp + k * nl
    return result
