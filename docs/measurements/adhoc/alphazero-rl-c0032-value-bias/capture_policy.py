"""αβが実際に指した捕獲の局面で、サイクル32のネットが同じ種類の捕獲を選ぶかを調べる。

引数はminaseのバイナリ、ネット、STCの実行ディレクトリである。
`go nodes 1`は方策の最大の手を返す。選んだ手の効果を盤で判定するので、別の駒で取る手も数える。
"""

import random
import statistics as st
import sys

from common import final_square, is_royal, load_pair, opening_moves, pair_files, piece_value, replay, win_probability
from engine import Engine

binary, net, run = sys.argv[1:4]
royal, big, quiet = [], [], []
rng = random.Random(20261104)
for path in pair_files(run):
    pair = load_pair(path)
    opening = opening_moves(pair)
    for game in pair["games"]:
        candidate = game["candidate_color"]
        end = game["termination"]
        turns = [t for t in game["turns"] if t["response"].get("kind") == "move"]
        for index, (_, history, board, turn) in enumerate(replay(game, opening)):
            if turn["side"] == candidate:
                continue
            usi = turn["response"]["usi"]
            after = board.copy()
            captured = after.play(usi, turn["side"])
            total = sum(piece_value(c) for c in captured)
            entry = (list(history), usi, board.copy(), turn["side"])
            if index == len(turns) - 1:
                if end["reason"] == "RoyalCapture" and end.get("winner") != candidate:
                    royal.append(entry)
            elif total >= 8:
                capturer = after.pieces[final_square(usi)]
                reply = after.play(turns[index + 1]["response"]["usi"], turns[index + 1]["side"])
                if not any(c is capturer for c in reply):
                    big.append(entry)
            elif total == 0 and rng.random() < 0.01:
                quiet.append(entry)

engine = Engine(binary, net, 16)


def report(name, positions, limit, wanted):
    sample = rng.sample(positions, min(limit, len(positions)))
    for nodes in (1, 800):
        part = sample if nodes == 1 else sample[:200]
        same = effect = 0
        values = []
        for history, usi, board, side in part:
            engine.send("usinewgame")
            engine.send("position startpos moves " + " ".join(history))
            engine.send(f"go nodes {nodes}")
            lines = engine.until("bestmove")
            best = lines[-1].split()[1]
            same += best == usi
            effect += wanted(board.copy().play(best, side))
            info = [line for line in lines if " score cp " in line]
            if info:
                values.append(win_probability(int(info[-1].split(" score cp ")[1].split()[0])))
        median = f"{st.median(values):.3f}" if values else "none"
        print(f"{name} nodes={nodes}: same move {same} of {len(part)}; wanted capture {effect} of {len(part)} "
              f"({effect / len(part):.3f}); side-to-move win probability median {median} (n={len(values)})", flush=True)


report("royal capture", royal, 200, lambda captured: any(is_royal(c) for c in captured))
report("unanswered capture of value >= 8", big, 600, lambda captured: sum(piece_value(c) for c in captured) >= 8)
report("alpha-beta quiet move", quiet, 600, lambda captured: not captured)
engine.quit()
