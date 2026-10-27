"""固定局面集をCPUのPyTorchで評価し、minase dev nn-evalの照合用JSONを書く。"""

import argparse
import json
from pathlib import Path
import sys
import tempfile

import torch

sys.path.insert(0, str(Path(__file__).resolve().parent / "feasibility"))
from make_nets import SIZES, ZeroNet, export_onnx
from minase_zero.fixture import evaluate_fixture


def reproduce_model(blocks: int, channels: int) -> ZeroNet:
    """乱数をSIZESの順に消費し、make_nets.pyと同じ重みを再現する。"""
    if (blocks, channels) not in SIZES:
        raise ValueError(f"unsupported network size {(blocks, channels)}; expected one of {SIZES}")
    torch.manual_seed(0)
    for size in SIZES:
        model = ZeroNet(*size).eval()
        if size == (blocks, channels):
            return model
    raise ValueError("requested network size was not constructed")


def evaluate(args: argparse.Namespace) -> dict:
    model = reproduce_model(args.blocks, args.channels)
    onnx_bytes = args.onnx.read_bytes()
    with tempfile.TemporaryDirectory(prefix="minase-nn-reference-") as directory:
        reproduced = Path(directory) / "reference.onnx"
        export_onnx(model, reproduced)
        if reproduced.read_bytes() != onnx_bytes:
            raise ValueError("reproduced ONNX bytes differ from --onnx; weights are not identical")

    return evaluate_fixture(model, args.fixture, onnx_bytes)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--onnx", type=Path, required=True)
    parser.add_argument("--blocks", type=int, required=True)
    parser.add_argument("--channels", type=int, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = evaluate(args)
        text = json.dumps(result, ensure_ascii=False, allow_nan=False, separators=(",", ":"))
        args.out.write_text(text + "\n", encoding="utf-8")
    except (OSError, ValueError, KeyError, IndexError, TypeError, RuntimeError) as error:
        print(f"PyTorch reference failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
