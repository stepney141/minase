"""学習と推論に共通のネット定義と、opset 17のONNXへの出力。"""

from pathlib import Path

import onnx
import torch
from torch import nn


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


class ZeroNet(nn.Module):
    """95面から方策のlogitと手番側の勝率を返すネット。"""

    def __init__(self, blocks: int, channels: int) -> None:
        super().__init__()
        self.trunk = nn.Sequential(
            nn.Conv2d(95, channels, 3, padding=1, bias=False),
            nn.BatchNorm2d(channels),
            nn.ReLU(),
            *(ResidualBlock(channels) for _ in range(blocks)),
        )
        self.policy_head = nn.Conv2d(channels, 249, 1, bias=True)
        self.value_head = nn.Sequential(
            nn.Conv2d(channels, 1, 1, bias=False),
            nn.BatchNorm2d(1),
            nn.ReLU(),
            nn.Flatten(1),
            nn.Linear(144, 256),
            nn.ReLU(),
            nn.Linear(256, 1),
            nn.Sigmoid(),
        )

    def forward(self, x: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        """推論用の方策のlogitとsigmoid後の勝率を返す。"""
        x = self.trunk(x)
        return self.policy_head(x).flatten(1), self.value_head(x)

    def forward_logits(self, x: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        """学習用に方策とsigmoid前の価値のlogitを返す。"""
        x = self.trunk(x)
        value = x
        for layer in self.value_head[:-1]:
            value = layer(value)
        return self.policy_head(x).flatten(1), value


def export_onnx(model: ZeroNet, path: Path) -> None:
    """実現性の測定と参照推論で同じ設定のONNXを書き出し、検査する。"""
    sample = torch.zeros(1, 95, 12, 12, dtype=torch.float32)
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
