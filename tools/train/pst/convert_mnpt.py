"""王の遮蔽と開いた筋の追加特徴を0にして、MNPTの各端点を13,680列から13,704列へ変換する。"""

from __future__ import annotations

import argparse
from pathlib import Path

import numpy as np

from features import FEATURE_COUNT as PST_FEATURE_COUNT
from train_pst import read_mnpt, write_mnpt


COLUMN_COUNT = 24
FEATURE_COUNT = PST_FEATURE_COUNT + COLUMN_COUNT


def convert_mnpt(source: str | Path, destination: str | Path) -> None:
    """既存重み、駒価値、K、規則セット名を保持し、各端点へ0の列を挿入する。"""
    source = Path(source)
    rule_set = source.read_bytes()[16:48].rstrip(b"\0")
    middlegame, endgame, piece_values, k = read_mnpt(
        source, feature_count=PST_FEATURE_COUNT, rule_set=rule_set,
    )
    extra = np.zeros(COLUMN_COUNT, dtype="<i2")
    write_mnpt(
        destination, np.concatenate((middlegame, extra)), np.concatenate((endgame, extra)),
        piece_values, k, feature_count=FEATURE_COUNT, rule_set=rule_set,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("destination", type=Path)
    arguments = parser.parse_args()
    convert_mnpt(arguments.source, arguments.destination)


if __name__ == "__main__":
    main()
