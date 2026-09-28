#!/usr/bin/env python3
"""C5の保存記録から局面を抽出し、標準時間制御で段階12の計数を集計する。"""

import argparse
import bisect
from collections import Counter
import json
import os
from pathlib import Path
import random
import selectors
import subprocess
import time

BANDS = [50, 100, 150, 200, 300, 400]
REMAINING = {
    "stc": [9690, 8745, 7524, 6173, 4101, 2025, 2654],
    "ltc": [57782, 51999, 44835, 37079, 25269, 13534, 7523],
}
INCREMENT = {"stc": 100, "ltc": 200}
MATCH_ROOT = Path("/home/stepney141/board-games/minase/data/matches")


def sample_positions(run_dir, count, seed):
    """適格な全対局から非復元抽出し、各対局の[10, 最終手数-1]から一様抽出する。"""
    files = sorted((run_dir / "pairs").glob("*.json"))
    if not files:
        raise ValueError(f"pair records not found: {run_dir}")
    # 対局の候補は識別子だけ保持し、選ばれた棋譜を後で読み直す。
    games = []
    for path in files:
        pair = json.loads(path.read_text())
        for index, game in enumerate(pair["games"]):
            total = len(pair["opening"]["moves"]) + sum(
                turn["response"]["kind"] == "move" for turn in game["turns"]
            )
            if total > 10:
                games.append((path, index, total))
    rng = random.Random(seed)
    selected = rng.sample(games, count)
    positions = []
    for path, index, total in selected:
        pair = json.loads(path.read_text())
        ply = rng.randint(10, total - 1)
        moves = list(pair["opening"]["moves"]) + [
            turn["response"]["usi"]
            for turn in pair["games"][index]["turns"]
            if turn["response"]["kind"] == "move"
        ]
        assert len(moves) == total
        positions.append({"pair": pair["pair_number"], "game": index,
                          "record": str(path), "ply": ply, "moves": moves[:ply]})
    return positions


class Engine:
    """期限つきのUSI入出力。文字列バッファ内の行をselectで取りこぼさない。"""

    def __init__(self, binary):
        self.proc = subprocess.Popen(
            [str(binary), "--protocol", "usi", "--rules", "L0,P0,R1,E0"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            env={**os.environ, "MINASE_STAGE12_DIAG": "1"},
        )
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.proc.stdout, selectors.EVENT_READ)
        self.buffer = b""
        self.transcript = []

    def send(self, command):
        self.transcript.append("> " + command)
        self.proc.stdin.write((command + "\n").encode())
        self.proc.stdin.flush()

    def line(self, deadline):
        while b"\n" not in self.buffer:
            seconds = deadline - time.monotonic()
            if seconds <= 0 or not self.selector.select(seconds):
                raise TimeoutError("engine response timeout")
            chunk = os.read(self.proc.stdout.fileno(), 65536)
            if not chunk:
                raise RuntimeError("engine closed stdout")
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b"\n", 1)
        text = line.decode().strip()
        self.transcript.append("< " + text)
        if text.startswith("info string error"):
            raise RuntimeError(text)
        return text

    def handshake(self, command, token):
        self.send(command)
        deadline = time.monotonic() + 15
        while self.line(deadline) != token:
            pass

    def close(self):
        self.selector.close()
        if self.proc.poll() is None:
            self.proc.kill()
        self.proc.communicate(timeout=10)


def run_position(binary, position, tc):
    engine = Engine(binary)
    try:
        engine.handshake("usi", "usiok")
        engine.handshake("isready", "readyok")
        engine.send("usinewgame")
        engine.send("position startpos moves " + " ".join(position["moves"]))
        remaining = REMAINING[tc][bisect.bisect_right(BANDS, position["ply"])]
        increment = INCREMENT[tc]
        engine.send(f"go btime {remaining} wtime {remaining} binc {increment} winc {increment}")
        deadline = time.monotonic() + 30
        diag = None
        depth = 0
        stop = None
        previous_line = None
        while True:
            line = engine.line(deadline)
            if line.startswith("info string diag "):
                if diag is not None:
                    raise RuntimeError("duplicate diagnostic line")
                diag = dict((key, int(value)) for key, value in
                            (token.split("=", 1) for token in line.split()[3:]))
            elif line.startswith("info depth "):
                depth = int(line.split()[2])
            elif line.startswith("info string stop "):
                stop = line.split()[3]
            elif line.startswith("bestmove "):
                if diag is None or not previous_line.startswith("info string diag "):
                    raise RuntimeError("diagnostic line must immediately precede bestmove")
                bestmove = line.split()[1]
                break
            previous_line = line
        if stop is None:
            raise RuntimeError("missing stop reason")
        engine.send("quit")
        _, stderr = engine.proc.communicate(timeout=10)
        if engine.proc.returncode != 0:
            raise RuntimeError(stderr.decode())
        return {**position, "remaining_ms": remaining, "increment_ms": increment,
                "depth": depth, "stop": stop, "bestmove": bestmove, "diag": diag,
                "transcript": engine.transcript}
    finally:
        engine.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--tc", choices=REMAINING, required=True)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--seed", type=int, default=20260927)
    parser.add_argument("--run-dir", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.count < 1:
        parser.error("--count must be positive")
    run_dir = args.run_dir if args.run_dir is not None else MATCH_ROOT / f"spsa-stage9-20260925-c5-{args.tc}"
    positions = sample_positions(run_dir, args.count, args.seed)
    results = []
    for index, position in enumerate(positions, 1):
        result = run_position(args.binary.resolve(), position, args.tc)
        results.append(result)
        print(f"{index}/{len(positions)} pair={position['pair']} game={position['game']} "
              f"ply={position['ply']} depth={result['depth']} stop={result['stop']}", flush=True)
    keys = results[0]["diag"].keys()
    if any(result["diag"].keys() != keys for result in results):
        raise RuntimeError("diagnostic key sets differ between searches")
    totals = {key: sum(result["diag"][key] for result in results) for key in keys}
    report = {"tc": args.tc, "seed": args.seed, "rules": "L0,P0,R1,E0", "threads": 1,
              "run_dir": str(run_dir), "binary": str(args.binary.resolve()),
              "totals": totals, "positions": results}
    args.out.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print("stops", dict(Counter(result["stop"] for result in results)))
    print("totals", json.dumps(totals, sort_keys=True))


if __name__ == "__main__":
    main()
