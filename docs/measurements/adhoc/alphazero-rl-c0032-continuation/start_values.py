"""指し継ぎの開始局面を探索なしで評価し、元の対局でMCTSが持っていた側の勝率を集合ごとに求める。

引数はminaseのバイナリ、ネット、および開始局面のJSONLである。
"""

import json
import statistics as st
import sys

from common import win_probability
from engine import Engine

binary, net, openings = sys.argv[1:4]
rows = [json.loads(line) for line in open(openings)]
engine = Engine(binary, net, 16)
values = {}
for row in rows:
    cp = engine.search(row["moves"], "go nodes 1")[0]
    if cp is None:
        continue
    probability = win_probability(cp)
    if row["side_to_move"] != row["mcts_color"]:
        probability = 1.0 - probability
    values.setdefault(row["set"], []).append(probability)
engine.quit()
for name, probabilities in sorted(values.items()):
    print(f"{name}: n={len(probabilities)}; win probability of the original MCTS side at the start: "
          f"median {st.median(probabilities):.3f} mean {st.mean(probabilities):.3f}")
