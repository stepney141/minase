#!/usr/bin/env python3
"""Rustのnnue-float.binをremoval診断用の浮動小数点npzへ変換する。

PYTHONPATH=tools/train/src python tools/train/scripts/rust_float_to_npz.py \
    --input /path/to/nnue-float.bin --output /path/to/nnue-float.npz

チェックポイントの読み取りと検査にはcrosscheck_reference.checkpointを使う。
Kは診断時に対応するMNUEから読むため、重みの尺度は変更しない。
"""

from __future__ import annotations

import argparse
from pathlib import Path
import struct
import sys

import numpy as np

from crosscheck_reference import checkpoint
from minase_train.data.features import FEATURE_COUNT, canonical_feature_indices
from minase_train.nnue.feature_sets import get_feature_set


def convert(input_path: Path, output_path: Path) -> None:
    """正準行を展開し、特徴分解を畳み込んだ単精度の重みを保存する。"""
    saved = checkpoint(input_path)
    spec = get_feature_set(saved["feature_set"])
    parameters = saved["parameters"]
    indices = np.arange(FEATURE_COUNT)
    canonical = canonical_feature_indices(indices)
    linear = parameters["linear.weight"][canonical]
    biases = parameters["head.biases"]
    output_weights = parameters["head.output_weights"]
    if spec.factorized:
        base = spec.base_indices(np.arange(spec.feature_count))
        weights = (parameters["ft.embedding.weight"][:-1]
                   + parameters["ft.virtual.weight"][base])
    else:
        weights = parameters["ft.embedding.weight"][canonical]
        reflected = indices % 12 >= 6
        weights[reflected] = np.roll(weights[reflected], saved["width"] // 2, axis=1)
        biases = np.tile(biases, 2)
        output_weights = np.tile(output_weights, (1, 2))
    # ファイル名に拡張子を自動付与せず、指定されたパスへ書く。
    with output_path.open("wb") as stream:
        np.savez(stream, middlegame=linear[:, 0], endgame=linear[:, 1],
                 weights=weights, biases=biases, output_weights=output_weights,
                 output_bias=parameters["head.output_bias"], mask=saved["masks"][0][canonical])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        convert(args.input, args.output)
    except (OSError, ValueError, struct.error) as error:
        print(f"rust_float_to_npz: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
