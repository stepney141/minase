#!/usr/bin/env python3
"""対局の連続する局面の静的評価の変化を比べる。

着手で手番が入れ替わるので、E(t)+E(t+1)は局面の変化に対する評価の増減（手番側から見た符号の反転込み）である。
探索の枝刈りと補正は、親子の静的評価の差に依存する。

使い方: eval_steps.py --run <実行ディレクトリ> --engine 名前=エンジン [--engine ...] --pairs 1500 --seed 1 --output <JSON>
"""
import argparse
import json
import random
import statistics
import sys
import os

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from ood_compare import Engine, load_positions


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--run", required=True)
    parser.add_argument("--engine", action="append", required=True)
    parser.add_argument("--pairs", type=int, default=1500)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    positions = load_positions(args.run, 1)
    rng = random.Random(args.seed)
    rng.shuffle(positions)
    chosen = [p for p in positions if p[1] >= 20][: args.pairs]
    names = [item.split("=", 1) for item in args.engine]
    engines = {name: Engine(path) for name, path in names}
    samples = {name: [] for name, _ in names}
    for game, ply, moves in chosen:
        for name, engine in engines.items():
            first = engine.evaluate(moves[:-1])
            second = engine.evaluate(moves)
            samples[name].append(first + second)
    for engine in engines.values():
        engine.close()
    report = {"pairs": len(chosen), "seed": args.seed}
    for name, values in samples.items():
        ordered = sorted(abs(v) for v in values)
        report[name] = {
            "mean": statistics.mean(values),
            "stdev": statistics.pstdev(values),
            "mean_absolute": statistics.mean(ordered),
            "median_absolute": statistics.median(ordered),
            "p90_absolute": ordered[int(len(ordered) * 0.9)],
            "p99_absolute": ordered[int(len(ordered) * 0.99)],
        }
    with open(args.output, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2, ensure_ascii=False)
    print(json.dumps(report, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
