#!/usr/bin/env python3
"""Phase 0 of docs/plans/byoyomi-time-usage.md: m, r, p, kappa* and the candidate predictions.

Run in the directory that holds bot.log, searches.jsonl (parse.py), moves.json (moves.py) and
iterations.csv (analyze.py). Writes phase0.json and prints a summary.
"""
import bisect
import collections
import csv
import json
import math
import random
import re
import statistics as st
from datetime import datetime

PAWN_VALUE = 100  # Pst::pawn_value() of every deployed weight file (stored piece table, pawn = 100)
MATE = 100000  # parse.py encodes mate scores as +-100000
BANDS = [(0, 50), (50, 100), (100, 200), (200, 10 ** 6)]
BAND_NAMES = ["0-50", "50-100", "100-200", "200+"]
OUT = {}


def band(ply):
    for i, (lo, hi) in enumerate(BANDS):
        if lo <= ply < hi:
            return i


def q(xs, p):
    xs = sorted(xs)
    if not xs:
        return float("nan")
    k = (len(xs) - 1) * p
    lo = math.floor(k)
    hi = min(lo + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)


def ts(s):
    return datetime.strptime(s, "%Y-%m-%d %H:%M:%S,%f").timestamp()


# ---------------------------------------------------------------- 1. margin m
S = [json.loads(l) for l in open("searches.jsonl")]
G = json.load(open("searches.jsonl.games.json"))
BYO_GAMES = {g for g, v in G.items() if v.get("clock") and v["clock"][2] > 0}
post_re = re.compile(r"^(\S+ \S+) lishogi DEBUG POST https://lishogi.org/api/bot/game/(\w+)/move/(\S+)$")
posts = collections.defaultdict(list)  # (game, move) -> [ts]
with open("bot.log", errors="replace") as f:
    for line in f:
        if "/move/" in line and "lishogi DEBUG POST" in line:
            m_ = post_re.match(line.rstrip("\n"))
            if m_:
                posts[(m_.group(2), m_.group(3))].append(ts(m_.group(1)))

proc_byo, proc_all = [], []
unmatched = 0
for s in S:
    own = (not s["ponder"]) or s["hit_ts"]
    if not own or s["game"] not in BYO_GAMES or s["movetime"] is not None or s["bm_ts"] is None:
        continue
    cand = [t for t in posts.get((s["game"], s["bestmove"]), []) if 0 <= t - s["bm_ts"] < 5]
    if not cand:
        unmatched += 1
        continue
    d = (min(cand) - s["bm_ts"]) * 1000.0
    color = G[s["game"]].get("color")
    R = int((s["btime"] if color == "sente" else s["wtime"]) or 0)
    proc_all.append(d)
    if R == 0:
        proc_byo.append(d)
m_ms = math.ceil((max(proc_byo) + 500) / 100) * 100
OUT["margin"] = dict(
    n_byo=len(proc_byo), median=q(proc_byo, .5), p99=q(proc_byo, .99), max=max(proc_byo),
    n_all_phases=len(proc_all), max_all_phases=max(proc_all), p99_all=q(proc_all, .99),
    unmatched_own_searches=unmatched, m_ms=m_ms)
print("1. bot processing (POST - bestmove) ms, byo phase: n=%d median %.1f p99 %.1f max %.1f -> m=%d ms" % (
    len(proc_byo), q(proc_byo, .5), q(proc_byo, .99), max(proc_byo), m_ms))
print("   all phases: n=%d p99 %.1f max %.1f; unmatched own searches %d" % (
    len(proc_all), q(proc_all, .99), max(proc_all), unmatched))

# ---------------------------------------------------------------- 2-3. signal, r, p
M = json.load(open("moves.json"))
mainm = [m for m in M if m["R"] > 0 and not m["ponderhit"] and not m["overlap"] and m["main"]]


def is_mate(s):
    return s is None or abs(s) >= MATE


def signals(it):
    """Per completed iteration index: True/False, or None when the score comparison involves mate.
    Only depth >= 6 points carry a signal; others are None."""
    out = []
    for i, (d, t, s, b) in enumerate(it):
        if d < 6:
            out.append(None)
            continue
        ch = (i >= 1 and it[i][3] != it[i - 1][3]) or (i >= 2 and it[i - 1][3] != it[i - 2][3])
        if i < 2 or is_mate(s) or is_mate(it[i - 2][2]):
            out.append(None)  # excluded: mate score (depth >= 6 always has i >= 5)
            continue
        out.append(bool(ch or s <= it[i - 2][2] - PAWN_VALUE))
    return out


def best_move_only_signal(it, i):
    return (i >= 1 and it[i][3] != it[i - 1][3]) or (i >= 2 and it[i - 1][3] != it[i - 2][3])


grp = {True: [0, 0, 0], False: [0, 0, 0]}  # [points with completed next, changes, points without next]
mate_excluded = 0
p_true = collections.Counter()
p_n = collections.Counter()
p_mate_excl = 0
for m in mainm:
    it = m["main"]
    sg = signals(it)
    for i, s_ in enumerate(sg):
        if it[i][0] < 6:
            continue
        if s_ is None:
            mate_excluded += 1
            continue
        if i + 1 < len(it):
            grp[s_][0] += 1
            grp[s_][1] += it[i + 1][3] != it[i][3]
        else:
            grp[s_][2] += 1
    last = len(it) - 1
    if it[last][0] < 6:
        continue
    if sg[last] is None:
        p_mate_excl += 1
        continue
    p_n[band(m["ply"])] += 1
    p_true[band(m["ply"])] += sg[last]
for k in (True, False):
    print("   group %s: decision points %d, best move changed at next %d (%.4f), points without completed next %d (%.1f%%)" % (
        k, grp[k][0], grp[k][1], grp[k][1] / grp[k][0] if grp[k][0] else float("nan"), grp[k][2],
        100 * grp[k][2] / (grp[k][0] + grp[k][2])))
stop = None
if min(grp[True][0], grp[False][0]) < 200:
    stop = "group below 200"
if grp[False][1] == 0:
    stop = "false-group rate is 0"
r = (grp[True][1] / grp[True][0]) / (grp[False][1] / grp[False][0])
n_moves = sum(p_n.values())
p = sum(p_true.values()) / n_moves
vbar = (1 - p) + p * r
kappa = math.e * math.exp(-((1 - p) * (1 / vbar) * math.log(1 / vbar) + p * (r / vbar) * math.log(r / vbar)))
p_band = [p_true[i] / p_n[i] if p_n[i] else float("nan") for i in range(4)]
OUT["signal"] = dict(pawn_value=PAWN_VALUE, moves=len(mainm), mate_excluded_points=mate_excluded,
                     group_true=grp[True], group_false=grp[False], r=r, p=p, p_moves=n_moves,
                     p_mate_excluded_moves=p_mate_excl, p_band=p_band, p_band_n=[p_n[i] for i in range(4)],
                     vbar=vbar, kappa=kappa, stop=stop)
print("2-3. moves %d; mate-excluded points %d; r=%.4f p=%.4f (n=%d moves, %d excluded for mate) vbar=%.4f kappa*=%.4f stop=%s" % (
    len(mainm), mate_excluded, r, p, n_moves, p_mate_excl, vbar, kappa, stop))
print("   p by band:", ["%s %.3f (n=%d)" % (BAND_NAMES[i], p_band[i], p_n[i]) for i in range(4)])
if stop:
    json.dump(OUT, open("phase0.json", "w"), indent=1)
    raise SystemExit("STOP: " + stop)

# ---------------------------------------------------------------- KM ratio distribution
rows = list(csv.DictReader(open("iterations.csv")))
STRATA = [(500, 2000), (2000, 8000), (8000, 32000), (32000, 10 ** 12)]


def km(obs):
    obs = sorted(obs)
    n = len(obs)
    S_ = 1.0
    vals, surv = [], []
    i = 0
    while i < n:
        v = obs[i][0]
        d = c_ = 0
        while i < n and obs[i][0] == v:
            if obs[i][1]:
                d += 1
            else:
                c_ += 1
            i += 1
        at_risk = n - (i - d - c_)
        if d:
            S_ *= 1 - d / at_risk
            vals.append(v)
            surv.append(S_)
    return vals, surv


strata = []
for lo, hi in STRATA:
    obs = []
    for rw in rows:
        tp = int(rw["t_prev_ms"])
        if lo <= tp < hi:
            if rw["censored"] == "1":
                obs.append((int(rw["stop_ms"]) / tp, False))
            else:
                obs.append((int(rw["t_next_ms"]) / tp, True))
    v, s_ = km(obs)
    strata.append((v, s_, [-x for x in s_]))


def draw_ratio(t, cond, rng):
    idx = next((i for i, (lo, hi) in enumerate(STRATA) if lo <= t < hi), 0)
    vals, surv, neg = strata[idx]
    j = bisect.bisect_right(vals, cond)
    S0 = surv[j - 1] if j > 0 else 1.0
    if S0 <= 0:
        return float("inf")
    u = rng.random() * S0
    k = bisect.bisect_left(neg, -u, lo=j)
    return vals[k] if k < len(vals) else float("inf")


# ---------------------------------------------------------------- budgets
def mtg(ply):
    return max(88, (432 - ply) // 2 if ply < 432 else 0)


def opening(R, ply):
    return min(ply + 4, 40) if R > 0 else 40


def soft_raw(R, B, ply, inc=0):
    return R // mtg(ply) + inc * 76 // 100 + B * 8 * opening(R, ply) // 400


def current_budget(R, B, ply):
    sr = soft_raw(R, B, ply)
    hard = max(1, min(sr * 451 // 100, R * 27 // 100 + B * 8 // 10, max(1, R + B - 30)))
    return sr, min(sr, hard), hard  # base, soft (signal-independent), hard


def cand_base(name, R, B, b, ply):
    if name == "B0":
        return soft_raw(R, B, ply)
    if name == "B1":
        return max(R // mtg(ply), int(min(1.0, (ply + 4) / 40) * kappa * b))
    if name == "B2":
        return R // max(1, (200 - ply) // 2) if ply < 200 else R
    raise ValueError(name)


def budget(name, R, B, m, ply, signal):
    if name == "current":
        base, soft, hard = current_budget(R, B, ply)
        return base, soft, hard
    b = max(1, B - m)
    base = cand_base(name, R, B, b, ply)
    hard = max(1, min(base * 451 // 100, R * 27 // 100 + B * 8 // 10, R + B - m))
    soft = min(int(base * (r if signal else 1.0) / vbar), hard)
    return base, soft, hard


def should_start(t, soft, hard, stable):
    pred = t * 263
    return pred <= hard * 100 and (not stable or pred <= soft * 100) and (stable or t < soft)


# ---------------------------------------------------------------- stage 1 replay
CANDS = ["current", "B0", "B1", "B2"]
rng = random.Random(20261006)
stage1 = {c: [[] for _ in range(4)] for c in CANDS}  # (ratio, final_signal)
DRAWS = 20
for m in mainm:
    it = m["main"]
    sg = signals(it)
    sig = [bool(x) if x is not None else (best_move_only_signal(it, i) if it[i][0] >= 6 else False)
           for i, x in enumerate(sg)]
    stable = [i >= 3 and len({it[j][3] for j in range(i - 3, i + 1)}) == 1 for i in range(len(it))]
    R, B, ply = m["R"], m["B"], m["ply"]
    for c in CANDS:
        for _ in range(DRAWS):
            i = 0
            t = it[0][1]
            beyond = False
            cond = None
            while True:
                s_i = sig[min(i, len(it) - 1)]
                st_i = stable[min(i, len(it) - 1)]
                base, soft, hard = budget(c, R, B, m_ms, ply, s_i)
                if not should_start(t, soft, hard, st_i):
                    think = t
                    break
                if i + 1 < len(it):
                    T = it[i + 1][1]
                else:
                    if not beyond:
                        beyond = True
                        cond = (m["stop_el"] / t) if m["censored"] else 1.0
                    else:
                        cond = 1.0
                    T = t * draw_ratio(t, cond, rng)
                if T > hard:
                    think = hard
                    break
                t = T
                i += 1
            fin_sig = sig[min(i, len(it) - 1)]
            stage1[c][band(ply)].append((think / base, fin_sig))
s1 = {}
for c in CANDS:
    s1[c] = []
    for bi in range(4):
        xs = [x for x, _ in stage1[c][bi]]
        s1[c].append(dict(band=BAND_NAMES[bi], n=len(xs) // DRAWS, p10=q(xs, .1), median=q(xs, .5), p90=q(xs, .9),
                          mean=st.mean(xs) if xs else None,
                          median_easy=q([x for x, s_ in stage1[c][bi] if not s_], .5),
                          median_difficult=q([x for x, s_ in stage1[c][bi] if s_], .5)))
OUT["stage1"] = s1
print("5a. stage 1 think/base by ply band (n = moves):")
for c in CANDS:
    print("   %-7s " % c + "  ".join("%s n=%d med %.2f [p10 %.2f p90 %.2f] easy %.2f diff %.2f" % (
        d["band"], d["n"], d["median"], d["p10"], d["p90"], d["median_easy"], d["median_difficult"]) for d in s1[c]))

# ---------------------------------------------------------------- stage 2 clocks
CLOCKS = [("short 6000+300", 6000, 300, False), ("long 45000+300", 45000, 300, False),
          ("10min+30s", 600000, 30000, True), ("90min+60s", 5400000, 60000, True)]
BYO_THINK_CURRENT = 0.551  # observed mean byo-phase think / B of the current rule (research Q1)
N_GAMES = 1000
POOLS = {}
for c in CANDS:
    for bi in range(4):
        for sg_ in (True, False):
            POOLS[(c, bi, sg_)] = [x for x, s_ in stage1[c][bi] if s_ == sg_] or [x for x, _ in stage1[c][bi]]
stage2 = {}
for cname, init, B, lishogi in CLOCKS:
    m_c = m_ms if lishogi else 30
    b = max(1, B - m_c)
    for c in CANDS:
        rng2 = random.Random("%s|%s" % (cname, c))
        exh, rem432, never = [], [], 0
        R_at = {0: [], 100: [], 200: []}
        for g in range(N_GAMES):
            T = init
            ex = None
            for ply in range(0, 432, 2):
                R = max(0, T - 1900 - B) if lishogi else T
                if ply in R_at:
                    R_at[ply].append(R)
                if R > 0:
                    bi = band(ply)
                    sig = rng2.random() < p_band[bi]
                    base, soft, hard = budget(c, R, B, m_c, ply, sig)
                    pool = POOLS[(c, bi, sig)]
                    think = min(int(base * pool[rng2.randrange(len(pool))]), hard)
                else:
                    think = int(BYO_THINK_CURRENT * B) if c == "current" else b
                think = min(think, T + B)
                T = max(0, T - think)
                if T == 0 and ex is None:
                    ex = ply
            if ex is None:
                never += 1
            exh.append(ex if ex is not None else 10 ** 6)
            rem432.append(T)
        tab = {}
        for ply in (0, 100, 200):
            Rm = int(q(R_at[ply], .5))
            if Rm > 0:
                _, se, _ = budget(c, Rm, B, m_c, ply, False)
                _, sd, _ = budget(c, Rm, B, m_c, ply, True)
                tab[ply] = (Rm, se / b, sd / b)
            else:
                tab[ply] = (0, None, None)
        stage2[(cname, c)] = dict(median_exhaustion_ply=q(exh, .5), share_not_exhausted=never / N_GAMES,
                                  median_remaining_432_ms=q(rem432, .5), soft_over_b=tab, b_ms=b, m_ms=m_c)
OUT["stage2"] = {"%s|%s" % k: v for k, v in stage2.items()}
print("5b. stage 2:")
for (cname, c), v in stage2.items():
    def fmt(i):
        R_, e, d = v["soft_over_b"][i]
        return "byo" if e is None else "%.2f/%.2f" % (e, d)
    mex = v["median_exhaustion_ply"]
    print("   %-15s %-7s soft/b easy/diff ply0 %s ply100 %s ply200 %s | exhaust med %s | not exhausted %.1f%% | rem@432 med %.0f ms" % (
        cname, c, fmt(0), fmt(100), fmt(200), ">432" if mex >= 10 ** 6 else "%d" % mex,
        100 * v["share_not_exhausted"], v["median_remaining_432_ms"]))
json.dump(OUT, open("phase0.json", "w"), indent=1, default=str)
