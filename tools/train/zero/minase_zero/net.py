"""学習と推論に共通のネット定義と、opset 17のONNXへの出力。"""

import copy
import math
from pathlib import Path
from typing import NamedTuple

import onnx
import torch
from torch import nn
from torch.nn import functional as F

from .encoding import INPUT_PLANES


def validate_ema(period_samples: int, scale: float) -> None:
    """平均の周期と係数は、ともに0か、ともに正の有限値に限る。"""
    if type(period_samples) is not int or period_samples < 0:
        raise ValueError("ema-period-samples must be a nonnegative integer")
    if not math.isfinite(scale) or scale < 0:
        raise ValueError("ema-scale must be finite and nonnegative")
    if (period_samples == 0) != (scale == 0):
        raise ValueError("ema-period-samples and ema-scale must both be zero or both be positive")


class Ema:
    """標本数の周期でパラメータを平均し、バッファは更新時の値を複写する。"""

    def __init__(self, model: nn.Module, period_samples: int, scale: float) -> None:
        validate_ema(period_samples, scale)
        if period_samples == 0:
            raise ValueError("EMA requires a positive period")
        self.model = copy.deepcopy(model).eval().requires_grad_(False)
        self.period_samples = period_samples
        self.scale = scale
        self.samples_since_update = 0
        self.updates = 0

    @torch.no_grad()
    def update(self, model: nn.Module, samples: int) -> None:
        """更新後の重みを、越えた周期の数だけ混ぜ、余りの標本数を残す。"""
        self.samples_since_update += samples
        while self.samples_since_update >= self.period_samples:
            for average, current in zip(self.model.parameters(), model.parameters(), strict=True):
                average.add_((current - average) / self.scale)
            for average, current in zip(self.model.buffers(), model.buffers(), strict=True):
                average.copy_(current)
            self.samples_since_update -= self.period_samples
            self.updates += 1

    def state_dict(self) -> dict:
        """重み、更新後の標本数、および更新回数の累計を保存する。"""
        return {"model": self.model.state_dict(), "samples_since_update": self.samples_since_update,
                "updates": self.updates}

    def load_state_dict(self, state: dict) -> None:
        """サイクルの切り替えと中断からの再開で、同じ平均の状態を復元する。"""
        for name in ("samples_since_update", "updates"):
            if type(state[name]) is not int or state[name] < 0:
                raise ValueError(f"EMA {name} must be a nonnegative integer")
        self.model.load_state_dict(state["model"])
        self.samples_since_update = state["samples_since_update"]
        self.updates = state["updates"]


class ResidualBlock(nn.Module):
    """大域プーリングのバイアスを加える残差ブロック。"""

    def __init__(self, channels: int) -> None:
        super().__init__()
        self.conv1 = nn.Conv2d(channels, channels, 3, padding=1, bias=False)
        self.bn1 = nn.BatchNorm2d(channels)
        self.pool_bias = nn.Linear(2 * channels, channels)
        self.conv2 = nn.Conv2d(channels, channels, 3, padding=1, bias=False)
        self.bn2 = nn.BatchNorm2d(channels)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        """残差接続を通して特徴面を更新する。"""
        y = torch.relu(self.bn1(self.conv1(x)))
        pooled = torch.cat((y.mean(dim=(2, 3)), y.amax(dim=(2, 3))), dim=1)
        y = y + self.pool_bias(pooled)[:, :, None, None]
        return torch.relu(x + self.bn2(self.conv2(y)))


class TrainingLogits(NamedTuple):
    """主方策、4つの価値、および3つの補助方策の学習用logit。"""

    policy: torch.Tensor
    value: torch.Tensor
    soft_policy: torch.Tensor
    opponent_policy: torch.Tensor
    opponent_soft_policy: torch.Tensor


class ZeroNet(nn.Module):
    """159面から方策のlogitと手番側の勝率を返すネット。"""

    def __init__(self, blocks: int, channels: int) -> None:
        super().__init__()
        self.trunk = nn.Sequential(
            nn.Conv2d(INPUT_PLANES, channels, 3, padding=1, bias=False),
            nn.BatchNorm2d(channels),
            nn.ReLU(),
            *(ResidualBlock(channels) for _ in range(blocks)),
        )
        self.policy_head = nn.Conv2d(channels, 249, 1, bias=True)
        self.soft_policy_head = nn.Conv2d(channels, 249, 1, bias=True)
        self.opponent_policy_head = nn.Conv2d(channels, 249, 1, bias=True)
        self.opponent_soft_policy_head = nn.Conv2d(channels, 249, 1, bias=True)
        self.value_head = nn.Sequential(
            nn.Conv2d(channels, 32, 1, bias=False),
            nn.BatchNorm2d(32),
            nn.ReLU(),
            nn.Flatten(1),
            nn.Linear(4608, 256),
            nn.ReLU(),
            nn.Linear(256, 4),
        )

    def forward(self, x: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        """推論用の方策のlogitとsigmoid後の勝率を返す。"""
        x = self.trunk(x)
        value = self.value_head[:-1](x)
        final = self.value_head[-1]
        # ONNXには主の価値を計算する行だけを含める。
        value = F.linear(value, final.weight[:1], final.bias[:1]).sigmoid()
        return self.policy_head(x).flatten(1), value

    def forward_logits(self, x: torch.Tensor) -> TrainingLogits:
        """学習用に主と補助のすべてのlogitを返す。"""
        x = self.trunk(x)
        return TrainingLogits(
            self.policy_head(x).flatten(1), self.value_head(x),
            self.soft_policy_head(x).flatten(1), self.opponent_policy_head(x).flatten(1),
            self.opponent_soft_policy_head(x).flatten(1),
        )


def export_onnx(model: ZeroNet, path: Path) -> None:
    """実現性の測定と参照推論で同じ設定のONNXを書き出し、検査する。"""
    sample = torch.zeros(1, INPUT_PLANES, 12, 12, dtype=torch.float32)
    with torch.inference_mode():
        torch.onnx.export(
            model,
            sample,
            path,
            input_names=["input"],
            output_names=["policy", "value"],
            opset_version=17,
            dynamo=False,
            dynamic_axes={
                "input": {0: "batch"},
                "policy": {0: "batch"},
                "value": {0: "batch"},
            },
        )
    onnx.checker.check_model(str(path))
