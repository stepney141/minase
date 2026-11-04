#!/usr/bin/env python3
"""対局ごとの損失の和から、対局結果を目標とした汎化差と、対応のあるブートストラップの区間を求める。

使い方: gap_report.py <CSV>...
"""
import csv
import sys
import numpy as np

RESAMPLES = 1000
SEED = 1


def main():
    data = {}
    for path in sys.argv[1:]:
        with open(path, encoding="utf-8") as handle:
            for row in csv.DictReader(handle):
                key = (int(row["file"]), int(row["game"]), row["split"])
                data.setdefault(row["net"], {})[key] = (float(row["sum_loss"]), int(row["count"]))
    nets = list(data)
    keys = sorted(data[nets[0]])
    for net in nets:
        if sorted(data[net]) != keys:
            raise SystemExit(f"{net}: game sets differ")
    train = [i for i, key in enumerate(keys) if key[2] == "train"]
    validation = [i for i, key in enumerate(keys) if key[2] == "validation"]
    counts = np.array([data[nets[0]][key][1] for key in keys], dtype=np.float64)
    sums = {net: np.array([data[net][key][0] for key in keys]) for net in nets}
    train = np.array(train)
    validation = np.array(validation)
    print(f"訓練の対局 {len(train)}局 {int(counts[train].sum())}局面  検証の対局 {len(validation)}局 {int(counts[validation].sum())}局面")

    def statistics(values, train_sel, validation_sel):
        t = values[train_sel].sum() / counts[train_sel].sum()
        v = values[validation_sel].sum() / counts[validation_sel].sum()
        return t, v, v - t

    print("ネット  訓練  検証  汎化差")
    for net in nets:
        t, v, g = statistics(sums[net], train, validation)
        print(f"{net} {t:.6f} {v:.6f} {g:+.6f}")
    rng = np.random.default_rng(SEED)
    samples = [(rng.choice(train, len(train)), rng.choice(validation, len(validation))) for _ in range(RESAMPLES)]
    pairs = [(a, b) for i, a in enumerate(nets) for b in nets[i + 1:]]
    print("比較（左−右）  汎化差の差 [95%区間]  検証の損失の差 [95%区間]")
    for a, b in pairs:
        d1, d2 = [], []
        for t_sel, v_sel in samples:
            sa, sb = statistics(sums[a], t_sel, v_sel), statistics(sums[b], t_sel, v_sel)
            d1.append(sa[2] - sb[2])
            d2.append(sa[1] - sb[1])
        pa, pb = statistics(sums[a], train, validation), statistics(sums[b], train, validation)
        print(f"{a}-{b} {pa[2]-pb[2]:+.6f} [{np.percentile(d1,2.5):+.6f}, {np.percentile(d1,97.5):+.6f}]  {pa[1]-pb[1]:+.6f} [{np.percentile(d2,2.5):+.6f}, {np.percentile(d2,97.5):+.6f}]")


if __name__ == "__main__":
    main()
