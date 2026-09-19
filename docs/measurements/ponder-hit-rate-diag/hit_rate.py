#!/usr/bin/env python3
"""先読みを実装する前に、予想手（PVの2手目）の的中率を見積もる診断。

minaseを2プロセス起動して時間制御つきで自己対局させ、各着手の最後のinfo行のPVの
2手目が、相手の次の実着手と一致した割合を数える。先読みは行わない。
"""
import argparse, json, random, subprocess, sys, time
from concurrent.futures import ThreadPoolExecutor


class Engine:
    def __init__(self, binary, rules):
        self.p = subprocess.Popen([binary, "--protocol", "usi", "--rules", rules],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        self.send("usi"); self.until("usiok")
        self.send("isready"); self.until("readyok")
        self.send("usinewgame")

    def send(self, line):
        self.p.stdin.write(line + "\n"); self.p.stdin.flush()

    def until(self, head):
        lines = []
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("engine closed")
            line = line.strip(); lines.append(line)
            if line.split(" ", 1)[0] == head:
                return lines

    def close(self):
        try:
            self.send("quit"); self.p.wait(timeout=5)
        except Exception:
            self.p.kill()


def position(moves):
    return "position startpos" + (" moves " + " ".join(moves) if moves else "")


def play(args, game_seed):
    rng = random.Random(game_seed)
    engines = [Engine(args.binary, args.rules), Engine(args.binary, args.rules)]
    clocks = [args.base, args.base]
    moves, turns, status = [], [], "ongoing"
    opening = rng.randint(8, 12)
    try:
        while len(moves) < args.max_ply:
            side = len(moves) % 2
            e = engines[side]
            e.send(position(moves)); e.send("state")
            status = e.until("state")[-1].split(" status ", 1)[1]
            if status != "ongoing":
                break
            if len(moves) < opening:
                e.send("moves")
                legal = e.until("moves")[-1].split()[1:]
                moves.append(rng.choice(legal)); turns.append(None)
                continue
            b, w = clocks
            start = time.monotonic()
            e.send(f"go btime {b} wtime {w} binc {args.inc} winc {args.inc}")
            lines = e.until("bestmove")
            elapsed = int((time.monotonic() - start) * 1000)
            if elapsed > clocks[side]:
                status = "time forfeit"; break
            clocks[side] += args.inc - elapsed
            pv, depth = [], 0
            for line in lines:
                t = line.split()
                if t[:1] == ["info"] and "pv" in t:
                    pv = t[t.index("pv") + 1:]; depth = int(t[t.index("depth") + 1])
            best = lines[-1].split()[1]
            turns.append({"ply": len(moves), "best": best, "depth": depth, "elapsed_ms": elapsed,
                          "pv_matches_best": bool(pv) and pv[0] == best,
                          "predicted": pv[1] if len(pv) > 1 and pv[0] == best else None})
            moves.append(best)
    finally:
        for e in engines:
            e.close()
    for i, t in enumerate(turns):
        if t is not None:
            t["next"] = moves[i + 1] if i + 1 < len(moves) else None
    return {"seed": game_seed, "plies": len(moves), "status": status,
            "turns": [t for t in turns if t is not None]}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True); ap.add_argument("--rules", default="engine-default")
    ap.add_argument("--base", type=int, required=True); ap.add_argument("--inc", type=int, required=True)
    ap.add_argument("--games", type=int, required=True); ap.add_argument("--seed", type=int, required=True)
    ap.add_argument("--concurrency", type=int, default=8); ap.add_argument("--max-ply", type=int, default=4096)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    seeds = [args.seed + 1000 * i for i in range(args.games)]
    with ThreadPoolExecutor(args.concurrency) as pool:
        games = list(pool.map(lambda s: play(args, s), seeds))
    json.dump({"args": vars(args), "games": games}, open(args.out, "w"))
    print("done", len(games), file=sys.stderr)


if __name__ == "__main__":
    main()
