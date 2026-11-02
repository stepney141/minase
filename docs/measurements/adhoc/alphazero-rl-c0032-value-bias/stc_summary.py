"""サイクル32対αβのSTCの記録から、終局の内訳と評価値、および駒の損得の推移を集計する。"""

import statistics as st
import sys

from common import load_pair, opening_moves, pair_files, replay

run = sys.argv[1]
endings = {}
perspective_mismatch = 0
decided = []
mcts_at_decided = []
last_positive = []
marks = [20, 40, 60, 80, 100, 150, 200]
material = {m: [] for m in marks}


def cp(turn, candidate):
    """手番側から見た評価値を返す。詰みは±30000とする。

    コミット98dcfb18より前のハーネスの記録は、`mate -0`の符号を落として`mate_in`の0手にする。
    αβの`mate +0`は王駒を取る手であり、MCTSのこの記録は根の負けの確定である。
    """
    score = (turn.get("evaluation") or {}).get("score")
    if score is None:
        return None
    if score["kind"] == "cp":
        return score["value"]
    if score["kind"] != "mate_in" or (score["moves"] == 0 and turn["side"] == candidate):
        return -30000
    return 30000


for path in pair_files(run):
    pair = load_pair(path)
    opening = opening_moves(pair)
    for game in pair["games"]:
        candidate = game["candidate_color"]
        turns = game["turns"]
        for turn in turns:
            evaluation = turn.get("evaluation") or {}
            if evaluation.get("score") is not None and evaluation.get("perspective") != turn["side"]:
                perspective_mismatch += 1
        end = game["termination"]
        key = (end["reason"], "MCTS" if end.get("winner") == candidate else "alpha-beta")
        endings[key] = endings.get(key, 0) + 1
        for ply, _, board, _ in replay(game, opening):
            if ply in material:
                material[ply].append(board.material(candidate))
        if end.get("winner") == candidate:
            continue
        ab = [(i, cp(t, candidate)) for i, t in enumerate(turns) if t["side"] != candidate]
        mc = [(i, cp(t, candidate)) for i, t in enumerate(turns) if t["side"] == candidate]
        first = None
        for j, (i, value) in enumerate(ab):
            if value is not None and value >= 1000 and all(v is not None and v >= 500 for _, v in ab[j:]):
                first = i
                break
        if first is not None:
            decided.append(first + len(opening))
            near = [v for i, v in mc if i >= first - 1 and v is not None][:1]
            if near:
                mcts_at_decided.append(near[0])
        positive = [i for i, v in mc if v is not None and v >= 0]
        last_positive.append(len(turns) - positive[-1] if positive else len(turns))

print("perspective != side:", perspective_mismatch)
print("endings:", endings)
print("first ply where alpha-beta stays above +500 after reaching +1000: median", st.median(decided),
      "n", len(decided))
print("MCTS cp at that point: median", st.median(mcts_at_decided),
      "share >= 0", round(sum(v >= 0 for v in mcts_at_decided) / len(mcts_at_decided), 3))
print("plies before the end at which MCTS last evaluated >= 0: median", st.median(last_positive))
for ply in marks:
    values = sorted(material[ply])
    print(f"ply {ply}: n={len(values)} material for MCTS median {st.median(values)} "
          f"p10 {values[len(values) // 10]} p90 {values[9 * len(values) // 10]}")
