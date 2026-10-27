"""Summarizes the FP16 differences on legal moves and the per-layer activation ranges of the trained net.

Usage (from tools/train/zero): uv run --frozen python analysis.py <checkpoint.pt> <directory with the JSON outputs>
"""

import json
from pathlib import Path
import sys

import numpy as np
import torch

from minase_zero.net import ZeroNet


def softmax(logits: np.ndarray) -> np.ndarray:
    weights = np.exp(logits - logits.max())
    return weights / weights.sum()


def main() -> None:
    checkpoint, directory = Path(sys.argv[1]), Path(sys.argv[2])
    reference = json.loads((directory / "pytorch.json").read_text())
    for name in ("cpu-b1", "cpu-b7", "cpu-b256", "trt-b1", "trt-b7", "trt-b256"):
        candidate = json.loads((directory / f"{name}.json").read_text())
        logit_diff, probability_diff, kl, value_diff = 0.0, 0.0, 0.0, 0.0
        for expected, actual in zip(reference["positions"], candidate["positions"]):
            a = np.array([move[1] for move in expected["moves"]], dtype=np.float64)
            b = np.array([move[1] for move in actual["moves"]], dtype=np.float64)
            p, q = softmax(a), softmax(b)
            logit_diff = max(logit_diff, float(np.abs(a - b).max()))
            probability_diff = max(probability_diff, float(np.abs(p - q).max()))
            kl = max(kl, float((p * (np.log(p) - np.log(q))).sum()))
            value_diff = max(value_diff, abs(expected["value"] - actual["value"]))
        print(f"{name}: max_logit_diff={logit_diff:.3e} max_legal_probability_diff={probability_diff:.3e} "
              f"max_kl={kl:.3e} max_value_diff={value_diff:.3e}")
    largest = [max(abs(move[1]) for move in position["moves"]) for position in reference["positions"]]
    values = np.array([position["value"] for position in reference["positions"]])
    print(f"largest |legal logit| per position: median={np.median(largest):.2f} max={max(largest):.2f}")
    print(f"value: min={values.min():.4f} max={values.max():.4f} std={values.std():.4f}")
    # Scale of a wiring error: reading each legal move's logit from its neighbor in the move list.
    shifted = []
    for position in reference["positions"]:
        logits = np.array([move[1] for move in position["moves"]], dtype=np.float64)
        shifted.append(float(np.abs(softmax(logits) - softmax(np.roll(logits, 1))).max()))
    print(f"neighbor-misread max legal probability diff per position: "
          f"median={np.median(shifted):.3e} max={max(shifted):.3e}")

    state = torch.load(checkpoint, map_location="cpu", weights_only=True)
    model = ZeroNet(state["config"]["blocks"], state["config"]["channels"])
    model.load_state_dict(state["model"])
    model.eval()
    fixtures = [json.loads(line) for line in Path("fixtures/nn-positions.jsonl").open(encoding="utf-8")]
    inputs = np.zeros((len(fixtures), 95 * 144), dtype=np.float32)
    for index, fixture in enumerate(fixtures):
        inputs[index, fixture["input"]] = 1.0
    ranges: list[tuple[str, float]] = []
    for name, module in model.named_modules():
        if not list(module.children()):
            module.register_forward_hook(
                lambda module, args, output, name=name: ranges.append((name, float(output.abs().max())))
            )
    with torch.inference_mode():
        model(torch.from_numpy(inputs.reshape(-1, 95, 12, 12)))
    top = sorted(ranges, key=lambda item: -item[1])[:5]
    print("largest |activation| by layer (FP16 max 65504): "
          + ", ".join(f"{name}={value:.2f}" for name, value in top))


if __name__ == "__main__":
    main()
