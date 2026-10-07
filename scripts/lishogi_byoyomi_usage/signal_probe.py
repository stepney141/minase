#!/usr/bin/env python3
"""持ち時間期の局面で、深さ6への到達率と難しさの信号の発動率を数える診断。

使い方: signal_probe.py <実行ディレクトリ> <エンジン> [局面数] [pawn_value]
実行ディレクトリの対局記録から候補側の持ち時間期の手を一様に選び、着手前の持ち時間を
記録から再現して、同じ局面と時計を`go btime/wtime/byoyomi`でエンジンに与える。
主ワーカーの完了反復（`info depth`行、lowerbound を除く）の最善手と評価値から、
設計書の「難しさの信号」を最後の完了反復で計算する。
"""
import json
import pathlib
import random
import subprocess
import sys

MATE_THRESHOLD_TEXT = "mate"


def main_phase_positions(run_dir, count, seed=1):
    run = pathlib.Path(run_dir)
    limit = json.loads((run / "manifest.json").read_text())["candidate"]["limit"]
    base, byoyomi = limit["base_ms"], limit["byoyomi_ms"]
    samples = []
    for path in sorted((run / "pairs").glob("*.json")):
        record = json.loads(path.read_text())
        opening = list(record["opening"]["moves"])
        for game in record["games"]:
            moves = list(opening)
            remaining = {}
            for turn in game["turns"]:
                side = turn["side"]
                before = remaining.setdefault(side, base)
                if side == game["candidate_color"] and before > 0:
                    samples.append((list(moves), int(before)))
                remaining[side] = max(0.0, before - turn["think_time_ns"] / 1e6)
                if turn["response"]["kind"] != "move":
                    break
                moves.append(turn["response"]["usi"])
    random.Random(seed).shuffle(samples)
    return samples[:count], byoyomi


def iterations(lines):
    result = []
    for line in lines:
        if not line.startswith("info depth") or " lowerbound" in line:
            continue
        tokens = line.split()
        depth = int(tokens[2])
        kind, value = tokens[4], tokens[5]
        score = None if kind == MATE_THRESHOLD_TEXT else int(value)
        best = tokens[tokens.index("pv") + 1] if "pv" in tokens else None
        if result and result[-1][0] == depth:
            result[-1] = (depth, best, score)
        else:
            result.append((depth, best, score))
    return result


def signal(its, pawn_value):
    if len(its) < 6 or its[-1][0] < 6:
        return False
    bests = [best for _, best, _ in its]
    changed = bests[-1] != bests[-2] or bests[-2] != bests[-3]
    current, earlier = its[-1][2], its[-3][2]
    dropped = current is not None and earlier is not None and earlier - current >= pawn_value
    return changed or dropped


def main(run_dir, engine, count=200, pawn_value=100):
    samples, byoyomi = main_phase_positions(run_dir, int(count))
    proc = subprocess.Popen([engine, "--protocol", "usi", "--rules", "engine-default"],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)

    def send(line):
        proc.stdin.write(line + "\n")

    def wait(token):
        lines = []
        while True:
            line = proc.stdout.readline().strip()
            lines.append(line)
            if line.startswith(token):
                return lines

    send("usi"); wait("usiok"); send("isready"); wait("readyok")
    reached = fired = 0
    for moves, remaining in samples:
        send("usinewgame")
        send("position startpos" + (" moves " + " ".join(moves) if moves else ""))
        send(f"go btime {remaining} wtime {remaining} byoyomi {byoyomi}")
        its = iterations(wait("bestmove"))
        reached += bool(its) and its[-1][0] >= 6
        fired += signal(its, int(pawn_value))
    send("quit")
    n = len(samples)
    print(f"positions={n} depth>=6 rate={reached / n:.3f} signal rate={fired / n:.3f}")


if __name__ == "__main__":
    main(*sys.argv[1:])
