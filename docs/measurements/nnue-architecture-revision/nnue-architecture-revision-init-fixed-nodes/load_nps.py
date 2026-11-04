#!/usr/bin/env python3
"""同時に複数のbenchを動かしたときのNPSを、エンジンごとに交互に測る。

対局の測定と同じ同時対局数の負荷（各局2エンジンのうち同時に動くのは1つ）を、benchの並列実行で模す。

使い方: load_nps.py --engine 名前=パス [--engine ...] --copies 16 --depth 15 --rounds 3 --output <JSON>
"""
import argparse
import json
import re
import statistics
import subprocess

SUMMARY = re.compile(r"^summary: .* nodes=(\d+) elapsed=([\d.]+)s nps=(\d+)")


def run(path, copies, depth):
    processes = [
        subprocess.Popen([path, "dev", "bench", "--depth", str(depth), "--repetitions", "1"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        for _ in range(copies)
    ]
    values = []
    for process in processes:
        out, _ = process.communicate()
        for line in out.splitlines():
            match = SUMMARY.match(line)
            if match:
                values.append(int(match[3]))
    if len(values) != copies:
        raise SystemExit("missing summary lines")
    return values


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", action="append", required=True)
    parser.add_argument("--copies", type=int, default=16)
    parser.add_argument("--depth", type=int, default=15)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    names = [item.split("=", 1) for item in args.engine]
    rounds = {name: [] for name, _ in names}
    for round_number in range(args.rounds):
        for name, path in names:
            values = run(path, args.copies, args.depth)
            rounds[name].append(statistics.median(values))
            print(round_number, name, statistics.median(values), min(values), max(values), flush=True)
    report = {"copies": args.copies, "depth": args.depth, "rounds": rounds, "median": {k: statistics.median(v) for k, v in rounds.items()}}
    with open(args.output, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2)
    print(json.dumps(report["median"], indent=1))


if __name__ == "__main__":
    main()
