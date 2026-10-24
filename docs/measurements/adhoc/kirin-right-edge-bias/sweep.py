"""初期局面の麒麟を自陣の各空升へ移した局面を、minaseのUSIのevalで評価する。"""
import subprocess, sys
BIN = sys.argv[1]
INIT = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL"
R = "abcdefghijkl"
def parse(b):
    g = {}
    for r, row in enumerate(b.split("/")):
        f = 12; i = 0
        while i < len(row):
            if row[i].isdigit():
                j = i
                while j < len(row) and row[j].isdigit(): j += 1
                f -= int(row[i:j]); i = j; continue
            g[(f, R[r])] = row[i]; f -= 1; i += 1
    return g
def emit(g):
    rows = []
    for r in R:
        s = ""; e = 0
        for f in range(12, 0, -1):
            p = g.get((f, r))
            if p is None: e += 1; continue
            if e: s += str(e); e = 0
            s += p
        if e: s += str(e)
        rows.append(s)
    return "/".join(rows)
base = parse(INIT)
def variants(piece, frm, ranks, stm):
    out = []
    for r in ranks:
        for f in range(1, 13):
            sq = (f, r)
            if sq in base and sq != frm: continue
            g = dict(base); del g[frm]; g[sq] = piece
            out.append((sq, f"{emit(g)} {stm} - 1"))
    return out
def evaluate(sfens):
    cmds = ["usi", "isready", "usinewgame"]
    for _, s in sfens: cmds += [f"position sfen {s}", "eval"]
    cmds.append("quit")
    out = subprocess.run([BIN, "--protocol", "usi", "--rules", "engine-default"], input="\n".join(cmds) + "\n", capture_output=True, text=True).stdout
    return [int(l.split()[-1]) for l in out.splitlines() if l.startswith("info string evaluation")]
for name, piece, frm, ranks, stm, own in [("先手麒麟", "O", (7, "k"), "ghijkl", "b", lambda f: f), ("後手麒麟", "o", (6, "b"), "abcdef", "w", lambda f: 13 - f)]:
    vs = variants(piece, frm, ranks, stm)
    ev = evaluate(vs)
    ref = ev[[sq for sq, _ in vs].index(frm)]
    print(f"== {name}（手番側から見た評価値、初期位置との差。列は持ち主から見た筋で左端12→右端1）")
    rows = {}
    for (sq, _), e in zip(vs, ev): rows.setdefault(sq[1], {})[own(sq[0])] = e - ref
    for r in (ranks if stm == "b" else ranks[::-1]):
        print(f"  {r}: " + " ".join(f"{rows[r][of]:+5d}" if of in rows.get(r, {}) else "    ." for of in range(12, 0, -1)))
