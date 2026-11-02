"""サイクル32対αβのSTCの対局から、指し継ぎの開始局面を選ぶ。

引数はSTCの実行ディレクトリ、出力のJSONL、および各集合の局面数である。
各対局の40手目から59手目で、手番側の駒の損得が±5未満の局面のうち、最初のMCTSの手番の局面と
最初のαβの手番の局面を候補とする。同じ対局から各集合へ1局面ずつを選ぶ。
出力の1行は、集合の名前、対局の識別子、手数、手番側の損得、手番の色、MCTS側の色、および開始手順である。
"""

import json
import random
import sys

from common import load_pair, opening_moves, pair_files, replay

run, output, count = sys.argv[1], sys.argv[2], int(sys.argv[3])
candidates = {"mcts_to_move": [], "alpha_beta_to_move": []}
for path in pair_files(run):
    pair = load_pair(path)
    opening = opening_moves(pair)
    for game_index, game in enumerate(pair["games"]):
        mcts_color = game["candidate_color"]
        found = set()
        for ply, history, board, turn in replay(game, opening):
            if ply >= 60:
                break
            if ply < 40:
                continue
            name = "mcts_to_move" if turn["side"] == mcts_color else "alpha_beta_to_move"
            material = board.material(turn["side"])
            if name not in found and -5 <= material < 5:
                found.add(name)
                candidates[name].append({
                    "set": name, "pair": pair["pair_number"], "game": game_index, "ply": ply,
                    "material": material, "side_to_move": turn["side"], "mcts_color": mcts_color,
                    "moves": list(history),
                })
rng = random.Random(20261105)
selected = []
for name, rows in candidates.items():
    print(name, "candidates", len(rows))
    selected.extend(rng.sample(rows, min(count, len(rows))))
rng.shuffle(selected)
with open(output, "w") as f:
    for row in selected:
        f.write(json.dumps(row) + "\n")
print("selected", len(selected))
