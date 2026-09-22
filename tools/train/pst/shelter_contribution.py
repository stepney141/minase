"""露出局面の集合に対する項目1の寄与を、量子化した重みから計算する。

典拠は docs/plans/lookahead-teacher.md の「用語」（項目1の寄与）と設計判断「事前の判定」。
寄与は、手番側の視点の24列の特徴値と追加重みの積の和を、整数評価と同じ
q = min(90, max(0, N − 2)) で補間した値（センチポーン、実数）である。
"""

from __future__ import annotations

import argparse
import json
import struct
from pathlib import Path

import numpy as np

from mnsd import map_records
from taper import phase_numerators
from train_pst import FEATURE_COUNT, read_mnpt

PHASE_DIVISOR = 90
MNKF_HEADER = struct.Struct("<4sIIIQ32s")


def read_mnkf_rows(path: Path, indices: np.ndarray, columns: int = 24) -> np.ndarray:
    """MNKFの指定行の先頭`columns`列を読む。"""
    with path.open("rb") as handle:
        magic, version, _definition, column_count, count, _digest = MNKF_HEADER.unpack(
            handle.read(MNKF_HEADER.size)
        )
        if magic != b"MNKF" or version != 1:
            raise ValueError(f"{path}: unsupported MNKF")
        if columns > column_count:
            raise ValueError(f"{path}: fewer than {columns} columns")
        rows = np.memmap(handle, dtype=np.uint8, mode="r", offset=MNKF_HEADER.size,
                         shape=(count, column_count))
        return np.asarray(rows[np.asarray(indices, dtype=np.int64), :columns], dtype=np.int64)


def contributions(sample: dict, group: str, features: dict[str, Path], pst: Path) -> dict:
    middlegame, endgame, _, _ = read_mnpt(pst, feature_count=None)
    extra = np.column_stack((middlegame[FEATURE_COUNT:], endgame[FEATURE_COUNT:])).astype(np.int64)
    if extra.shape[0] < 24:
        raise ValueError("the weights must carry the 24 shelter columns")
    extra = extra[:24]
    values: list[float] = []
    strata: list[int] = []
    by_file: dict[str, list[int]] = {}
    for position in sample["positions"]:
        if position["group"] != group:
            continue
        by_file.setdefault(position["file"], []).append(position["index"])
    for file, indices in by_file.items():
        name = Path(file).name
        if name not in features:
            raise ValueError(f"no MNKF for {name}")
        records = map_records(file)
        rows = np.asarray(indices, dtype=np.int64)
        q = phase_numerators(records["board"][rows])
        f = read_mnkf_rows(features[name], rows)
        sums = f @ extra
        blended = q * sums[:, 0] + (PHASE_DIVISOR - q) * sums[:, 1]
        values.extend((blended / (PHASE_DIVISOR * 8)).tolist())
    array = np.asarray(values, dtype=np.float64)
    return {
        "group": group,
        "positions": int(array.size),
        "mean": float(array.mean()),
        "median": float(np.median(array)),
        "std": float(array.std(ddof=1)) if array.size > 1 else None,
        "quantiles": {str(p): float(np.percentile(array, p)) for p in (5, 25, 50, 75, 95)},
        "fraction_negative": float((array < 0).mean()),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sample", required=True, type=Path)
    parser.add_argument("--group", default="exposed")
    parser.add_argument("--data", required=True, nargs="+", type=Path, help="標本が参照するMNSD")
    parser.add_argument("--king-features", required=True, nargs="+", type=Path,
                        help="--data と同じ順序の118列のMNKF")
    parser.add_argument("--pst", required=True, type=Path)
    parser.add_argument("--output", type=Path)
    arguments = parser.parse_args()
    sample = json.loads(arguments.sample.read_text())
    if len(arguments.data) != len(arguments.king_features):
        raise SystemExit("--data and --king-features must have the same length")
    features = {data.name: mnkf for data, mnkf in zip(arguments.data, arguments.king_features)}
    result = contributions(sample, arguments.group, features, arguments.pst)
    result["pst"] = str(arguments.pst)
    text = json.dumps(result, ensure_ascii=False, indent=2)
    if arguments.output:
        arguments.output.write_text(text + "\n")
    print(text)


if __name__ == "__main__":
    main()
