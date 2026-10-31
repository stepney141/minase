"""固定局面集の入力を組み立て、CPUの参照推論を照合用JSONへまとめる。"""

import hashlib
import json
from pathlib import Path

import numpy as np
import torch

from .encoding import INPUT_PLANES
from .net import ZeroNet


def evaluate_fixture(model: ZeroNet, fixture_path: Path, onnx_bytes: bytes) -> dict:
    """全局面を1バッチで推論し、nn-evalと同じ順序の合法手のlogitを返す。"""
    model = model.cpu().float().eval()
    with fixture_path.open(encoding="utf-8") as source:
        fixtures = [json.loads(line) for line in source]
    if not fixtures:
        raise ValueError("fixture contains no positions")
    inputs = np.zeros((len(fixtures), INPUT_PLANES, 12, 12), dtype=np.float32)
    flat = inputs.reshape(len(fixtures), -1)
    for index, fixture in enumerate(fixtures):
        active = fixture["input"]
        if not isinstance(active, list) or any(
            type(offset) is not int or not 0 <= offset < INPUT_PLANES * 12 * 12 for offset in active
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

