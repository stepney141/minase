#!/usr/bin/env python3
"""HaChu戦で負けた局の崩れた局面を再生し、盤面差分と深い再探索を集める。"""
import json, glob, subprocess, sys, re
from concurrent.futures import ThreadPoolExecutor
BIN = "/home/stepney141/board-games/minase/target/release/minase"
RULES = "L1,L3,P0,P5,P6,R2,E1,E2"
RUN = "/home/stepney141/board-games/minase/data/matches/strength-stage12-hachu-elo200"
SKIP = {58, 111, 118}
MOVETIME = int(sys.argv[1]) if len(sys.argv) > 1 else 30000

class Engine:
    def __init__(self, threads=2, hash_mb=1024):
        self.p = subprocess.Popen([BIN, "--protocol", "usi", "--rules", RULES], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        self.send("usi"); self.until("usiok")
        self.send(f"setoption name Threads value {threads}"); self.send(f"setoption name USI_Hash value {hash_mb}")
        self.send("setoption name ResignValue value 99999")
        self.send("isready"); self.until("readyok")
    def send(self, s): self.p.stdin.write(s + "\n"); self.p.stdin.flush()
    def until(self, tok):
        out = []
        while True:
            l = self.p.stdout.readline()
            if not l: raise RuntimeError("engine died")
            out.append(l.rstrip("\n"))
            if l.startswith(tok): return out
    def pos(self, moves):
        self.send("position startpos" + (" moves " + " ".join(moves) if moves else ""))
    def board(self, moves):
        self.pos(moves); self.send("d"); lines = self.until("rights-zobrist")
        cells = {}
        rows = [l for l in lines if l.startswith("|")]
        files = list(range(12, 0, -1))
        for r, l in enumerate(rows):
            parts = l.strip("|").split("|")[:12]
            rank = "abcdefghijkl"[r]
            for f, c in zip(files, parts):
                c = c.strip()
                if c and c != "・": cells[f"{f}{rank}"] = c
        side = next(l.split()[1] for l in lines if l.startswith("side "))
        return cells, side
    def search(self, moves, movetime):
        self.pos(moves);
        self.send(f"go movetime {movetime}")
        lines = self.until("bestmove")
        info = [l for l in lines if l.startswith("info depth")]
        last = info[-1] if info else ""
        m = re.search(r"score (cp|mate) (-?\d+)", last); d = re.search(r"depth (\d+)", last); pv = re.search(r" pv (.*)$", last)
        return {"best": lines[-1].split()[1], "score": (m.group(1), int(m.group(2))) if m else None,
                "depth": int(d.group(1)) if d else None, "pv": pv.group(1).split()[:8] if pv else []}
    def evaluate(self, moves):
        self.pos(moves); self.send("eval"); lines = self.until("info string end")
        return int(next(l.split()[-1] for l in lines if l.startswith("info string evaluation")))
    def quit(self): self.send("quit"); self.p.wait(timeout=10)

def cp(ev, side):
    s = ev["score"]
    if s["kind"] != "cp": return None
    return s["value"] if ev["perspective"] == side else -s["value"]

def games():
    for f in sorted(glob.glob(f"{RUN}/pairs/*.json")):
        p = json.load(open(f)); op = list(p["opening"]["moves"])
        for gi, g in enumerate(p["games"]):
            t = g["termination"]; c = g["candidate_color"]
            if not (t["kind"] == "adjudicated_win" and t.get("winner") != c): continue
            if p["pair_number"] in SKIP: continue
            moves = op + [tu["response"]["usi"] for tu in g["turns"] if tu["response"]["kind"] == "move"]
            evs = []
            for i, tu in enumerate(g["turns"]):
                if tu["side"] == c and tu["evaluation"]:
                    v = cp(tu["evaluation"], c)
                    if v is not None: evs.append((len(op) + i, v, tu["evaluation"]["depth"]))
            best = None
            for a, b in zip(evs, evs[1:]):
                if a[1] > -300 and b[0] == a[0] + 2:
                    dropv = a[1] - b[1]
                    if best is None or dropv > best[0]: best = (dropv, a, b)
            yield {"pair": p["pair_number"], "game": gi, "color": c, "moves": moves, "drop": best, "plies": len(moves)}

def analyze(g):
    e = Engine()
    _, a, b = g["drop"]; k = a[0]; mv = g["moves"]
    b0, side0 = e.board(mv[:k]); b1, _ = e.board(mv[:k + 1]); b2, _ = e.board(mv[:k + 2])
    m, r = mv[k], mv[k + 1]
    res = {"pair": g["pair"], "game": g["game"], "color": g["color"], "ply": k, "plies": g["plies"],
           "eval_before": a[1], "depth_before": a[2], "eval_after": b[1], "depth_after": b[2],
           "minase_move": m, "hachu_reply": r}
    mine = lambda pc: (not pc.startswith("^")) if g["color"] == "black" else pc.startswith("^")
    res["minase_royals"] = [f"{sq}:{pc}" for sq, pc in b2.items() if mine(pc) and pc.lstrip("^ ").lstrip("+") in ("王", "玉", "太")]
    res["deep_p0"] = e.search(mv[:k], MOVETIME)
    res["deep_p1"] = e.search(mv[:k + 1], MOVETIME)
    res["deep_p2"] = e.search(mv[:k + 2], MOVETIME)
    res["static_p0"] = e.evaluate(mv[:k]); res["static_p2"] = e.evaluate(mv[:k + 2])
    e.quit()
    return res

if __name__ == "__main__":
    gs = list(games())
    with ThreadPoolExecutor(max_workers=8) as ex:
        out = list(ex.map(analyze, gs))
    json.dump(out, open(sys.argv[2] if len(sys.argv) > 2 else "results.json", "w"), ensure_ascii=False, indent=1)
    for r in out:
        print(json.dumps({k: r[k] for k in ("pair", "ply", "eval_before", "eval_after", "minase_move", "hachu_reply", "minase_royals")}, ensure_ascii=False))
        print("   deep_p0", r["deep_p0"], "\n   deep_p1", r["deep_p1"], "\n   deep_p2", r["deep_p2"], "static", r["static_p0"], r["static_p2"])
