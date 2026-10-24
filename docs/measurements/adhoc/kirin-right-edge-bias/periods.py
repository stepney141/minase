"""版の配備期間ごと・先後ごとに、麒麟が持ち主から見た右端2筋にいた手数の割合を出す。"""
import json, sys, datetime as dt
d = json.load(open(sys.argv[1]))
JST = dt.timezone(dt.timedelta(hours=9))
cuts = [("master(〜10-04)", "2026-09-01 00:00"), ("2.0.0", "2026-10-04 17:36"), ("2.2.0", "2026-10-08 09:43"),
        ("2.3.0", "2026-10-09 00:53"), ("2.4.0", "2026-10-12 12:03"), ("2.5.0", "2026-10-23 20:50"), ("end", "2100-01-01 00:00")]
cuts = [(n, dt.datetime.strptime(t, "%Y-%m-%d %H:%M").replace(tzinfo=JST).timestamp() * 1000) for n, t in cuts]
def own(side, f): return f if side == "sente" else 13 - f
def edge_share(games, who, side_filter=None):
    edge = tot = ngames = reached = 0
    for g in games:
        for key, tr in g["traj"].items():
            k, side, *_ = key.split("|")
            if k != "kirin" or ((side == g["me"]) != (who == "minase")): continue
            if side_filter and side != side_filter: continue
            ngames += 1; hit = False
            for (ply, sq, prom), nxt in zip(tr, tr[1:] + [[g["nply"], None, None]]):
                if sq is None or prom: break
                w = nxt[0] - ply; tot += w
                if own(side, sq[0]) <= 2: edge += w; hit = True
            reached += hit
    return f"{edge / tot:.0%} ({reached}/{ngames}局で到達)" if tot else "-"
for (name, lo), (_, hi) in zip(cuts, cuts[1:]):
    gs = [g for g in d if lo <= g["created"] < hi and not g["initial"]]
    print(f"{name:16} {len(gs):3}局  minase {edge_share(gs, 'minase'):22} 相手 {edge_share(gs, 'opp')}")
gs = [g for g in d if not g["initial"]]
for s in ("sente", "gote"):
    o = "gote" if s == "sente" else "sente"
    print(f"minaseが{s}: minase {edge_share(gs, 'minase', s)}  相手 {edge_share(gs, 'opp', o)}")
