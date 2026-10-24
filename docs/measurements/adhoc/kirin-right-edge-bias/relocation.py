"""実戦の局面で麒麟を自陣後方の各空升へ置き直し、NNUEの評価値の平均の差を筋・段ごとに出す。

引数は、minaseのバイナリ、lishogiの棋譜（NDJSON）、Botの利用者名、標本にする手数（カンマ区切り）、および置き直す麒麟の側（meはBot側、oppは相手側）である。
候補の升は、麒麟と同じ色の升（麒麟は斜め1升と縦横2升跳びしか持たず、升の色が変わらない）に限る。
最後の3行は、筋ごとの評価値、USIのevalが出力する非線形部分、およびその差である線形経路の偏差の平均である。

各局面について、置き直した全升の評価値の平均を引いた偏差を取り、局面間で平均する。
筋と段は持ち主から見た値で、筋1が右端、段1が最下段。
"""
import collections, json, re, subprocess, sys
sys.path.insert(0, ".")
from sweep_lib import parse, emit, START
BIN, PATH, ME = sys.argv[1], sys.argv[2], sys.argv[3]
PLIES = [int(x) for x in sys.argv[4].split(",")]
WHO = sys.argv[5]
SQ = re.compile(r"(\d{1,2})([a-l])")
R = "abcdefghijkl"
samples = []  # (sfen_list, own coords)
for line in open(PATH):
    g = json.loads(line)
    if "initialSfen" in g or not g.get("moves"): continue
    me = "sente" if g["players"]["sente"].get("user", {}).get("id") == ME else "gote"
    if WHO == "opp": me = "gote" if me == "sente" else "sente"
    board = parse(START)  # square -> letter ('+o' for promoted)
    kirin = (7, "k") if me == "sente" else (6, "b")
    moves = g["moves"].split()
    for ply, mv in enumerate(moves, 1):
        sqs = [(int(a), b) for a, b in SQ.findall(mv)]
        p = board.pop(sqs[0])
        for s in sqs[1:]:
            if s != sqs[0] and s in board: board.pop(s)
        if mv.endswith("+"): p = "+" + p
        board[sqs[-1]] = p
        if sqs[0] == kirin: kirin = sqs[-1] if not mv.endswith("+") else None
        elif kirin == sqs[-1] or kirin in sqs[1:-1]: kirin = None
        turn_me = (ply % 2 == 0) == (me == "sente")
        if ply in PLIES and turn_me and kirin is not None:
            own_ranks = "ijkl" if me == "sente" else "abcd"
            cands = []
            for r in own_ranks:
                for f in range(1, 13):
                    sq = (f, r)
                    if sq in board and sq != kirin: continue
                    if (f + R.index(r)) % 2 != (kirin[0] + R.index(kirin[1])) % 2: continue
                    b2 = dict(board); p2 = b2.pop(kirin); b2[sq] = p2
                    of = f if me == "sente" else 13 - f
                    orank = 12 - R.index(r) if me == "sente" else R.index(r) + 1
                    cands.append(((of, orank), f"{emit(b2)} {'b' if me == 'sente' else 'w'} - 1"))
            samples.append(cands)
cmds = ["usi", "isready", "usinewgame"]
for cands in samples:
    for _, s in cands: cmds += [f"position sfen {s}", "eval"]
cmds.append("quit")
out = subprocess.run([BIN, "--protocol", "usi", "--rules", "engine-default"], input="\n".join(cmds) + "\n", capture_output=True, text=True).stdout
ev = [int(l.split()[-1]) for l in out.splitlines() if l.startswith("info string evaluation")]
nl = [int(l.split()[-1]) for l in out.splitlines() if l.startswith("info string nonlinear")]
assert len(nl) == len(ev)
assert len(ev) == sum(len(c) for c in samples), (len(ev), sum(len(c) for c in samples))
acc = collections.defaultdict(list); accn = collections.defaultdict(list); i = 0
for cands in samples:
    vals = ev[i:i + len(cands)]; nls = nl[i:i + len(cands)]; i += len(cands)
    m = sum(vals) / len(vals); mn = sum(nls) / len(nls)
    for (key, _), v, n in zip(cands, vals, nls): acc[key].append(v - m); accn[key].append(n - mn)
print(f"局面数 {len(samples)}（手数 {PLIES}）")
print("段＼筋 " + " ".join(f"{f:>5}" for f in range(12, 0, -1)) + "   （持ち主から見て左端12→右端1、値は平均偏差cp、括弧なしはn>=20）")
for orank in (4, 3, 2, 1):
    cells = []
    for f in range(12, 0, -1):
        v = acc.get((f, orank), [])
        cells.append(f"{sum(v) / len(v):+5.0f}" if len(v) >= 20 else "    .")
    print(f"  {orank}段 " + " ".join(cells))
col = {f: [x for (ff, _), v in acc.items() if ff == f and len(v) >= 20 for x in v] for f in range(1, 13)}
print("筋平均 " + " ".join(f"{sum(col[f]) / len(col[f]):+5.0f}" if col[f] else "    ." for f in range(12, 0, -1)) + "  (n>=20の升だけ)")
print("筋のn  " + " ".join(f"{len(col[f]):5d}" for f in range(12, 0, -1)))

coln = {f: [x for (ff, _), v in accn.items() if ff == f and len(v) >= 20 for x in v] for f in range(1, 13)}
print("非線形 " + " ".join(f"{sum(coln[f]) / len(coln[f]):+5.0f}" if coln[f] else "    ." for f in range(12, 0, -1)))
print("線形   " + " ".join(f"{(sum(col[f]) - sum(coln[f])) / len(col[f]):+5.0f}" if col[f] else "    ." for f in range(12, 0, -1)))
