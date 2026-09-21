#!/usr/bin/env python3
"""先読みの煙試験の集計。

match_runnerの実行ディレクトリ（--ponderありとなし）を読み、1局あたりの消費時間、
手数帯ごとの完了深さ、終局時の残り時間、予想手の出力率、的中率、および的中した手と
通常の手の消費時間と停止理由の分布を出力する。的中は「同じ側の直前の着手に付いた
予想手が、相手の実着手の表記と一致した手」として数える。表記の一致で数えるので、
件数はmatch_runnerの最終サマリ（審判層での再生）と突き合わせて確認する。
"""
import json, statistics, sys
from collections import Counter, defaultdict
from pathlib import Path

BANDS = [(0, 100), (100, 200), (200, 300), (300, 400), (400, 10**9)]
IMMEDIATE_NS = 10_000_000  # 10 ms未満を「直ちに指した」と数える。


def band(ply):
    return next(i for i, (lo, hi) in enumerate(BANDS) if lo <= ply < hi)


def load(run_dir):
    run = Path(run_dir)
    manifest = json.loads((run / "manifest.json").read_text())
    limit = manifest["candidate"]["limit"]
    pairs = [json.loads(p.read_text()) for p in sorted((run / "pairs").glob("*.json"))]
    return manifest, limit, pairs


def summarize(run_dir):
    manifest, limit, pairs = load(run_dir)
    base, inc = limit["base_ms"] * 10**6, limit["increment_ms"] * 10**6
    ponder = manifest["ponder"]
    game_time, remaining, depth = [], [], defaultdict(list)
    kinds = {k: {"time": [], "stop": Counter()} for k in ("hit", "miss", "plain")}
    moves = predictions = failures = 0
    for pair in pairs:
        opening = len(pair["opening"]["moves"])
        for game in pair["games"]:
            clock = {"black": base, "white": base}
            spent = {"black": 0, "white": 0}
            pending = {"black": None, "white": None}
            last_move = None
            for index, turn in enumerate(game["turns"]):
                side, think = turn["side"], turn["think_time_ns"]
                spent[side] += think
                clock[side] += inc - think
                if turn["response"]["kind"] != "move":
                    failures += turn["response"]["kind"] == "failure"
                    continue
                moves += 1
                predictions += turn["ponder"] is not None
                expected = pending[side]
                kind = "plain" if not ponder or expected is None else ("hit" if expected == last_move else "miss")
                kinds[kind]["time"].append(think)
                kinds[kind]["stop"][turn["stop_reason"]] += 1
                if turn["evaluation"] and turn["evaluation"]["depth"] is not None:
                    depth[band(opening + index)].append(turn["evaluation"]["depth"])
                pending[side] = turn["ponder"]
                last_move = turn["response"]["usi"]
            game_time.extend(spent.values())
            remaining.extend(clock.values())
    print(f"run: {run_dir}")
    print(f"ponder: {ponder}  pairs: {len(pairs)}  games: {2 * len(pairs)}  moves: {moves}  failures: {failures}")
    print(f"think time per side per game: mean {statistics.mean(game_time) / 1e9:.3f} s  total {sum(game_time) / 1e9:.1f} s")
    print(f"final remaining per side: mean {statistics.mean(remaining) / 1e9:.3f} s  min {min(remaining) / 1e9:.3f} s")
    print(f"prediction output rate: {predictions}/{moves} = {predictions / moves:.4f}")
    for lo_hi, i in zip(BANDS, range(len(BANDS))):
        if depth[i]:
            hi = "" if lo_hi[1] > 10**8 else lo_hi[1] - 1
            print(f"depth ply {lo_hi[0]}-{hi}: mean {statistics.mean(depth[i]):.3f}  n {len(depth[i])}")
    for kind, data in kinds.items():
        times = data["time"]
        if not times:
            continue
        immediate = sum(t < IMMEDIATE_NS for t in times)
        print(f"{kind}: n {len(times)} ({len(times) / moves:.4f} of moves)  mean {statistics.mean(times) / 1e6:.1f} ms  "
              f"median {statistics.median(times) / 1e6:.1f} ms  under 10 ms {immediate / len(times):.4f}  "
              f"stop {dict(data['stop'])}")
    print()


if __name__ == "__main__":
    for run_dir in sys.argv[1:]:
        summarize(run_dir)
