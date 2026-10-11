"""ネットをONNXへ出力する。

出力するONNXは、入力名`input`（`[N, 96, 12, 12]`、float32）、出力名`policy`（`[N, 35856]`）と
`value`（`[N, 1]`）を持ち、バッチの大きさNを実行時に決められる。

使い方:
    python -m minase_zero.export --blocks 10 --channels 128 --out net.onnx
    python -m minase_zero.export --blocks 10 --channels 128 --weights model.pt --out net.onnx
"""

from __future__ import annotations

import argparse
from pathlib import Path

import torch

from minase_zero.encoding import BOARD_SIZE, INPUT_PLANES
from minase_zero.model import ZeroNet

OPSET = 18
"""ONNXのopset。torch.onnx.exportのtorch.export方式が対応する最小の版であり、
ONNX Runtime 1.28とTensorRT 10.14のどちらも実行できる。"""

INPUT_NAME = "input"
"""ONNXの入力名。"""
OUTPUT_NAMES = ("policy", "value")
"""ONNXの出力名。"""


def export_onnx(model: ZeroNet, path: Path) -> None:
    """推論モードのネットを、バッチの大きさを動的にしたONNXとして`path`へ書く。"""
    model.eval()
    example = torch.zeros(2, INPUT_PLANES, BOARD_SIZE, BOARD_SIZE)
    batch = torch.export.Dim("batch", min=1)
    with torch.no_grad():
        torch.onnx.export(
            model,
            (example,),
            str(path),
            input_names=[INPUT_NAME],
            output_names=list(OUTPUT_NAMES),
            dynamic_shapes={"x": {0: batch}},
            opset_version=OPSET,
            dynamo=True,
            external_data=False,
        )


def build_model(blocks: int, channels: int, weights: Path | None, seed: int) -> ZeroNet:
    """ネットを作り、`weights`があれば学習済みの重み（`state_dict`）を読み込む。"""
    torch.manual_seed(seed)
    model = ZeroNet(blocks, channels)
    if weights is not None:
        model.load_state_dict(torch.load(weights, map_location="cpu", weights_only=True))
    return model


def main(argv: list[str] | None = None) -> None:
    """コマンドラインの入口。"""
    parser = argparse.ArgumentParser(description="ZeroNetをONNXへ出力する。")
    parser.add_argument("--blocks", type=int, required=True, help="残差ブロックの数")
    parser.add_argument("--channels", type=int, required=True, help="チャネル数")
    parser.add_argument("--out", type=Path, required=True, help="出力するONNXファイル")
    parser.add_argument(
        "--weights", type=Path, help="学習済みの重み（state_dict）。省略時はランダム初期化"
    )
    parser.add_argument("--seed", type=int, default=0, help="ランダム初期化のシード")
    args = parser.parse_args(argv)
    model = build_model(args.blocks, args.channels, args.weights, args.seed)
    export_onnx(model, args.out)


if __name__ == "__main__":
    main()
