#!/usr/bin/env python3
"""同じ局面で2つのエンジンの完了深さと途中結果を比べる診断。

使い方: paired_depth_probe.py <実行ディレクトリ> <基準のエンジン> <候補のエンジン> <go引数> [局面数]
実行ディレクトリの対局記録から手数0〜200の局面を一様に選び、各エンジンへ
`position startpos moves ...`と指定の`go`引数（例: "btime 40000 wtime 40000 byoyomi 300"）
を与え、最後の`info depth`の深さ、`info string partial`の有無、および
途中結果の着手が直前の完了反復の主変化の先頭と異なるかを記録する。
"""
import json
import pathlib
import random
import statistics
import subprocess
import sys


def positions(run_dir, count, seed=1):
    games = []
    for path in sorted(pathlib.Path(run_dir, "pairs").glob("*.json")):
        record = json.loads(path.read_text())
        opening = list(record["opening"]["moves"])
        for game in record["games"]:
            moves = opening + [turn["response"]["usi"] for turn in game["turns"] if turn["response"]["kind"] == "move"]
            games.append((len(opening), moves))
    rng = random.Random(seed)
    picked = []
    while len(picked) < count:
        opening_plies, moves = rng.choice(games)
        ply = rng.randrange(opening_plies, min(200, len(moves)))
        picked.append(moves[:ply])
    return picked


class Engine:
    def __init__(self, path):
        self.proc = subprocess.Popen(
            [path, "--protocol", "usi", "--rules", "engine-default"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
        )
        self.send("usi"); self.wait("usiok")
        self.send("isready"); self.wait("readyok")

    def send(self, line):
        self.proc.stdin.write(line + "\n")

    def wait(self, token):
        lines = []
        while True:
            line = self.proc.stdout.readline().strip()
            lines.append(line)
            if line.startswith(token):
                return lines

    def search(self, moves, go):
        self.send("usinewgame")
        self.send("position startpos" + (" moves " + " ".join(moves) if moves else ""))
        self.send("go " + go)
        lines = self.wait("bestmove")
        depth, last_pv_head, partial = 0, None, False
        for line in lines:
            if line == "info string partial":
                partial = True
            elif line.startswith("info depth") and " lowerbound" not in line:
                depth = int(line.split()[2])
                tokens = line.split()
                last_pv_head = tokens[tokens.index("pv") + 1] if "pv" in tokens and tokens.index("pv") + 1 < len(tokens) else None
        best = lines[-1].split()[1]
        return depth, partial, partial and best != last_pv_head


def main(run_dir, baseline, candidate, go, count=200):
    picked = positions(run_dir, int(count))
    engines = {"baseline": Engine(baseline), "candidate": Engine(candidate)}
    rows = []
    for moves in picked:
        row = {}
        for name, engine in engines.items():
            row[name] = engine.search(moves, go)
        rows.append(row)
    diffs = [row["candidate"][0] - row["baseline"][0] for row in rows]
    for name in engines:
        depths = [row[name][0] for row in rows]
        partial = sum(row[name][1] for row in rows)
        changed = sum(row[name][2] for row in rows)
        print(f"{name:9} mean depth={statistics.mean(depths):.3f} partial={partial}/{len(rows)} changed={changed}")
    se = statistics.stdev(diffs) / len(diffs) ** 0.5
    print(f"paired depth diff (candidate - baseline): mean={statistics.mean(diffs):+.3f} se={se:.3f} n={len(diffs)}")


if __name__ == "__main__":
    main(*sys.argv[1:])
