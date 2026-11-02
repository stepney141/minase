"""自己対局またはαβの棋譜（MNSD）の局面で、駒の損得と探索の勝率と実際の得点を集計する。

引数はサイクルのディレクトリか、MNSDファイルである。投了で終えた対局のファイルは使わない。
サイクルNの自己対局はサイクルN-1のネットが生成するので、探索の勝率は1つ前のネットの値である。
"""

import json
import sys
from pathlib import Path

import numpy as np
import minase_mnsd_reader as reader

from common import MATERIAL_BINS
from selfplay_tables import TABLE

def load(paths):
    plies, diffs, probabilities, scores = [], [], [], []
    for path in paths:
        r = reader.read(str(path))
        boards = np.frombuffer(r["boards"], np.uint8).reshape(-1, 144)
        sides = np.frombuffer(r["sides"], np.uint8)
        outcomes = np.frombuffer(r["outcomes"], np.uint8)
        plies.append(np.frombuffer(r["plies"], "<u2").astype(np.int32))
        diffs.append(TABLE[boards & 0x7F].sum(axis=1) * np.where(sides == 0, 1, -1))
        probabilities.append(np.frombuffer(r["values"], "<i4") / 65535.0)
        scores.append(np.where(outcomes == 2, 0.5, (outcomes == sides).astype(float)))
    return tuple(np.concatenate(x) for x in (plies, diffs, probabilities, scores))


def report(name, paths, has_win_probability):
    ply, diff, probability, score = load(paths)
    window = (ply >= 40) & (ply < 300)
    print(f"== {name}: positions {len(ply)}; the rows use plies 40 to 299")
    for lo, hi in MATERIAL_BINS:
        s = window & (diff >= lo) & (diff < hi)
        if s.sum() >= 20:
            value = (f" search win probability median {np.median(probability[s]):.3f} mean {probability[s].mean():.3f}"
                     if has_win_probability else "")
            print(f"material [{lo:4d},{hi:4d}) n={s.sum():7d} actual score {score[s].mean():.3f}{value}")
    if has_win_probability:
        print(f"correlation of material and search win probability: {np.corrcoef(diff[window], probability[window])[0, 1]:.3f}")
        print("calibration by the predicted win probability:")
        total = 0.0
        for lo in np.arange(0.0, 1.0, 0.1):
            s = window & (probability >= lo) & (probability < lo + 0.1)
            if s.sum() >= 20:
                gap = probability[s].mean() - score[s].mean()
                total += abs(gap) * s.sum()
                print(f"predicted [{lo:.1f},{lo + 0.1:.1f}) n={s.sum():7d} mean predicted {probability[s].mean():.3f} actual score {score[s].mean():.3f}")
        print(f"expected calibration error: {total / window.sum():.3f}")


for argument in sys.argv[1:]:
    path = Path(argument)
    if path.is_dir():
        summary = json.loads((path / "selfplay/summary.json").read_text())
        files = [path / "selfplay" / n for n in summary["training_files"] if not n.endswith(".resigned.mnsd")]
        report(path.name, files, True)
    else:
        report(path.name, [path], False)
