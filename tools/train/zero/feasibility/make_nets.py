import argparse
from pathlib import Path
import sys

import torch


sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from minase_zero.encoding import INPUT_PLANES
from minase_zero.net import ResidualBlock, ZeroNet, export_onnx


SIZES = ((6, 96), (10, 128), (15, 192), (20, 256))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)

    torch.manual_seed(0)
    sample = torch.zeros(1, INPUT_PLANES, 12, 12, dtype=torch.float32)
    for blocks, channels in SIZES:
        model = ZeroNet(blocks, channels).eval()
        path = args.out_dir / f"zero-b{blocks}c{channels}.onnx"
        with torch.inference_mode():
            policy, value = model(sample)
        export_onnx(model, path)
        parameters = sum(parameter.numel() for parameter in model.parameters())
        print(
            f"{path.name}: parameters={parameters} bytes={path.stat().st_size} "
            f"policy={list(policy.shape)} value={list(value.shape)}",
            flush=True,
        )


if __name__ == "__main__":
    main()
