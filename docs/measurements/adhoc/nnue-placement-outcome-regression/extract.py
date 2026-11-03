"""保存済みの対局から、手番側視点の探索値、配置、および結果を抽出する。"""
import glob, json, os, pickle, re, sys
import numpy as np
ROOT = "../../../../data/matches"
RUNS = sys.argv[1:]
START = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL"
SQ = re.compile(r"(\d{1,2})([a-l])")
RANKS = "abcdefghijkl"
KINDS = "lfcsgekabtxomvrhdqnpi"
KI = {k: i for i, k in enumerate(KINDS)}
def parse(b):
    board = {}
    for r, row in enumerate(b.split("/")):
        f = 12; i = 0
        while i < len(row):
            ch = row[i]
            if ch.isdigit():
                j = i
                while j < len(row) and row[j].isdigit(): j += 1
                f -= int(row[i:j]); i = j; continue
            board[(f, r)] = (ch.lower(), 0 if ch.isupper() else 1, False)
            f -= 1; i += 1
    return board
def own(side, f, r):
    return (f, 12 - r) if side == 0 else (13 - f, r + 1)
FB = lambda f: int(abs(f - 6.5) - 0.5)  # 0..5 中央からの距離
RB = lambda r: min(r - 1, 6)  # 0..6（7段以上をまとめる）
D = 21 * 7 + 21 * 6
SNAP = (60, 80, 100, 120, 140)
rows = []
for run in RUNS:
    for fp in sorted(glob.glob(os.path.join(ROOT, run, "pairs", "*.json"))):
        p = json.load(open(fp))
        op = p["opening"]["moves"]
        for gi, g in enumerate(p["games"]):
            term = g["termination"]
            w = term.get("winner")
            if w not in ("black", "white", None): continue
            turns = g["turns"]
            board = parse(START)
            seq = [(m, None) for m in op] + [(t["response"].get("usi"), t) for t in turns if t["response"].get("kind") == "move"]
            for n, (mv, t) in enumerate(seq):
                # n手指した後の局面で、次の手番がtの探索値を出す
                if n in SNAP and t is not None:
                    ev = t.get("evaluation") or {}
                    sc = ev.get("score") or {}
                    if sc.get("kind") == "cp" and abs(sc["value"]) < 3000:
                        stm = 0 if t["side"] == "black" else 1
                        z = 0.5 if w is None else (1.0 if (w == "black") == (stm == 0) else 0.0)
                        x5 = np.zeros(5, dtype=np.int8)
                        xf = np.zeros(D, dtype=np.int8)
                        for (f, r), (k, side, prom) in board.items():
                            if prom: continue
                            of, or_ = own(side, f, r)
                            s = 1 if side == stm else -1
                            i = KI[k]
                            xf[i * 7 + RB(or_)] += s
                            xf[21 * 7 + i * 6 + FB(of)] += s
                            if k == "q" and or_ <= 2: x5[0] += s
                            if k == "h" and or_ <= 2: x5[1] += s
                            if k == "e" and 3 <= of <= 5: x5[2] += s
                            if k == "g" and or_ >= 3: x5[3] += s
                            if k == "o" and of <= 2: x5[4] += s
                        rows.append((f"{run}:{p['pair_number']}", f"{run}:{p['pair_number']}:{gi}", n, sc["value"], z, x5, xf))
                if mv is None or n >= SNAP[-1]: break
                sq = [(int(a), RANKS.index(b)) for a, b in SQ.findall(mv)]
                frm, to = sq[0], sq[-1]
                pc = board.pop(frm)
                for m in sq[1:-1]:
                    if m != frm: board.pop(m, None)
                board.pop(to, None)
                if mv.endswith("+"): pc = (pc[0], pc[1], True)
                board[to] = pc
    print(run, len(rows), flush=True)
pickle.dump(rows, open("rows.pkl", "wb"))
