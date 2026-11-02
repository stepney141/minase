"""対局記録の局面を、サイクル32のネットが探索なしと少ない探索でどう評価するかを集計する。

引数はminaseのバイナリ、ネット、STCの実行ディレクトリ、進捗測定の実行ディレクトリ、αβ同士の実行ディレクトリである。
40手目から299手目の局面を各集合から5,000局面抽出する。
探索回数の比較は、同じ局面で比べるために先頭の800局面で行う。
"""

import math
import random
import statistics as st
import sys

from common import MATERIAL_BINS, load_pair, opening_moves, pair_files, replay, win_probability
from engine import Engine

binary, net, stc, progress, alpha_beta = sys.argv[1:6]


def collect(run, who):
    """whoの手番の局面を(手数, 手番側の損得, 記録の勝率, 手順)で返す。"""
    positions = []
    for path in pair_files(run):
        pair = load_pair(path)
        opening = opening_moves(pair)
        for game in pair["games"]:
            for ply, history, board, turn in replay(game, opening):
                if (turn["side"] == game["candidate_color"]) != (who == "candidate") or not 40 <= ply < 300:
                    continue
                score = (turn.get("evaluation") or {}).get("score")
                if score is not None and score["kind"] == "cp":
                    positions.append((ply, board.material(turn["side"]), win_probability(score["value"]), list(history)))
    return positions


def table(rows):
    for lo, hi in MATERIAL_BINS:
        values = [v for _, d, v in rows if lo <= d < hi]
        if len(values) >= 10:
            print(f"  material [{lo:4d},{hi:4d}) n={len(values):6d} win probability median {st.median(values):.3f} "
                  f"share >= 0.5 {sum(v >= 0.5 for v in values) / len(values):.3f}")
    for lo, hi in ((40, 60), (60, 80), (80, 120), (120, 180), (180, 300)):
        values = [v for p, d, v in rows if lo <= p < hi and -5 <= d < 5]
        if len(values) >= 10:
            print(f"  plies [{lo:3d},{hi:3d}) material within 5 n={len(values):5d} win probability median {st.median(values):.3f}")


sets = {
    "c0032 vs alpha-beta, MCTS to move": collect(stc, "candidate"),
    "c0032 vs alpha-beta, alpha-beta to move": collect(stc, "baseline"),
    "alpha-beta vs alpha-beta": collect(alpha_beta, "candidate") + collect(alpha_beta, "baseline"),
    "c0030 vs c0020": collect(progress, "candidate") + collect(progress, "baseline"),
}
print("== the win probabilities recorded in the games")
table([(p, d, v) for p, d, v, _ in sets["c0032 vs alpha-beta, MCTS to move"]])
print("== c0030's own record in c0030 vs c0020")
table([(p, d, v) for p, d, v, _ in collect(progress, "candidate")])
print("== c0020's own record in c0030 vs c0020")
table([(p, d, v) for p, d, v, _ in collect(progress, "baseline")])

engine = Engine(binary, net, 16)
rng = random.Random(20261103)
for name, positions in sets.items():
    sample = rng.sample(positions, min(5000, len(positions)))
    rows = []
    for ply, diff, _, history in sample:
        cp = engine.search(history, "go nodes 1")[0]
        if cp is not None:
            rows.append((ply, diff, win_probability(cp)))
    print(f"== c0032 nodes=1 on {name}: n={len(rows)} overall median {st.median(v for _, _, v in rows):.3f}")
    table(rows)
    results = {1: [], 32: [], 800: []}
    for ply, diff, _, history in sample[:800]:
        values = {n: engine.search(history, f"go nodes {n}")[0] for n in results}
        if all(v is not None for v in values.values()):
            for n, cp in values.items():
                results[n].append((ply, diff, win_probability(cp)))
    for nodes, rows in results.items():
        print(f"== c0032 nodes={nodes} on {name}, the same positions: n={len(rows)} "
              f"overall median {st.median(v for _, _, v in rows):.3f}")
        table(rows)
    print(flush=True)
engine.quit()
