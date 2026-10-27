"""固定局面集をCPUのPyTorchで評価し、minase dev nn-evalの照合用JSONを書く。"""

import argparse
import hashlib
import json
from pathlib import Path
import sys
import tempfile

import numpy as np
import torch

sys.path.insert(0, str(Path(__file__).resolve().parent / "feasibility"))
from make_nets import SIZES, ZeroNet, export_onnx


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

    with args.fixture.open(encoding="utf-8") as source:
        fixtures = [json.loads(line) for line in source]
    if not fixtures:
        raise ValueError("fixture contains no positions")
    inputs = np.zeros((len(fixtures), 95, 12, 12), dtype=np.float32)
    flat = inputs.reshape(len(fixtures), -1)
    for index, fixture in enumerate(fixtures):
        active = fixture["input"]
        if not isinstance(active, list) or any(
            type(offset) is not int or not 0 <= offset < 95 * 12 * 12 for offset in active
        ):
            raise ValueError(f"position {index}: invalid input indices")
        flat[index, active] = 1.0

    with torch.inference_mode():
        policy, value = model(torch.from_numpy(inputs))
    positions = []
    for index, fixture in enumerate(fixtures):
        moves = []
        for move in fixture["moves"]:
            label = move[4]
            if type(label) is not int or not 0 <= label < 35856:
                raise ValueError(f"position {index}: invalid policy label {label}")
            moves.append([label, policy[index, label].item()])
        positions.append({"sfen": fixture["sfen"], "value": value[index, 0].item(), "moves": moves})
    return {
        "network_sha256": hashlib.sha256(onnx_bytes).hexdigest(),
        "backend": "pytorch",
        "batch": len(fixtures),
        "positions": positions,
    }


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
