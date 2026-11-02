"""固定ノードの対局の条件ごとに、終局の内訳と、王駒の捕獲で負けた局の最後の形勢を集計する。

引数は実行ディレクトリの列である。駒の損得は歩を1とする概算で、MCTSの最後の手番の着手前の値である。
"""

import statistics as st
import sys

from common import load_pair, opening_moves, pair_files, replay, win_probability

for run in sys.argv[1:]:
    endings = {}
    plies = []
    final_material = []
    final_probability = []
    for path in pair_files(run):
        pair = load_pair(path)
        opening = opening_moves(pair)
        for game in pair["games"]:
            candidate = game["candidate_color"]
            end = game["termination"]
            won = end.get("winner") == candidate
            key = (end["reason"], "MCTS" if won else "alpha-beta")
            endings[key] = endings.get(key, 0) + 1
            plies.append(len(opening) + len(game["turns"]))
            if won or end["reason"] != "RoyalCapture":
                continue
            last = None
            for _, _, board, turn in replay(game, opening):
                if turn["side"] == candidate:
                    last = (board.material(candidate), (turn.get("evaluation") or {}).get("score"))
            if last is None:
                continue
            final_material.append(last[0])
            if last[1] is not None and last[1]["kind"] == "cp":
                final_probability.append(win_probability(last[1]["value"]))
    print(f"{run.rsplit('/', 1)[-1]}: games {len(plies)}; plies median {st.median(plies)}; endings {sorted(endings.items())}")
    if final_material:
        print(f"  royal-capture losses {len(final_material)}: material for MCTS median {st.median(final_material)}, "
              f"share >= 0 {sum(m >= 0 for m in final_material) / len(final_material):.3f}, "
              f"share >= -20 {sum(m >= -20 for m in final_material) / len(final_material):.3f}; "
              f"MCTS win probability median {st.median(final_probability):.3f}, "
              f"share >= 0.5 {sum(p >= 0.5 for p in final_probability) / len(final_probability):.3f}")
