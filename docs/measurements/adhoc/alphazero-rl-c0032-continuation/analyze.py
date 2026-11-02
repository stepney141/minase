"""指し継ぎの対局から、元の対局でMCTSが持っていた側の得点を集合ごとに集計する。

引数は実行ディレクトリと開始局面のJSONLである。
ペア番号はJSONLの行番号（1から）に対応する。信頼区間はペアを単位とする正規近似である。
"""

import glob
import json
import math
import statistics as st
import sys

run, openings = sys.argv[1:3]
rows = [json.loads(line) for line in open(openings)]
scores = {}
endings = {}
lengths = {}
different = {}
for path in sorted(glob.glob(f"{run}/pairs/*.json")):
    pair = json.load(open(path))
    row = rows[pair["pair_number"] - 1]
    if pair["opening"]["moves"] != row["moves"]:
        raise SystemExit(f"pair {pair['pair_number']} does not start from its opening")
    name = row["set"]
    games = []
    for game in pair["games"]:
        end = game["termination"]
        winner = end.get("winner")
        games.append(0.5 if winner is None else float(winner == row["mcts_color"]))
        key = (name, end.get("reason"))
        endings[key] = endings.get(key, 0) + 1
        lengths.setdefault(name, []).append(len(game["turns"]))
    moves = [[t["response"].get("usi") for t in g["turns"]] for g in pair["games"]]
    different.setdefault(name, []).append(moves[0] != moves[1])
    scores.setdefault(name, []).append(sum(games) / len(games))
for name, values in sorted(scores.items()):
    mean = st.mean(values)
    half = 1.96 * st.stdev(values) / math.sqrt(len(values)) if len(values) > 1 else float("nan")
    print(f"{name}: pairs {len(values)}; score of the original MCTS side {mean:.3f} +- {half:.3f}; "
          f"plies after the opening median {st.median(lengths[name])}; "
          f"pairs whose two games differ {sum(different[name])}")
print("endings:", sorted(endings.items(), key=str))
