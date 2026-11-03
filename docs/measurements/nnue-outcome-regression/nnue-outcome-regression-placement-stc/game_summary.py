#!/usr/bin/env python3
"""対局の終わり方、手数、候補の手番ごとの成績、および思考時間を集計する。

使い方: game_summary.py --data <repoのdata> --runs <実行ディレクトリ名>...
"""
import argparse
import collections
import glob
import json
import os
import statistics


def summarize(data, run):
    outcome = collections.Counter()
    by_color = collections.Counter()
    terminations = collections.Counter()
    lengths = {"win": [], "loss": [], "draw": []}
    candidate_ns, baseline_ns = [], []
    candidate_depth, baseline_depth = [], []
    for path in sorted(glob.glob(os.path.join(data, "matches", run, "pairs", "*.json"))):
        with open(path, encoding="utf-8") as handle:
            pair = json.load(handle)
        opening = len(pair["opening"]["moves"])
        for game in pair["games"]:
            termination = game["termination"]
            winner = termination.get("winner")
            color = game["candidate_color"]
            if termination["kind"] == "cutoff":
                result = "cutoff"
            elif winner is None:
                result = "draw"
            else:
                result = "win" if winner == color else "loss"
            outcome[result] += 1
            by_color[(color, result)] += 1
            terminations[(result, termination["kind"], termination.get("reason"))] += 1
            if result in lengths:
                lengths[result].append(opening + len(game["turns"]))
            for turn in game["turns"]:
                evaluation = turn.get("evaluation") or {}
                is_candidate = turn["side"] == color
                (candidate_ns if is_candidate else baseline_ns).append(turn["think_time_ns"])
                if "depth" in evaluation:
                    (candidate_depth if is_candidate else baseline_depth).append(evaluation["depth"])
    print(f"== {run}")
    print("候補の結果", dict(outcome))
    print("候補の手番ごと", {f"{color}:{result}": count for (color, result), count in sorted(by_color.items())})
    print("終わり方", {f"{result}/{kind}/{reason}": count for (result, kind, reason), count in sorted(terminations.items(), key=str)})
    for result, values in lengths.items():
        if values:
            print(f"手数の中央値 {result}: {statistics.median(values)}（{len(values)}局）")
    print(f"1手の思考時間の平均 候補 {statistics.mean(candidate_ns) / 1e6:.1f} ms  基準 {statistics.mean(baseline_ns) / 1e6:.1f} ms")
    print(f"到達深さの平均 候補 {statistics.mean(candidate_depth):.2f}  基準 {statistics.mean(baseline_depth):.2f}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--data", required=True)
    parser.add_argument("--runs", required=True, nargs="+")
    args = parser.parse_args()
    for run in args.runs:
        summarize(args.data, run)


if __name__ == "__main__":
    main()
