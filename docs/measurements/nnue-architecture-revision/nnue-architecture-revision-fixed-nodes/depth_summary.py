#!/usr/bin/env python3
"""保存済みの対局から、候補側と基準側の1手あたりの到達深さとノード数を集計する。

使い方: depth_summary.py <実行ディレクトリ>...
"""
import glob
import json
import os
import statistics
import sys
from collections import defaultdict


def walk(node, key):
    if isinstance(node, dict):
        for k, v in node.items():
            if k == key:
                yield v
            else:
                yield from walk(v, key)
    elif isinstance(node, list):
        for v in node:
            yield from walk(v, key)


def main():
    for run in sys.argv[1:]:
        paths = sorted(glob.glob(os.path.join(run, "pairs", "*.json")))
        values = defaultdict(list)
        sample_keys = None
        for path in paths:
            with open(path, encoding="utf-8") as handle:
                pair = json.load(handle)
            for game in pair["games"]:
                cc = game["candidate_color"]
                for turn in game["turns"]:
                    color = turn.get("color") or turn.get("side")
                    if sample_keys is None:
                        sample_keys = turn
                    who = "candidate" if color == cc else "baseline"
                    report = turn.get("search") or turn.get("info") or turn
                    depth = next(iter(walk(report, "depth")), None)
                    nodes = next(iter(walk(report, "nodes")), None)
                    if depth is not None:
                        values[(who, "depth")].append(depth)
                    if nodes is not None:
                        values[(who, "nodes")].append(nodes)
        print(run, len(paths))
        print(json.dumps(sample_keys, ensure_ascii=False)[:600])
        for key in sorted(values):
            data = values[key]
            print(key, len(data), round(statistics.mean(data), 3), statistics.median(data))


if __name__ == "__main__":
    main()
