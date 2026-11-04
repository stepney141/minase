#!/usr/bin/env python3
"""対局で現れた局面で、ネットの静的評価と教師の探索値の誤差を比べる。

保存済みの対局から局面を抽出し、教師を作ったエンジンで探索値を求め、各エンジンのUSIの`eval`と比べる。
BCEは学習と同じ形で、目標はsigma(探索値/教師の勝率尺度)、予測はsigma(評価値/勝率尺度)である。
差の区間は対局単位の復元抽出で求める。

使い方:
  ood_compare.py --run <実行ディレクトリ> --teacher <エンジン> --engine 名前=エンジン [--engine ...]
      --step 20 --max-positions 2000 --nodes 20000 --seed 1 --output <JSON>
"""
import argparse
import glob
import json
import math
import os
import random
import subprocess
from collections import defaultdict

TEACHER_K = 1039.5786170404363
NET_K = 1072.6529541015625


class Engine:
    def __init__(self, path):
        self.process = subprocess.Popen(
            [path, "--protocol", "usi", "--rules", "engine-default"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
        )
        self.send("usi")
        self.until("usiok")
        self.send("setoption name USI_Hash value 16")
        self.send("isready")
        self.until("readyok")

    def send(self, line):
        self.process.stdin.write(line + "\n")
        self.process.stdin.flush()

    def until(self, prefix):
        lines = []
        while True:
            line = self.process.stdout.readline()
            if not line:
                raise SystemExit("engine exited")
            line = line.rstrip("\n")
            lines.append(line)
            if line.startswith(prefix):
                return lines

    def position(self, moves):
        self.send("usinewgame")
        self.send("position startpos moves " + " ".join(moves))

    def search(self, moves, nodes):
        self.position(moves)
        self.send(f"go nodes {nodes}")
        score = None
        for line in self.until("bestmove"):
            fields = line.split()
            if "score" in fields:
                index = fields.index("score")
                score = (fields[index + 1], int(fields[index + 2]))
        return score

    def evaluate(self, moves):
        self.position(moves)
        self.send("eval")
        for line in self.until("info string end"):
            fields = line.split()
            if fields[:3] == ["info", "string", "evaluation"]:
                return int(fields[3])
        raise SystemExit("no evaluation line")

    def close(self):
        self.send("quit")
        self.process.wait(timeout=30)


def sigma(x):
    return 1.0 / (1.0 + math.exp(-x))


def bce(pred_logit, target):
    return max(pred_logit, 0.0) - pred_logit * target + math.log1p(math.exp(-abs(pred_logit)))


def load_positions(run, step):
    games = []
    for path in sorted(glob.glob(os.path.join(run, "pairs", "*.json"))):
        with open(path, encoding="utf-8") as handle:
            pair = json.load(handle)
        opening = pair["opening"]["moves"]
        for game in pair["games"]:
            moves = opening + [t["response"]["usi"] for t in game["turns"] if t["response"]["kind"] == "move"]
            games.append(moves)
    positions = []
    for index, moves in enumerate(games):
        for ply in range(step, len(moves), step):
            positions.append((index, ply, moves[:ply]))
    return positions


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--run", required=True)
    parser.add_argument("--teacher", required=True)
    parser.add_argument("--engine", action="append", required=True)
    parser.add_argument("--step", type=int, default=20)
    parser.add_argument("--max-positions", type=int, default=2000)
    parser.add_argument("--nodes", type=int, default=20000)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    positions = load_positions(args.run, args.step)
    rng = random.Random(args.seed)
    rng.shuffle(positions)
    positions = sorted(positions[: args.max_positions], key=lambda x: (x[0], x[1]))
    names = [item.split("=", 1) for item in args.engine]
    teacher = Engine(args.teacher)
    engines = {name: Engine(path) for name, path in names}
    records = []
    excluded = 0
    for number, (game, ply, moves) in enumerate(positions):
        score = teacher.search(moves, args.nodes)
        if score is None or score[0] != "cp":
            excluded += 1
            continue
        row = {"game": game, "ply": ply, "teacher": score[1]}
        for name, engine in engines.items():
            row[name] = engine.evaluate(moves)
        records.append(row)
        if (number + 1) % 200 == 0:
            print(f"{number + 1}/{len(positions)}", flush=True)
    teacher.close()
    for engine in engines.values():
        engine.close()
    labels = [name for name, _ in names]

    def per_game(metric):
        groups = defaultdict(lambda: defaultdict(list))
        for row in records:
            for label in labels:
                groups[row["game"]][label].append(metric(row, label))
        return groups

    def summarize(metric):
        groups = per_game(metric)
        keys = sorted(groups)
        means = {label: sum(sum(groups[k][label]) for k in keys) / sum(len(groups[k][label]) for k in keys) for label in labels}
        result = {"mean": means}
        reference = labels[-1]
        draw = random.Random(args.seed)
        differences = {label: [] for label in labels[:-1]}
        for _ in range(1000):
            sample = [draw.choice(keys) for _ in keys]
            total = sum(len(groups[k][reference]) for k in sample)
            base = sum(sum(groups[k][reference]) for k in sample) / total
            for label in labels[:-1]:
                value = sum(sum(groups[k][label]) for k in sample) / total
                differences[label].append(value - base)
        result["difference_from_" + reference] = {
            label: {
                "mean": means[label] - means[reference],
                "low": sorted(differences[label])[25],
                "high": sorted(differences[label])[974],
            }
            for label in labels[:-1]
        }
        return result

    def mae(row, label):
        return abs(row[label] - row["teacher"])

    def loss(row, label):
        return bce(row[label] / NET_K, sigma(row["teacher"] / TEACHER_K))

    report = {
        "run": args.run,
        "positions": len(records),
        "excluded_not_cp": excluded,
        "games": len({row["game"] for row in records}),
        "nodes": args.nodes,
        "step": args.step,
        "seed": args.seed,
        "mean_absolute_error_cp": summarize(mae),
        "bce": summarize(loss),
    }
    bands = [(20, 100), (100, 200), (200, 300), (300, 10000)]
    by_ply = {}
    for low, high in bands:
        subset = [row for row in records if low <= row["ply"] < high]
        if subset:
            by_ply[f"{low}-{high}"] = {
                "positions": len(subset),
                "mae": {label: sum(mae(row, label) for row in subset) / len(subset) for label in labels},
                "bce": {label: sum(loss(row, label) for row in subset) / len(subset) for label in labels},
            }
    report["by_ply"] = by_ply
    with open(args.output, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2, ensure_ascii=False)
    print(json.dumps(report, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
