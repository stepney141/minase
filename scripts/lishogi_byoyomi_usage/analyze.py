#!/usr/bin/env python3
"""Q1-Q4 analyses on moves.json; writes iterations.csv and prints results."""
import bisect
import collections
import csv
import json
import math
import random
import statistics as st

M = json.load(open("moves.json"))

EPOCHS = [("A", "2026-09-25 07:46"), ("B", "2026-09-26 06:22"), ("C", "9999")]
PARAMS = {  # expected_plies, min_moves, inc_share, hard_soft, hard_rem, iter_ratio
    "A": (450, 100, 0.70, 4.00, 0.25, 2.50),
    "B": (435, 94, 0.73, 3.94, 0.25, 2.44),
    "C": (432, 88, 0.76, 4.51, 0.27, 2.63),
}


def epoch(m):
    for name, bound in EPOCHS:
        if m["go_stamp"] < bound:
            return name


for m in M:
    m["epoch"] = epoch(m)


def q(xs, p):
    xs = sorted(xs)
    if not xs:
        return float("nan")
    k = (len(xs) - 1) * p
    lo = math.floor(k)
    hi = min(lo + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)


def summ(xs, fmt="%.3f"):
    if not xs:
        return "n=0"
    return ("n=%d p10=" + fmt + " med=" + fmt + " p90=" + fmt + " mean=" + fmt) % (
        len(xs), q(xs, .1), q(xs, .5), q(xs, .9), st.mean(xs))


def P(*a):
    print(*a)


ok = [m for m in M if not m["overlap"]]
P("== data ==")
P("own moves in byoyomi games:", len(M), " non-overlap:", len(ok))
P("games with any non-overlap move:", len({m["game"] for m in ok}))
c = collections.Counter((m["phase"], m["ponderhit"]) for m in ok)
P("phase x ponderhit:", dict(c))
P("epoch counts (non-overlap):", collections.Counter(m["epoch"] for m in ok))
P("byoyomi values (non-overlap byo moves):", collections.Counter(m["B"] for m in ok if m["phase"] == "byo"))

# ---------------- Q1 ----------------
P("\n== Q1 byoyomi utilization (engine-view byo phase, non-ponderhit, non-overlap) ==")
byo = [m for m in ok if m["phase"] == "byo" and not m["ponderhit"]]
P("moves", len(byo))
P("think/B          ", summ([m["think"] / m["B"] for m in byo]))
P("t_last_main/B    ", summ([m["t_last"] / m["B"] for m in byo]))
P("discarded/B      ", summ([(m["think"] - m["t_last"]) / m["B"] for m in byo]))
P("unused (1-think/B)", summ([1 - m["think"] / m["B"] for m in byo]))
P("mean B-share: completed-work %.3f discarded %.3f unused %.3f" % (
    st.mean(m["t_last"] / m["B"] for m in byo),
    st.mean((m["think"] - m["t_last"]) / m["B"] for m in byo),
    st.mean(1 - m["think"] / m["B"] for m in byo)))
hard = [m for m in byo if m["censored"]]
soft = [m for m in byo if not m["censored"]]
P("stop: soft(next not started) %d (%.1f%%), aborted(hard) %d (%.1f%%)" % (
    len(soft), 100 * len(soft) / len(byo), len(hard), 100 * len(hard) / len(byo)))
P("  soft: think/B", summ([m["think"] / m["B"] for m in soft]))
P("  hard: think/B", summ([m["think"] / m["B"] for m in hard]), " t_last/B", summ([m["t_last"] / m["B"] for m in hard]))
for e in "ABC":
    x = [m for m in byo if m["epoch"] == e]
    if x:
        P("  epoch %s n=%d think/B med %.3f mean %.3f  t_last/B mean %.3f  hard-share %.1f%%" % (
            e, len(x), q([m["think"] / m["B"] for m in x], .5), st.mean(m["think"] / m["B"] for m in x),
            st.mean(m["t_last"] / m["B"] for m in x), 100 * sum(m["censored"] for m in x) / len(x)))
for B in sorted({m["B"] for m in byo}):
    x = [m for m in byo if m["B"] == B]
    P("  B=%5d n=%4d think/B med %.3f mean %.3f  t_last/B mean %.3f  hard %.0f%%  depth med %s" % (
        B, len(x), q([m["think"] / B for m in x], .5), st.mean(m["think"] / B for m in x),
        st.mean(m["t_last"] / B for m in x), 100 * sum(m["censored"] for m in x) / len(x),
        q([m["depth_main"] for m in x], .5)))
# true clock view
# true clock view: lishogi reports main remaining during main time and exactly B during byoyomi;
# the bot's pre-subtraction value is pre = lishogi_time - 1900 - elapsed, engine R = max(0, pre - B)
def true_byo(m):
    return m["pre"] is not None and abs(m["pre"] - (m["B"] - 1900)) <= 60
truemain = [m for m in byo if not true_byo(m)]
P("engine-byo moves with true main time left (lishogi main remaining = pre+1.9s < B+1.9s): %d (%.1f%%); think/B mean %.3f; (pre+1900)/B med %.3f" % (
    len(truemain), 100 * len(truemain) / len(byo), st.mean(m["think"] / m["B"] for m in truemain) if truemain else 0,
    q([(m["pre"] + 1900) / m["B"] for m in truemain], .5)))
truebyo = [m for m in byo if true_byo(m)]
P("true byo think/B", summ([m["think"] / m["B"] for m in truebyo]), " t_last/B", summ([m["t_last"] / m["B"] for m in truebyo]))
P("true byo: mean shares completed %.3f discarded %.3f unused-of-B %.3f; hard %.1f%%" % (
    st.mean(m["t_last"] / m["B"] for m in truebyo), st.mean((m["think"] - m["t_last"]) / m["B"] for m in truebyo),
    st.mean(1 - m["think"] / m["B"] for m in truebyo), 100 * st.mean(m["censored"] for m in truebyo)))
cm = [m for m in ok if m["phase"] in ("main", "cross") and not m["ponderhit"]]
P("engine-main moves: min (pre+1900-B)/1000 s (true main above B) = %.1f" % min((m["pre"] + 1900 - m["B"]) / 1000 for m in cm if m["pre"] is not None))
P("periods>1 games among byo moves:", sum(1 for m in byo if m["clock"][3] > 1))

# ponderhit section
P("\n== ponderhit moves in byo phase (non-overlap) ==")
ph = [m for m in ok if m["phase"] == "byo" and m["ponderhit"]]
P("n", len(ph), " share of byo own moves %.1f%%" % (100 * len(ph) / (len(ph) + len(byo))))
P("think-after-hit/B", summ([m["think"] / m["B"] for m in ph]))
P("share think-after-hit < 0.05B: %.1f%%" % (100 * sum(m["think"] < 0.05 * m["B"] for m in ph) / len(ph)))
P("hit_off/B (ponder time before hit)", summ([m["hit_off"] / m["B"] for m in ph]))
P("total (hit_off+think)/B", summ([(m["hit_off"] + m["think"]) / m["B"] for m in ph]))
P("hit_off > 0.49B share: %.1f%%" % (100 * sum(m["hit_off"] > 0.49 * m["B"] for m in ph) / len(ph)))
c = collections.Counter((m["reason"], m["censored"], (m["stop_el"] - m["hit_off"]) <= 50) for m in ph)
P("reason,censored,stop<=50ms after hit:", dict(c))
for band in [(0, 100), (100, 200), (200, 300), (300, 1000)]:
    a = [m["depth_adopted"] for m in ph if band[0] <= m["ply"] < band[1]]
    b = [m["depth_adopted"] for m in byo if band[0] <= m["ply"] < band[1]]
    if a and b:
        P("  ply %s adopted depth: ponderhit med %.1f (n=%d) vs non-ponderhit med %.1f (n=%d)" % (
            band, q(a, .5), len(a), q(b, .5), len(b)))

# ---------------- Q2 ----------------
P("\n== Q2 iteration ratios ==")
rows = []
for m in ok:
    mn = m["main"]
    for i in range(len(mn) - 1):
        d, t, *_ = mn[i]
        if d >= 6 and t >= 500:
            rows.append(dict(game=m["game"], ply=m["ply"], phase=m["phase"], epoch=m["epoch"], version=m["version"],
                             ponderhit=int(m["ponderhit"]), hit_off_ms=round(m["hit_off"]), depth=d, t_prev_ms=t,
                             t_next_ms=mn[i + 1][1], censored=0, stop_ms="", B_ms=m["B"], R_ms=m["R"]))
    if m["censored"] and mn:
        d, t, *_ = mn[-1]
        if d >= 6 and t >= 500:
            rows.append(dict(game=m["game"], ply=m["ply"], phase=m["phase"], epoch=m["epoch"], version=m["version"],
                             ponderhit=int(m["ponderhit"]), hit_off_ms=round(m["hit_off"]), depth=d, t_prev_ms=t,
                             t_next_ms="", censored=1, stop_ms=round(m["stop_el"]), B_ms=m["B"], R_ms=m["R"]))
with open("iterations.csv", "w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=list(rows[0].keys()))
    w.writeheader()
    w.writerows(rows)
P("rows", len(rows), "censored", sum(r["censored"] for r in rows))
for r in rows:
    r["x"] = (r["t_next_ms"] if not r["censored"] else r["stop_ms"]) / r["t_prev_ms"]


def km(obs):
    """obs: list of (value, event). Returns (sorted event values, survival after each)."""
    obs = sorted(obs)
    n = len(obs)
    S = 1.0
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
            S *= 1 - d / at_risk
            vals.append(v)
            surv.append(S)
    return vals, surv


def km_median(vals, surv, p=0.5):
    for v, s in zip(vals, surv):
        if s <= 1 - p:
            return v
    return float("inf")


def kmq(obs):
    v, s = km(obs)
    return "KM p10 %.2f p25 %.2f med %.2f p75 %.2f p90 %.2f (n=%d, cens=%d, S_end=%.3f)" % (
        km_median(v, s, .1), km_median(v, s, .25), km_median(v, s, .5), km_median(v, s, .75), km_median(v, s, .9),
        len(obs), sum(1 for o in obs if not o[1]), s[-1] if s else 1)


allobs = [(r["x"], not r["censored"]) for r in rows]
P("pooled:", kmq(allobs))
P("uncensored only: med %.2f p90 %.2f" % (q([r["x"] for r in rows if not r["censored"]], .5),
                                           q([r["x"] for r in rows if not r["censored"]], .9)))
for lo, hi in [(6, 10), (10, 13), (13, 16), (16, 99)]:
    P("  depth %2d-%2d" % (lo, hi - 1), kmq([(r["x"], not r["censored"]) for r in rows if lo <= r["depth"] < hi]))
for lo, hi in [(500, 2000), (2000, 8000), (8000, 32000), (32000, 10 ** 9)]:
    P("  t_prev %5d-%s" % (lo, hi), kmq([(r["x"], not r["censored"]) for r in rows if lo <= r["t_prev_ms"] < hi]))
for ph_ in ("main", "cross", "byo"):
    P("  phase %s" % ph_, kmq([(r["x"], not r["censored"]) for r in rows if r["phase"] == ph_]))
P("  non-ponderhit", kmq([(r["x"], not r["censored"]) for r in rows if not r["ponderhit"]]))
for e in "ABC":
    P("  epoch %s" % e, kmq([(r["x"], not r["censored"]) for r in rows if r["epoch"] == e]))
# successive ratio correlation
pairs = []
for m in ok:
    mn = [x for x in m["main"] if x[0] >= 6 and x[1] >= 500]
    rs = [mn[i + 1][1] / mn[i][1] for i in range(len(mn) - 1)]
    pairs += [(math.log(rs[i]), math.log(rs[i + 1])) for i in range(len(rs) - 1)]
if pairs:
    a, b = zip(*pairs)
    P("corr(log r_i, log r_i+1) = %.3f (n=%d)" % (st.correlation(a, b), len(pairs)))

# ---------------- simulation ----------------
P("\n== simulation (byo phase, non-ponderhit, non-overlap) ==")
STRATA = [(500, 2000), (2000, 8000), (8000, 32000), (32000, 10 ** 12)]


def build(obs):
    v, s = km(obs)
    return v, s


strata_km = []
for lo, hi in STRATA:
    sub = [(r["x"], not r["censored"]) for r in rows if lo <= r["t_prev_ms"] < hi]
    strata_km.append(build(sub))
pooled_km = build(allobs)
NEG = {id(s): [-x for x in s] for v, s in strata_km + [pooled_km]}


def sample_ratio(t, cond, rng, stratified=True):
    """Sample r from KM (optionally stratified by t) conditional on r > cond. inf if beyond support."""
    if stratified:
        idx = next((i for i, (lo, hi) in enumerate(STRATA) if lo <= t < hi), 0)
        if t < 500:
            idx = 0
        vals, surv = strata_km[idx]
    else:
        vals, surv = pooled_km
    # survival at cond
    j = bisect.bisect_right(vals, cond)
    S0 = surv[j - 1] if j > 0 else 1.0
    if S0 <= 0:
        return float("inf")
    u = rng.random() * S0  # target survival level in (0, S0)
    # find first value with surv <= u among vals[j:]
    neg = NEG[id(surv)]
    k = bisect.bisect_left(neg, -u, lo=j)
    return vals[k] if k < len(vals) else float("inf")  # residual mass beyond the largest event


def simulate(m, X, Y, rng, stratified=True):
    B = m["B"]
    comps = [x[1] for x in m["main"]]
    # walk observed completions
    for i, t in enumerate(comps):
        if i == len(comps) - 1:
            break
        if t >= X * B:
            return dict(end=t, completed_last=True, extra=-(len(comps) - 1 - i), aborted_waste=0.0)
    t = comps[-1]
    extra = 0
    cond = (m["stop_el"] / t) if m["censored"] else 1.0
    first = True
    while True:
        if t >= X * B:
            return dict(end=t, completed_last=True, extra=extra, aborted_waste=0.0)
        r = sample_ratio(t, cond if first else 1.0, rng, stratified)
        first = False
        T = t * r
        if T > Y * B:
            return dict(end=Y * B, completed_last=False, extra=extra, aborted_waste=Y * B - t)
        t = T
        extra += 1


def run(X, Y, sel, draws=100, stratified=True, own_epoch_x=False):
    rng = random.Random(12345)
    used, comp, extra, waste = [], 0, [], []
    gain1 = 0
    n = 0
    for m in sel:
        x = 0.8 / PARAMS[m["epoch"]][5] if own_epoch_x else X
        for _ in range(draws):
            r = simulate(m, x, Y, rng, stratified)
            used.append(r["end"] / m["B"])
            comp += r["completed_last"]
            extra.append(r["extra"])
            gain1 += r["extra"] >= 1
            waste.append(r["aborted_waste"] / m["B"])
            n += 1
    return dict(used_mean=st.mean(used), used_med=q(used, .5), used_p10=q(used, .1), used_p90=q(used, .9),
                complete_rate=comp / n, extra_mean=st.mean(extra), waste_mean=st.mean(waste), gain1=gain1 / n)


sim = [m for m in byo if m["main"]]
obs_used = [m["think"] / m["B"] for m in sim]
P("observed: used mean %.3f med %.3f, not-aborted rate %.3f, discarded mean %.3f" % (
    st.mean(obs_used), q(obs_used, .5), 1 - st.mean(m["censored"] for m in sim),
    st.mean((m["think"] - m["t_last"]) / m["B"] for m in sim)))
for strat in (True,):
    r = run(None, 0.8, sim, draws=20, stratified=strat, own_epoch_x=True)
    P("calibration (own-epoch X, Y=0.8, stratified=%s): used mean %.3f med %.3f, not-aborted %.3f, discarded %.3f, extra %.3f" % (
        strat, r["used_mean"], r["used_med"], r["complete_rate"], r["waste_mean"], r["extra_mean"]))
# non-tautological check: at the start point of the final started iteration, draw r without
# using the move's own outcome, and compare predicted abort rate with observed
rng = random.Random(7)
pred_abort, pred_used = [], []
for m in sim:
    B = m["B"]
    X0 = 0.8 / PARAMS[m["epoch"]][5]
    comps = [x[1] for x in m["main"]]
    # start point of the final started iteration: last completion < X0*B
    starts = [t for t in comps if t < X0 * B]
    if not starts:
        continue
    t = starts[-1]
    if not m["censored"] and comps[-1] < X0 * B:
        continue  # ended for another reason (depth limit, etc.)
    ab = 0
    us = 0.0
    for _ in range(100):
        tt = t
        while True:
            T = tt * sample_ratio(tt, 1.0, rng)
            if T > 0.8 * B:
                ab += 1
                us += 0.8
                break
            tt = T
            if tt >= X0 * B:
                us += tt / B
                break
    pred_abort.append(ab / 100)
    pred_used.append(us / 100)
P("holdout check (n=%d): predicted abort rate %.3f vs observed %.3f; predicted used %.3f vs observed %.3f" % (
    len(pred_abort), st.mean(pred_abort), st.mean(m["censored"] for m in sim), st.mean(pred_used), st.mean(obs_used)))
results = {}
P("X    Y     used_mean used_med used_p10 used_p90 last_started_completes discarded_mean extra_iters_vs_observed P(>=1 extra)")
for X in (0.3, 0.5, 0.7):
    for Y in (0.8, 0.9, 0.95):
        r = run(X, Y, sim, draws=100)
        results["%s,%s" % (X, Y)] = r
        P("%.2f %.2f  %.3f     %.3f    %.3f    %.3f    %.3f                  %.3f          %+.3f" % (
            X, Y, r["used_mean"], r["used_med"], r["used_p10"], r["used_p90"], r["complete_rate"], r["waste_mean"],
            r["extra_mean"]) + "   %.3f" % r["gain1"])
json.dump(results, open("sim_results.json", "w"), indent=1)
# simulation using C epoch only
simC = [m for m in sim if m["epoch"] == "C"]
r = run(0.304, 0.8, simC, draws=100)
P("epoch C only calibration X=0.304,Y=0.8: used %.3f, not-aborted %.3f vs observed used %.3f not-aborted %.3f (n=%d)" % (
    r["used_mean"], r["complete_rate"], st.mean(m["think"] / m["B"] for m in simC),
    1 - st.mean(m["censored"] for m in simC), len(simC)))

# ---------------- Q3 ----------------
P("\n== Q3 difficulty ==")
MATE = 100000
prev_overlap = {}
by_game = collections.defaultdict(list)
for m in M:
    by_game[m["game"]].append(m)
for g, L in by_game.items():
    L.sort(key=lambda m: m["ply"])
    for i, m in enumerate(L):
        m["prev_ok"] = i > 0 and not L[i - 1]["overlap"]


def diff_signals(m):
    it = [x for x in m["main"] if x[0] >= 6]
    bests = [x[3] for x in it]
    ch = [bests[i] != bests[i - 1] for i in range(1, len(bests))]
    n_changes = sum(ch)
    last_changed = bool(ch and ch[-1])
    changed_last2 = any(ch[-2:]) if ch else False
    cp = lambda s: s is not None and abs(s) < MATE
    drops = [it[i - 1][2] - it[i][2] for i in range(1, len(it)) if cp(it[i][2]) and cp(it[i - 1][2])]
    drop_any = max(drops) if drops else 0
    # same-parity drop over the last two iterations (removes odd-even oscillation)
    drop2 = (it[-3][2] - it[-1][2]) if len(it) >= 3 and cp(it[-3][2]) and cp(it[-1][2]) else 0
    fs = it[-1][2] if it else None
    ps = m["prev_final_score"] if m["prev_ok"] else None
    drop_prev = (ps - fs) if (cp(fs) and cp(ps)) else None
    return n_changes, last_changed, changed_last2, drop_any, drop2, drop_prev, len(it)


for m in ok:
    n_ch, lc, cl2, da, d2, dp, nit = diff_signals(m)
    m.update(n_changes=n_ch, last_changed=lc, changed_last2=cl2, drop_iter=da, drop2=d2, drop_prev=dp, n_it6=nit)
    m["difficult"] = cl2 or d2 >= 100 or (dp is not None and dp >= 100)

P("flag: changed_last2 OR same-parity drop over last 2 iterations >= 100cp OR drop vs previous own final >= 100cp")
P("|final score| by phase:", {ph_: round(q([abs(m["final_score"]) for m in ok if m["phase"] == ph_ and m["final_score"] is not None and abs(m["final_score"]) < MATE], .5)) for ph_ in ("main", "byo")})
bands = [(0, 50), (50, 100), (100, 200), (200, 300), (300, 10000)]


def rate(xs, key):
    return 100 * sum(1 for m in xs if key(m)) / len(xs) if xs else float("nan")


P("component rates by phase (non-overlap, incl. ponderhit):")
for phs in ("main", "cross", "byo"):
    x = [m for m in ok if m["phase"] == phs]
    P("  %-5s n=%4d iters>=6 med %.0f  n_changes mean %.2f  last_changed %.1f%%  changed_last2 %.1f%%  any-consec-drop>=50 %.1f%%  drop2>=50 %.1f%%  drop2>=100 %.1f%%  drop_prev>=50 %.1f%%  drop_prev>=100 %.1f%%  DIFFICULT %.1f%%" % (
        phs, len(x), q([m["n_it6"] for m in x], .5), st.mean(m["n_changes"] for m in x), rate(x, lambda m: m["last_changed"]),
        rate(x, lambda m: m["changed_last2"]), rate(x, lambda m: m["drop_iter"] >= 50), rate(x, lambda m: m["drop2"] >= 50),
        rate(x, lambda m: m["drop2"] >= 100),
        rate(x, lambda m: m["drop_prev"] is not None and m["drop_prev"] >= 50),
        rate(x, lambda m: m["drop_prev"] is not None and m["drop_prev"] >= 100), rate(x, lambda m: m["difficult"])))
P("component rates by ply band (all phases):")
for lo, hi in [(0, 50), (50, 100), (100, 200), (200, 300), (300, 10000)]:
    x = [m for m in ok if lo <= m["ply"] < hi]
    P("  ply %d-%d n=%d changed_last2 %.1f%% drop2>=100 %.1f%% drop_prev>=100 %.1f%% |score| med %.0f" % (
        lo, hi, len(x), rate(x, lambda m: m["changed_last2"]), rate(x, lambda m: m["drop2"] >= 100),
        rate(x, lambda m: m["drop_prev"] is not None and m["drop_prev"] >= 100),
        q([abs(m["final_score"]) for m in x if m["final_score"] is not None and abs(m["final_score"]) < MATE], .5)))
P("difficult rate by ply band x phase:")
for lo, hi in bands:
    line = "  ply %3d-%-4s" % (lo, hi if hi < 10000 else "")
    for phs in ("main", "cross", "byo", None):
        x = [m for m in ok if lo <= m["ply"] < hi and (phs is None or m["phase"] == phs)]
        line += "  %s: %5.1f%% (n=%d)" % (phs or "all", rate(x, lambda m: m["difficult"]), len(x))
    P(line)
P("difficult moves, non-ponderhit: think/B and main depth by phase (and by ply band):")
for phs in ("main", "byo"):
    x = [m for m in ok if m["phase"] == phs and not m["ponderhit"] and m["difficult"]]
    y = [m for m in ok if m["phase"] == phs and not m["ponderhit"] and not m["difficult"]]
    P("  %s difficult n=%d think/B med %.2f mean %.2f depth med %.1f | easy n=%d think/B med %.2f depth med %.1f" % (
        phs, len(x), q([m["think"] / m["B"] for m in x], .5), st.mean(m["think"] / m["B"] for m in x),
        q([m["depth_main"] for m in x], .5), len(y), q([m["think"] / m["B"] for m in y], .5),
        q([m["depth_main"] for m in y], .5)))
    for lo, hi in bands:
        xx = [m for m in x if lo <= m["ply"] < hi]
        if len(xx) >= 10:
            P("     ply %d-%d n=%d think/B med %.2f depth med %.1f think_s med %.1f" % (
                lo, hi, len(xx), q([m["think"] / m["B"] for m in xx], .5), q([m["depth_main"] for m in xx], .5),
                q([m["think"] / 1000 for m in xx], .5)))
# within-game depth relative to game's median byo depth
med_byo_depth = {}
for g, L in by_game.items():
    d = [m["depth_adopted"] for m in L if not m["overlap"] and m["phase"] == "byo" and not m["ponderhit"]]
    if len(d) >= 10:
        med_byo_depth[g] = q(d, .5)
for phs in ("main", "byo"):
    for dif in (True, False):
        x = [m["depth_adopted"] - med_byo_depth[m["game"]] for m in ok if m["game"] in med_byo_depth
             and m["phase"] == phs and not m["ponderhit"] and m["difficult"] == dif]
        P("  depth - game median byo depth: %s difficult=%s n=%d med %+.1f mean %+.2f" % (phs, dif, len(x), q(x, .5), st.mean(x) if x else 0))
alld = [m for m in ok if m["difficult"]]
P("share of difficult moves in byo phase: %.1f%% (byo share of all moves %.1f%%)" % (
    rate(alld, lambda m: m["phase"] == "byo"), rate(ok, lambda m: m["phase"] == "byo")))
P("difficult moves in byo phase: think/B and hard-stop share:",
  summ([m["think"] / m["B"] for m in ok if m["difficult"] and m["phase"] == "byo" and not m["ponderhit"]]),
  "hard %.1f%%" % rate([m for m in ok if m["difficult"] and m["phase"] == "byo" and not m["ponderhit"]], lambda m: m["censored"]))
P("easy moves in byo phase hard share %.1f%%" % rate([m for m in ok if not m["difficult"] and m["phase"] == "byo" and not m["ponderhit"]], lambda m: m["censored"]))
# per-game: count of difficult byo moves
pg = []
for g, L in by_game.items():
    x = [m for m in L if not m["overlap"]]
    if len(x) >= 20:
        pg.append((sum(1 for m in x if m.get("difficult") and m["phase"] == "byo"), sum(1 for m in x if m.get("difficult"))))
P("per game (>=20 non-overlap moves): difficult-in-byo / difficult-total, median share %.2f over %d games" % (
    q([a / b for a, b in pg if b], .5), len(pg)))
# |score| filter sensitivity
x = [m for m in ok if m["final_score"] is not None and abs(m["final_score"]) < 1000]
P("restricted to |final score|<1000: difficult main %.1f%% byo %.1f%%" % (
    rate([m for m in x if m["phase"] == "main"], lambda m: m["difficult"]),
    rate([m for m in x if m["phase"] == "byo"], lambda m: m["difficult"])))


# contested positions only: |final score| < 500 and |previous own final| < 500 (when known)
con = [m for m in ok if m["final_score"] is not None and abs(m["final_score"]) < 500
       and (m["prev_final_score"] is None or abs(m["prev_final_score"]) < 500)]
P("contested subset (|score|<500): n=%d, main %d byo %d" % (len(con), sum(m["phase"] == "main" for m in con), sum(m["phase"] == "byo" for m in con)))
for lo, hi in bands:
    line = "  ply %3d-%-4s" % (lo, hi if hi < 10000 else "")
    for phs in ("main", "byo"):
        x = [m for m in con if lo <= m["ply"] < hi and m["phase"] == phs]
        line += "  %s: %5.1f%% (n=%d)" % (phs, rate(x, lambda m: m["difficult"]), len(x))
    P(line)
for phs in ("main", "byo"):
    x = [m for m in con if m["phase"] == phs and not m["ponderhit"] and m["difficult"]]
    if x:
        P("  contested %s difficult n=%d think/B med %.2f think_s med %.1f depth med %.1f" % (
            phs, len(x), q([m["think"] / m["B"] for m in x], .5), q([m["think"] / 1000 for m in x], .5), q([m["depth_main"] for m in x], .5)))
    for lo, hi in [(50, 100), (100, 200), (200, 300)]:
        xx = [m for m in x if lo <= m["ply"] < hi]
        if len(xx) >= 10:
            P("     ply %d-%d n=%d think_s med %.1f depth med %.1f" % (lo, hi, len(xx), q([m["think"] / 1000 for m in xx], .5), q([m["depth_main"] for m in xx], .5)))

# ---------------- Q4 ----------------
P("\n== Q4 main phase think vs soft ==")


def budget(m):
    EP, MM, IS, HS, HR, IR = PARAMS[m["epoch"]]
    R, B, inc, ply = m["R"], m["B"], m["inc"], m["ply"]
    mtg = max(MM, (EP - ply) // 2 if EP > ply else 0)
    w = min(1.0, (ply + 4) / 40) if R > 0 else 1.0
    soft_raw = R / mtg + IS * inc + 0.8 * B * w
    hard = max(1, min(HS * soft_raw, HR * R + 0.8 * B, R + B - 30))
    return min(soft_raw, hard), hard


mainm = [m for m in ok if m["phase"] in ("main", "cross") and not m["ponderhit"]]
for m in mainm:
    m["soft"], m["hard"] = budget(m)
r_ = [m["think"] / m["soft"] for m in mainm]
P("n", len(mainm), "think/soft", summ(r_))
hist = collections.Counter(min(int(x * 10), 30) for x in r_)
P("histogram think/soft (bins of 0.1, last=>=3.0):", [(k / 10, hist[k]) for k in sorted(hist)])
P("think/hard", summ([m["think"] / m["hard"] for m in mainm]))
P("hard stops in main phase %d (%.1f%%)" % (sum(m["censored"] for m in mainm), rate(mainm, lambda m: m["censored"])))
# stable vs unstable modes: stable = last 4 completed same best
for m in mainm:
    b = [x[3] for x in m["main"]]
    m["stable"] = len(b) >= 4 and len(set(b[-4:])) == 1
for s_ in (True, False):
    x = [m["think"] / m["soft"] for m in mainm if m["stable"] == s_]
    P("  stable=%s n=%d think/soft %s" % (s_, len(x), summ(x)))
P("soft/B in main phase", summ([m["soft"] / m["B"] for m in mainm]))
P("think seconds main phase", summ([m["think"] / 1000 for m in mainm], "%.1f"))
# per game main-time accounting
P("per-game main time on moves with adopted depth <= game's median byo depth:")
tot_all = tot_cov = tot_shallow = 0
pgs = []
for g, L in by_game.items():
    if g not in med_byo_depth:
        continue
    mm = [m for m in L if m["phase"] in ("main", "cross")]
    if not mm:
        continue
    reached = any(m["R"] == 0 for m in L)
    if not reached:
        continue
    all_t = L[0]["clock"][0]  # initial main time (games that reached the byo phase)
    cov = [m for m in mm if not m["overlap"]]
    cov_t = sum(m["think"] for m in cov)
    sh_t = sum(m["think"] for m in cov if m["depth_adopted"] <= med_byo_depth[g])
    tot_all += all_t
    tot_cov += cov_t
    tot_shallow += sh_t
    if cov_t > 0:
        pgs.append((g, sh_t / cov_t, cov_t / all_t, med_byo_depth[g], len(mm), L[0]["clock"][0] // 60000, L[0]["B"]))
P("  games %d (reached byo phase); analysable main-phase think covers %.1f%% of initial main time; shallow share overall %.1f%%" % (len(pgs), 100 * tot_cov / tot_all, 100 * tot_shallow / tot_cov))
P("  per-game shallow share: %s" % summ([p[1] for p in pgs]))
for p in sorted(pgs, key=lambda p: -p[1])[:60]:
    P("   %s shallow %.2f cov %.2f medByoDepth %.1f mainMoves %d init %dmin B %ds" % (p[0], p[1], p[2], p[3], p[4], p[5], p[6] // 1000))
# ply at which engine R reached 0
P("ply of first byo-phase own move per game (initial minutes, B):")
for g, L in sorted(by_game.items(), key=lambda kv: kv[1][0]["clock"][0]):
    first = next((m["ply"] for m in L if m["R"] == 0), None)
    P("   %s init %3d min B %3ds first_byo_ply %s total_own %d" % (g, L[0]["clock"][0] // 60000, L[0]["B"] // 1000 if L[0]["B"] else 0, first, len(L)))
