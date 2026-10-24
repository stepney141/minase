"""初期局面の麒麟を自陣の各空升へ移した局面を、minaseのUSIのevalで評価する。"""
import subprocess, sys
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
START = INIT
