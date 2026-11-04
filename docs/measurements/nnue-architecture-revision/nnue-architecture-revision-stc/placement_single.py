#!/usr/bin/env python3
"""1回の対局の候補側と基準側の配置の距離Mと5配置の割合を求める。

使い方: placement_single.py --data <リポジトリのdata> --run <実行ディレクトリ名> --tool <placement_distance.pyのあるディレクトリ>
"""
import argparse
import math
import sys


def elo(p):
    return -400 * math.log10(1 / p - 1)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", required=True)
    parser.add_argument("--run", required=True)
    parser.add_argument("--tool", required=True)
    args = parser.parse_args()
    sys.path.insert(0, args.tool)
    import placement_distance as pd

    distance = pd.Distance(pd.load_counts(args.data))
    _, _, candidate, baseline = pd.load_run(args.data, args.run)
    first, second = candidate.sum(axis=0), baseline.sum(axis=0)
    print(f"ペア数 {len(candidate)}")
    print(f"M(候補側) = {distance.m(first):.4f}")
    print(f"M(基準側) = {distance.m(second):.4f}")
    a, b = pd.placement_rates(first), pd.placement_rates(second)
    for name in a:
        print(f"  {name}: 候補側 {a[name]:.1%}  基準側 {b[name]:.1%}")


if __name__ == "__main__":
    main()
