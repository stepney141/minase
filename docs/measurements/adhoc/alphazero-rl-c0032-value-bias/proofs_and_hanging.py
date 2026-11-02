"""MCTSの根の確定と、王駒を取られる直前の手を調べる。

引数はminaseのバイナリ、ネット、STCの実行ディレクトリである。
前半は記録だけから、根が確定して探索を止めた手の数を集計する。
後半は、王駒の捕獲で負けた対局の最後のMCTSの手番を、MCTSの探索回数を変えて指し直し、
選んだ手の直後に相手が王駒を取れるかをαβの深さ2の探索で確かめる。
"""

import re
import statistics as st
import subprocess
import sys

from common import load_pair, opening_moves, pair_files, replay, win_probability
from engine import Engine

binary, net, run = sys.argv[1:4]
stops = {}
false_win = []
last = []
for path in pair_files(run):
    pair = load_pair(path)
    opening = opening_moves(pair)
    for game in pair["games"]:
        candidate = game["candidate_color"]
        end = game["termination"]
        final = None
        for ply, history, board, turn in replay(game, opening):
            if turn["side"] != candidate:
                continue
            stops[turn.get("stop_reason")] = stops.get(turn.get("stop_reason"), 0) + 1
            score = (turn.get("evaluation") or {}).get("score")
            if turn.get("stop_reason") == "depth" and score and score["kind"] == "cp" and score["value"] >= 20000:
                false_win.append((path[-10:], ply, len(game["turns"]) + len(opening), end.get("winner") == candidate))
            final = (ply, list(history), score, board.material(candidate), turn["response"]["usi"])
        if end["reason"] == "RoyalCapture" and end.get("winner") != candidate and final is not None:
            last.append(final)
print("MCTS stop reasons:", stops)
print("root stopped with a claimed win of at least 20000 cp:", len(false_win),
      "games", len({x[0] for x in false_win}), "of which the MCTS side won:", sum(x[3] for x in false_win))
print("examples (pair, ply, game length, MCTS won):", false_win[:6])
values = [(win_probability(x[2]["value"]) if x[2] and x[2]["kind"] == "cp" else None) for x in last]
known = [v for v in values if v is not None]
print(f"royal-capture losses: {len(last)}; final MCTS win probability median {st.median(known):.3f}; "
      f"share >= 0.5 {sum(v >= 0.5 for v in known) / len(known):.3f}; share >= 0.2 {sum(v >= 0.2 for v in known) / len(known):.3f}")
print("material for MCTS at the final move: median", st.median(x[3] for x in last))

ab = subprocess.Popen([binary, "--protocol", "usi", "--rules", "engine-default"], stdin=subprocess.PIPE,
                      stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)


def send(line):
    ab.stdin.write(line + "\n")
    ab.stdin.flush()


def until(token):
    lines = []
    while True:
        line = ab.stdout.readline()
        if not line:
            raise RuntimeError("alpha-beta stopped: " + "".join(lines[-5:]))
        lines.append(line)
        if line.startswith(token):
            return lines


send("usi")
until("usiok")
send("isready")
until("readyok")


def alpha_beta_mate(moves, depth):
    """手番側から見たmateの(符号, 手数)を返す。詰みの読みがなければNoneである。

    手番側が最後の王駒をすぐ取れる局面は`mate +0`と出力される。
    """
    send("usinewgame")
    send("position startpos moves " + " ".join(moves))
    send(f"go depth {depth}")
    info = [line for line in until("bestmove") if " score " in line]
    found = re.search(r"score mate ([+-])(\d+)", info[-1]) if info else None
    return (found[1], int(found[2])) if found else None


def leaves_royal_capturable(moves):
    reply = alpha_beta_mate(moves, 2)
    return reply is not None and reply[0] == "+" and reply[1] <= 1


control = sum(leaves_royal_capturable(x[1] + [x[4]]) for x in last)
print(f"control: the detector marks {control} of {len(last)} actual final moves")
mcts = Engine(binary, net, 16)
hopeless = 0
counts = {n: [0, 0] for n in (2000, 20000)}
searched = {n: [] for n in counts}
proven_loss = {n: 0 for n in counts}
for ply, history, score, material, _ in last:
    defended = alpha_beta_mate(history, 8)
    forced = defended is not None and defended[0] == "-"
    hopeless += forced
    for nodes in counts:
        mcts.send("usinewgame")
        mcts.send("position startpos moves " + " ".join(history))
        mcts.send(f"go nodes {nodes}")
        lines = mcts.until("bestmove")
        best = lines[-1].split()[1]
        info = [line for line in lines if " nodes " in line][-1]
        searched[nodes].append(int(re.search(r"nodes (\d+)", info)[1]))
        # 根の負けの確定は`score mate -`で出力される。確定した根は探索を止める。
        if "score mate -" in info:
            proven_loss[nodes] += 1
        hanging = leaves_royal_capturable(history + [best])
        counts[nodes][0] += hanging
        if not forced:
            counts[nodes][1] += hanging
print(f"positions {len(last)}; forced loss seen by alpha-beta at depth 8: {hopeless}; not forced: {len(last) - hopeless}")
for nodes, (all_hanging, unforced_hanging) in counts.items():
    print(f"nodes {nodes}: leaves the royal capturable in {all_hanging} of {len(last)}; "
          f"among the positions without a forced loss {unforced_hanging} of {len(last) - hopeless}; "
          f"root proven lost {proven_loss[nodes]}; simulations median {st.median(searched[nodes])}, "
          f"stopped before the limit {sum(n < nodes for n in searched[nodes])}")
send("quit")
mcts.quit()
