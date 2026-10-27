import argparse
from pathlib import Path

import onnx
import torch
from torch import nn


class ResidualBlock(nn.Module):
    def __init__(self, channels: int):
        super().__init__()
        self.conv1 = nn.Conv2d(channels, channels, 3, padding=1, bias=False)
        self.bn1 = nn.BatchNorm2d(channels)
        self.pool_bias = nn.Linear(2 * channels, channels)
        self.conv2 = nn.Conv2d(channels, channels, 3, padding=1, bias=False)
        self.bn2 = nn.BatchNorm2d(channels)

    def forward(self, x: torch.Tensor) -> torch.Tensor:
        y = torch.relu(self.bn1(self.conv1(x)))
        pooled = torch.cat((y.mean(dim=(2, 3)), y.amax(dim=(2, 3))), dim=1)
        y = y + self.pool_bias(pooled)[:, :, None, None]
        return torch.relu(x + self.bn2(self.conv2(y)))


class ZeroNet(nn.Module):
    def __init__(self, blocks: int, channels: int):
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
        x = self.trunk(x)
        return self.policy_head(x).flatten(1), self.value_head(x)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)

    torch.manual_seed(0)
    sample = torch.zeros(1, 95, 12, 12, dtype=torch.float32)
    for blocks, channels in ((6, 96), (10, 128), (15, 192), (20, 256)):
        model = ZeroNet(blocks, channels).eval()
        path = args.out_dir / f"zero-b{blocks}c{channels}.onnx"
        with torch.inference_mode():
            policy, value = model(sample)
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
        parameters = sum(parameter.numel() for parameter in model.parameters())
        print(
            f"{path.name}: parameters={parameters} bytes={path.stat().st_size} "
            f"policy={list(policy.shape)} value={list(value.shape)}",
            flush=True,
        )


if __name__ == "__main__":
    main()
