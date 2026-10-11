#!/usr/bin/env python3
"""dump-batchの全列をPythonのMNSD読み手と入力特徴集合で照合する。

PYTHONPATH=tools/train/src python tools/train/scripts/crosscheck_batch.py \
    --data first.bin second.bin --dump /tmp/nnue-batch/batch.json

通し番号、入力特徴集合、先読み条件はJSONに記録した値を使う。
特徴番号は厳密一致、教師探索値はf32保存の許容差0.0001 cpで検査する。
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

import numpy as np

from minase_train.data.features import FEATURE_COUNT, canonical_feature_indices
from minase_train.data.mnsd import Dataset
from minase_train.nnue.feature_sets import get_feature_set


SENTINEL = 2**32 - 1


def compare(name: str, actual: object, expected: np.ndarray, *, atol: float = 0.0) -> bool:
    """最初の不一致の局面・視点・列を報告する。形の違いも失敗にする。"""
    actual = np.asarray(actual)
    if actual.shape != expected.shape:
        print(f"{name}: shape {actual.shape}, expected {expected.shape}", file=sys.stderr)
        return False
    equal = (actual == expected if atol == 0.0
             else np.isclose(actual, expected, rtol=0.0, atol=atol))
    if bool(np.all(equal)):
        return True
    location = tuple(int(index) for index in np.argwhere(~equal)[0])
    print(f"{name}{location}: Rust={actual[location]}, Python={expected[location]}",
          file=sys.stderr)
    return False


def crosscheck(paths: list[Path], dump_path: Path) -> bool:
    dump = json.loads(dump_path.read_text(encoding="utf-8"))
    if dump["format"] != "minase-nnue-batch-indices" or dump["version"] != 1:
        raise ValueError("unsupported batch dump format/version")
    if dump["sentinel"] != SENTINEL:
        raise ValueError("unexpected sentinel")
    if [path.resolve() for path in paths] != [Path(path).resolve() for path in dump["data"]]:
        raise ValueError("--data files must match the dump's file order")
    indices = np.asarray(dump["record_indices"], dtype=np.int64)
    if indices.ndim != 1 or indices.size == 0:
        raise ValueError("record_indices must be a nonempty one-dimensional array")
    dataset = Dataset(paths, lookahead=dump["lookahead"], lambda_override=1.0)
    if any(teacher.origin != "random-selfplay" for teacher, _, _ in dataset.provenance):
        raise ValueError("only random selfplay is supported")
    records = dataset.gather(indices)
    spec = get_feature_set(dump["feature_set"])
    bases, inputs = [], []
    for side in (records["stm"], 1 - records["stm"]):
        base, features = spec.indices(records["board"], records["lion"], side)
        bases.append(base)
        if spec.name == "P":
            packed = canonical_feature_indices(features).astype(np.uint32) * 2
            packed += (features % 12 >= 6).astype(np.uint32)
            inputs.append(np.where(features == FEATURE_COUNT, SENTINEL, packed))
        else:
            inputs.append(np.where(features == spec.feature_count, SENTINEL,
                                   features.astype(np.uint32)))
    valid = compare("input_features[batch,view,column]", dump["input_features"],
                    np.stack(inputs, axis=1))
    if spec.name == "P":
        # Rustで復元した基底特徴をspec.indicesの独立な結果と全列で比較する。
        stacked = np.stack(bases, axis=1).astype(np.uint32)
        expected_base = np.where(stacked == FEATURE_COUNT, SENTINEL, stacked)
        valid &= compare("base_features[batch,view,column]", dump["base_features"], expected_base)
    linear = canonical_feature_indices(bases[0]).astype(np.uint32)
    linear = np.where(bases[0] == FEATURE_COUNT, SENTINEL, linear)
    valid &= compare("linear_features[batch,column]", dump["linear_features"], linear)
    valid &= compare("piece_counts[batch]", dump["piece_counts"],
                     np.count_nonzero(records["board"], axis=1))
    valid &= compare("outcomes[batch]", dump["outcomes"], records["result"])
    # JSONの十進表記はf32へ戻してから比較する。
    valid &= compare("teacher_scores[batch]", np.asarray(dump["teacher_scores"], dtype=np.float32),
                     dataset.teacher_scores(indices).astype(np.float32), atol=1e-4)
    # このフェーズの分類は指示書どおりヘッダの検査和・ノード数・先読み条件で決まる。
    # Python Datasetの拡張されたTeacherClassの分類番号には依存しない。
    classes, file_classes = [], []
    for header in dataset.headers:
        key = (header.network_checksum, header.teacher_nodes)
        if key not in classes:
            classes.append(key)
        file_classes.append(classes.index(key))
    file_numbers = np.searchsorted(dataset.offsets[1:], indices, side="right")
    valid &= compare("teacher_classes[batch]", dump["teacher_classes"],
                     np.asarray(file_classes)[file_numbers])
    if valid:
        print(f"一致: {spec.name}, {indices.size}局面, 両視点145列と線形145列")
    return bool(valid)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", nargs="+", required=True, type=Path)
    parser.add_argument("--dump", required=True, type=Path, help="dump-batchが書いたbatch.json")
    args = parser.parse_args()
    try:
        return 0 if crosscheck(args.data, args.dump) else 1
    except (OSError, ValueError, KeyError, IndexError, TypeError) as error:
        print(f"crosscheck_batch: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
