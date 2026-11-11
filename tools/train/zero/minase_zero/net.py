"""学習と推論に共通のネット定義と、opset 17のONNXへの出力。"""

from pathlib import Path
from typing import NamedTuple

import numpy as np
import onnx
import torch
from torch import nn
from torch.nn import functional as F

from .encoding import INPUT_PLANES, policy_mask


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


class _OnnxExport(nn.Module):
    """ONNXのuint8入力をfloat32へ変換し、可能なラベルの方策だけをFP16で返す。"""

    def __init__(self, model: ZeroNet) -> None:
        super().__init__()
        self.model = model
        self.register_buffer("policy_labels", torch.tensor(np.flatnonzero(policy_mask()), dtype=torch.int64))

    def forward(self, x: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        """0と1の入力を変換し、方策を詰めてからFP16にし、勝率はfloat32で返す。"""
        policy, value = self.model(x.float())
        return policy.index_select(1, self.policy_labels).half(), value


def export_onnx(model: ZeroNet, path: Path) -> None:
    """入力がuint8、方策がFP16、価値がfloat32の推論用ONNXを書き出し、検査する。

    方策はpolicy_maskが真のラベルへ昇順に詰める。メタデータのpolicy_labelsに、
    詰める前のラベルを空白なしの10進整数のカンマ区切りで記録する。
    """
    sample = torch.zeros(1, INPUT_PLANES, 12, 12, dtype=torch.uint8)
    export_model = _OnnxExport(model).eval()
    with torch.inference_mode():
        torch.onnx.export(
            export_model,
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
    network = onnx.load(path)
    labels = network.metadata_props.add()
    labels.key = "policy_labels"
    labels.value = ",".join(str(label) for label in export_model.policy_labels.tolist())
    onnx.save(network, path)
    onnx.checker.check_model(str(path))
