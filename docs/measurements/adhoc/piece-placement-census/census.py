"""全駒種の配置分布を、持ち主視点の144升で集団ごとに数える。"""
import glob, json, os, re, sys
R = "../../../../data"
START = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL"
SQ = re.compile(r"(\d{1,2})([a-l])")
RANKS = "abcdefghijkl"
PHASES = [(20, 60), (60, 100), (100, 160)]
STEP = 4
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
            prom = ch == "+"
            if prom: i += 1; ch = row[i]
            board[(f, r)] = (ch.lower(), 0 if ch.isupper() else 1, prom)
            f -= 1; i += 1
    return board
def own(side, f, r):  # 持ち主視点: file 1=右端, rank 1=自陣最下段
    return (f, 12 - r) if side == 0 else (13 - f, r + 1)
counts = {}
def bump(group, phase, kind, f, r):
    d = counts.setdefault(group, {}).setdefault(phase, {})
    a = d.get(kind)
    if a is None: a = d[kind] = [0] * 144
    a[(f - 1) * 12 + (r - 1)] += 1
games_n = {}
def play(moves, groups):
    """groups: {side: [group...]}"""
    board = parse(START)
    n = 0
    for mv in moves:
        n += 1
        prom = mv.endswith("+")
        sqs = [(int(a), RANKS.index(b)) for a, b in SQ.findall(mv)]
        frm, to = sqs[0], sqs[-1]
        pc = board.pop(frm)
        for mid in sqs[1:-1]:
            if mid != frm: board.pop(mid, None)
        board.pop(to, None)
        if prom: pc = (pc[0], pc[1], True)
        board[to] = pc
        if n % STEP: continue
        ph = next((i for i, (lo, hi) in enumerate(PHASES) if lo <= n < hi), None)
        if ph is None:
            if n >= PHASES[-1][1]: break
            continue
        for (f, r), (k, side, p) in board.items():
            gs = groups.get(side)
            if not gs: continue
            of, orank = own(side, f, r)
            kind = ("+" if p else "") + k
            for g in gs: bump(g, ph, kind, of, orank)
def harness(run, cand, base, opening_skip=True):
    i = 0
    for f in sorted(glob.glob(os.path.join(R, "matches", run, "pairs", "*.json"))):
        p = json.load(open(f))
        op = p["opening"]["moves"]
        for g in p["games"]:
            mv = op + [t["response"]["usi"] for t in g["turns"] if t["response"].get("kind") == "move"]
            cs = 0 if g["candidate_color"] == "black" else 1
            gr = {}
            for side, name in ((cs, cand), (1 - cs, base)):
                if name:
                    gr[side] = [name, f"{name}#{'AB'[i % 2]}"]
                    games_n[name] = games_n.get(name, 0) + 1
            i += 1
            try: play(mv, gr)
            except Exception: pass
def humans():
    i = 0
    for line in open(os.path.join(R, "lishogi/games.ndjson")):
        g = json.loads(line)
        if "initialSfen" in g or not g.get("moves") or g.get("variant") != "chushogi": continue
        ps = g["players"]
        if any(ps[s].get("user", {}).get("title") == "BOT" or "aiLevel" in ps[s] or "user" not in ps[s] for s in ("sente", "gote")): continue
        mv = g["moves"].split()
        if len(mv) < 60: continue
        gr = {}
        for sn, side in (("sente", 0), ("gote", 1)):
            rt = ps[sn].get("rating", 0)
            name = "human2000" if rt >= 2000 else "human1700" if rt >= 1700 else "humanlow"
            gr[side] = [name, f"{name}#{'AB'[i % 2]}"]
            games_n[name] = games_n.get(name, 0) + 1
        i += 1
        try: play(mv, gr)
        except Exception: pass
humans()
harness("nnue-material-linear-init-noninferiority-ltc", None, "nnue_now")
for run in ("nnue-replacement-ltc",):
    if os.path.isdir(os.path.join(R, "matches", run)): harness(run, "nnue_first", "pst_fm")
if os.path.isdir(os.path.join(R, "matches", "pst-gen1-nodes100k")): harness("pst-gen1-nodes100k", "pst_gen1", "pst_gen0")
if os.path.isdir(os.path.join(R, "matches", "hachu-pst-gen1")): harness("hachu-pst-gen1", None, "hachu")
for run in ("alphazero-rl-c0032-ab-ladder-m3200-a1000", "alphazero-rl-c0032-ab-ladder-m800-a250", "alphazero-rl-c0032-ab-ladder-m800-a500"):
    harness(run, "c0032", "ab_vs_zero")
json.dump({"counts": counts, "games": games_n}, open(os.path.join(R, "piece-placement-census/counts.json"), "w"))
print(games_n)
