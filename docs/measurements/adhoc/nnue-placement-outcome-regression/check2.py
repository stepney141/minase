"""打ち切りを除き、先後の両方の局面を取り、現行ネットの静的評価を付けて保存する。"""
import glob, json, os, pickle, re, subprocess, tempfile
import numpy as np
exec(open("extract.py").read().split("rows = []")[0].replace("RUNS = sys.argv[1:]", ""))
BIN = "../../../../target/match-cache/6d47da2a9663918f8992290834d9c47e695de98f/minase"
RUNS = ["nnue-material-linear-init-noninferiority-ltc", "nnue-material-linear-init-noninferiority-stc"]
SNAPS = [(60, 81, 100, 121, 140), (61, 80, 101, 120, 141)]
def emit(board):
    out = []
    for r in range(12):
        row = ""; e = 0
        for f in range(12, 0, -1):
            pc = board.get((f, r))
            if pc is None: e += 1; continue
            if e: row += str(e); e = 0
            k, side, prom = pc
            ch = k.upper() if side == 0 else k
            row += ("+" if prom else "") + ch
        if e: row += str(e)
        out.append(row)
    return "/".join(out)
rows = []; dropped = 0
for run in RUNS:
    for fp in sorted(glob.glob(os.path.join(ROOT, run, "pairs", "*.json"))):
        p = json.load(open(fp))
        op = p["opening"]["moves"]
        for gi, g in enumerate(p["games"]):
            term = g["termination"]
            if term.get("kind") == "cutoff": dropped += 1; continue
            w = term.get("winner")
            draw = term.get("kind") == "adjudicated_draw"
            assert draw or w in ("black", "white"), term
            snap = SNAPS[(p["pair_number"] + gi) % 2]
            board = parse(START)
            seq = [(m, None) for m in op] + [(t["response"].get("usi"), t) for t in g["turns"] if t["response"].get("kind") == "move"]
            for n, (mv, t) in enumerate(seq):
                if n in snap and t is not None:
                    ev = t.get("evaluation") or {}; sc = ev.get("score") or {}
                    stm = 0 if t["side"] == "black" else 1
                    assert ev.get("perspective") in (None, t["side"]), ev
                    if sc.get("kind") == "cp" and abs(sc["value"]) < 3000:
                        z = 0.5 if draw else (1.0 if (w == "black") == (stm == 0) else 0.0)
                        x5 = np.zeros(5, dtype=np.int8); xf = np.zeros(D, dtype=np.int8); xm = np.zeros(42, dtype=np.int8)
                        for (f, r), (k, side, prom) in board.items():
                            s = 1 if side == stm else -1
                            i = KI[k]
                            if prom: xm[21 + i] += s; continue
                            xm[i] += s
                            of, or_ = own(side, f, r)
                            xf[i * 7 + RB(or_)] += s
                            xf[21 * 7 + i * 6 + FB(of)] += s
                            if k == "q" and or_ <= 2: x5[0] += s
                            if k == "h" and or_ <= 2: x5[1] += s
                            if k == "e" and 3 <= of <= 5: x5[2] += s
                            if k == "g" and or_ >= 3: x5[3] += s
                            if k == "o" and of <= 2: x5[4] += s
                        sfen = f"{emit(board)} {'b' if stm == 0 else 'w'} - 1"
                        rows.append(dict(pair=f"{run}:{p['pair_number']}", game=f"{run}:{p['pair_number']}:{gi}", run=run, ply=n, stm=stm, search=sc["value"], z=z, x5=x5, xf=xf, xm=xm, sfen=sfen))
                if mv is None or n >= 141: break
                sq = [(int(a), RANKS.index(b)) for a, b in SQ.findall(mv)]
                frm, to = sq[0], sq[-1]
                pc = board.pop(frm)
                for m in sq[1:-1]:
                    if m != frm: board.pop(m, None)
                board.pop(to, None)
                if mv.endswith("+"): pc = (pc[0], pc[1], True)
                board[to] = pc
    print(run, len(rows), "cutoff", dropped, flush=True)
def usi_eval(sfens):
    with tempfile.TemporaryFile("w+", encoding="utf-8") as c:
        c.write("usi\nisready\nusinewgame\n")
        for s in sfens: c.write(f"position sfen {s}\neval\n")
        c.write("quit\n"); c.seek(0)
        out = subprocess.run([BIN, "--protocol", "usi", "--rules", "engine-default"], stdin=c, stdout=subprocess.PIPE, text=True, check=True).stdout
    vals = [int(l.split()[-1]) for l in out.splitlines() if l.startswith("info string evaluation ")]
    assert len(vals) == len(sfens), (len(vals), len(sfens))
    return vals
vals = usi_eval([r["sfen"] for r in rows])
for r, v in zip(rows, vals): r["static"] = v
pickle.dump(rows, open("../../../../data/nnue-placement-outcome-regression/rows2.pkl", "wb"))
s = np.array(vals, float); t = np.array([r["search"] for r in rows], float)
print("局面", len(rows), "先手番の割合", np.mean([r["stm"] == 0 for r in rows]), "静的評価と探索値の相関", np.corrcoef(s, t)[0, 1], "静的評価の標準偏差", s.std(), "探索値の標準偏差", t.std())
