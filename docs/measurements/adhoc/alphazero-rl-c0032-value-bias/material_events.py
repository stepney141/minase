"""αβとの対局で駒を失う形と、自己対局の終わり方を集計する。

第1引数はSTCの実行ディレクトリ、第2引数はサイクルのディレクトリである。
"""

import json
import statistics as st
import sys
from pathlib import Path

import numpy as np
import minase_mnsd_reader as reader

from common import Board, final_square, load_pair, opening_moves, pair_files, piece_value
from selfplay_tables import TABLE

run, cycle = sys.argv[1:3]


unanswered = answered = 0
lost_unanswered = lost_answered = gained = 0
big_events = 0
for path in pair_files(run):
    pair = load_pair(path)
    opening = opening_moves(pair)
    for game in pair["games"]:
        candidate = game["candidate_color"]
        turns = [t for t in game["turns"] if t["response"].get("kind") == "move"]
        board = Board()
        side = "black"
        for usi in opening:
            board.play(usi, side)
            side = "white" if side == "black" else "black"
        # 同じ駒を取り返したかは、取った駒の同一性で判定する（2段階の着手も含む）。
        pending = None
        for turn in turns:
            usi = turn["response"]["usi"]
            captured = board.play(usi, turn["side"])
            total = sum(piece_value(c) for c in captured)
            if turn["side"] == candidate:
                gained += total
                if pending is not None:
                    if any(c is pending[0] for c in captured):
                        answered += 1
                        lost_answered += pending[1]
                    else:
                        unanswered += 1
                        lost_unanswered += pending[1]
                        big_events += pending[1] >= 8
                    pending = None
            elif total > 0:
                pending = (board.pieces[final_square(usi)], total)
print(f"alpha-beta captures answered on the same square: {answered}, value {lost_answered}")
print(f"alpha-beta captures not answered: {unanswered}, value {lost_unanswered}, of which value >= 8: {big_events}")
print(f"value captured by MCTS: {gained}")

summary = json.loads((Path(cycle) / "selfplay/summary.json").read_text())
last = {}
for name in summary["training_files"]:
    if name.endswith(".resigned.mnsd"):
        continue
    r = reader.read(str(Path(cycle) / "selfplay" / name))
    games = np.frombuffer(r["games"], "<u4")
    plies = np.frombuffer(r["plies"], "<u2")
    boards = np.frombuffer(r["boards"], np.uint8).reshape(-1, 144)
    sides = np.frombuffer(r["sides"], np.uint8)
    values = np.frombuffer(r["values"], "<i4") / 65535.0
    outcomes = np.frombuffer(r["outcomes"], np.uint8)
    material = TABLE[boards & 0x7F].sum(axis=1)
    for g, p, m, s, v, o in zip(games, plies, material, sides, values, outcomes):
        if o != 2 and (g not in last or p > last[g][0]):
            winner_black = o == 0
            last[g] = (int(p), int(m) if winner_black else -int(m), float(v if s == o else 1 - v), s == o)
rows = list(last.values())
winner_material = sorted(r[1] for r in rows)
winner_value = sorted(r[2] for r in rows)
print(f"decided self-play games with a recorded position: {len(rows)}; game length median {st.median(r[0] for r in rows)}")
print(f"last recorded position, material for the eventual winner: median {st.median(winner_material)} "
      f"p10 {winner_material[len(rows) // 10]} p90 {winner_material[9 * len(rows) // 10]}; "
      f"share <= 0: {sum(m <= 0 for m in winner_material) / len(rows):.3f}; share < 10: {sum(m < 10 for m in winner_material) / len(rows):.3f}")
print(f"last recorded position, win probability for the eventual winner: median {st.median(winner_value):.3f}; "
      f"share < 0.5: {sum(v < 0.5 for v in winner_value) / len(rows):.3f}")
winner_turn = [r[2] for r in rows if r[3]]
print(f"last recorded position on the eventual winner's turn: {len(winner_turn)} games; "
      f"share of search win probability < 0.5: {sum(v < 0.5 for v in winner_turn) / len(winner_turn):.3f}")
