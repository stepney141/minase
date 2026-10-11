"""方策と価値を出力するResNet。

構成は docs/plans/alphazero.md の「入力とネットの構成」による。
入力層の3×3畳み込みに残差ブロックの列が続き、各ブロックはKataGo式の大域プーリング
（チャネルごとの平均と最大値を全結合でチャネル数のバイアスへ写して足す層）を持つ。
方策の出力は249チャネルの1×1畳み込み、価値の出力は1チャネルの1×1畳み込みと全結合層2つとシグモイドである。
"""

from __future__ import annotations

import torch
from torch import nn

from minase_zero.encoding import INPUT_PLANES, POLICY_PLANES, POLICY_SIZE, SQUARE_COUNT

VALUE_HIDDEN = 256
"""価値の出力の中間の全結合層の幅。AlphaZeroの価値の出力と同じ値である。"""


class GlobalPoolingBias(nn.Module):
    """チャネルごとの平均と最大値を全結合でチャネル数のバイアスへ写す。"""

    def __init__(self, channels: int) -> None:
        super().__init__()
        self.linear = nn.Linear(2 * channels, channels)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        """`x`（`[N, C, 12, 12]`）から升によらないバイアス`[N, C, 1, 1]`を返す。"""
        pooled = torch.cat([x.mean(dim=(2, 3)), x.amax(dim=(2, 3))], dim=1)
        return self.linear(pooled)[:, :, None, None]


class ResidualBlock(nn.Module):
    """大域プーリングを持つ残差ブロック。

    1つ目の畳み込みの出力に大域プーリングのバイアスを足してから2つ目の畳み込みへ渡す。
    """

    def __init__(self, channels: int) -> None:
        super().__init__()
        self.conv1 = nn.Conv2d(channels, channels, 3, padding=1, bias=False)
        self.bn1 = nn.BatchNorm2d(channels)
        self.pool = GlobalPoolingBias(channels)
        self.conv2 = nn.Conv2d(channels, channels, 3, padding=1, bias=False)
        self.bn2 = nn.BatchNorm2d(channels)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        """残差を足した`[N, C, 12, 12]`を返す。"""
        h = torch.relu(self.bn1(self.conv1(x)))
        h = h + self.pool(h)
        h = self.bn2(self.conv2(h))
        return torch.relu(h + x)


class ZeroNet(nn.Module):
    """入力平面`[N, 96, 12, 12]`から方策のlogit`[N, 35856]`と価値`[N, 1]`を返すネット。

    方策の要素の並びはRust側のラベル`相対移動元 * 249 + 面`（`crates/minase/src/nn/policy.rs`）であり、
    1×1畳み込みの出力`[N, 249, 12, 12]`を`[N, 12, 12, 249]`へ転置してから平坦化して作る。
    価値は手番側の勝率であり、0〜1の値をとる。
    """

    def __init__(self, blocks: int, channels: int) -> None:
        super().__init__()
        if blocks < 0 or channels < 1:
            raise ValueError(f"invalid network size: blocks={blocks}, channels={channels}")
        self.input_conv = nn.Conv2d(INPUT_PLANES, channels, 3, padding=1, bias=False)
        self.input_bn = nn.BatchNorm2d(channels)
        self.blocks = nn.Sequential(*(ResidualBlock(channels) for _ in range(blocks)))
        self.policy_conv = nn.Conv2d(channels, POLICY_PLANES, 1)
        self.value_conv = nn.Conv2d(channels, 1, 1, bias=False)
        self.value_bn = nn.BatchNorm2d(1)
        self.value_fc1 = nn.Linear(SQUARE_COUNT, VALUE_HIDDEN)
        self.value_fc2 = nn.Linear(VALUE_HIDDEN, 1)

    def forward(self, x: torch.Tensor) -> tuple[torch.Tensor, torch.Tensor]:
        """(方策のlogit`[N, 35856]`, 価値`[N, 1]`)を返す。"""
        h = torch.relu(self.input_bn(self.input_conv(x)))
        h = self.blocks(h)
        policy = self.policy_conv(h).permute(0, 2, 3, 1).reshape(-1, POLICY_SIZE)
        v = torch.relu(self.value_bn(self.value_conv(h))).reshape(-1, SQUARE_COUNT)
        value = torch.sigmoid(self.value_fc2(torch.relu(self.value_fc1(v))))
        return policy, value

