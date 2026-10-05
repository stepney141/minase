#!/usr/bin/env python3
"""Build a per-own-move table for byoyomi games from searches.jsonl.

Outputs moves.json (list of dicts) used by analyze.py.
"""
import json
import collections

G = json.load(open("searches.jsonl.games.json"))
S = [json.loads(l) for l in open("searches.jsonl")]
BYO_GAMES = {g for g, v in G.items() if v.get("clock") and v["clock"][2] > 0}

own = [s for s in S if (not s["ponder"] or s["hit_ts"]) and s["game"] in BYO_GAMES and s["movetime"] is None
       and not s["overlap"]]
# drop parity-inconsistent records and (game, ply) duplicates
own = [s for s in own if (len(s["moves"] or []) % 2 == 0) == (G[s["game"]]["color"] == "sente")]
_cnt = collections.Counter((s["game"], len(s["moves"] or [])) for s in own)
own = [s for s in own if _cnt[(s["game"], len(s["moves"] or []))] == 1]
# prefix consistency within a game
_bg = collections.defaultdict(list)
for s in own:
    _bg[s["game"]].append(s)
viol = 0
for _g, _L in _bg.items():
    _L.sort(key=lambda s: len(s["moves"] or []))
    for a, b in zip(_L, _L[1:]):
        if (b["moves"] or [])[:len(a["moves"] or [])] != (a["moves"] or []):
            viol += 1
print("prefix violations", viol)
by_game = collections.defaultdict(list)
for s in own:
    by_game[s["game"]].append(s)

moves = []
for gid, L in by_game.items():
    g = G[gid]
    color = g["color"]
    L.sort(key=lambda s: s["go_ts"])
    prev_final = None
    for i, s in enumerate(L):
        ply = len(s["moves"] or [])
        R = int((s["btime"] if color == "sente" else s["wtime"]) or 0)
        B = int(s["byoyomi"] or 0)
        inc = int((s["binc"] if color == "sente" else s["winc"]) or 0)
        pre = s["clock_pre"][0 if color == "sente" else 1] if s["clock_pre"] else None
        ph = s["hit_ts"] is not None
        base_ts = s["hit_ts"] if ph else s["go_ts"]
        think = (s["bm_ts"] - base_ts) * 1000.0
        stop_el = (s["bm_ts"] - s["go_ts"]) * 1000.0  # engine elapsed at stop, from go
        hit_off = (s["hit_ts"] - s["go_ts"]) * 1000.0 if ph else 0.0
        infos = s["infos"]
        # separate main-worker progress lines from a final adopted (auxiliary) line
        main = list(infos)
        aux = None
        if len(main) >= 2:
            last, prev = main[-1], main[-2]
            if last[0] > prev[0]:
                abort_at_hit = ph and s["reason"] == "soft" and stop_el - hit_off <= 50
                if s["reason"] == "soft" and not abort_at_hit:
                    is_aux = last[1] - prev[1] <= 10
                else:
                    is_aux = last[1] >= stop_el - 25 and last[1] - prev[1] > 3 or last[1] - prev[1] <= 3
                if is_aux:
                    aux = last
                    main = main[:-1]
        # contiguity check of main depths
        depths = [x[0] for x in main]
        contiguous = depths == list(range(1, len(depths) + 1))
        t_last = main[-1][1] if main else None
        gap = stop_el - t_last if t_last is not None else None
        censored = gap is not None and gap > max(50.0, 0.02 * t_last)
        final = infos[-1] if infos else None
        final_score = final[2] if final else None
        m = {
            "game": gid, "sid": s["sid"], "ply": ply, "color": color, "R": R, "B": B, "inc": inc,
            "pre": pre, "ponderhit": ph, "overlap": s["overlap"], "reason": s["reason"],
            "think": think, "stop_el": stop_el, "hit_off": hit_off,
            "main": [[x[0], x[1], x[2], x[3]] for x in main], "aux": aux and [aux[0], aux[1], aux[2], aux[3]],
            "contiguous": contiguous, "t_last": t_last, "gap": gap, "censored": censored,
            "depth_main": main[-1][0] if main else None,
            "depth_adopted": final[0] if final else None,
            "final_score": final_score, "prev_final_score": prev_final,
            "version": g["versions"][-1] if g["versions"] else None,
            "clock": g["clock"], "go_stamp": s["go_stamp"],
        }
        prev_final = final_score
        moves.append(m)

# phase: main (R>0, next own move also R>0), crossing (R>0, next own move R==0), byo (R==0)
by_game = collections.defaultdict(list)
for m in moves:
    by_game[m["game"]].append(m)
for gid, L in by_game.items():
    L.sort(key=lambda m: m["ply"])
    for i, m in enumerate(L):
        if m["R"] == 0:
            m["phase"] = "byo"
        else:
            nxt = L[i + 1] if i + 1 < len(L) else None
            nxt_zero = nxt is not None and nxt["ply"] == m["ply"] + 2 and nxt["R"] == 0
            m["phase"] = "cross" if (nxt_zero or m["think"] >= m["R"]) else "main"

json.dump(moves, open("moves.json", "w"))
c = collections.Counter((m["phase"], m["ponderhit"], m["overlap"]) for m in moves)
for k in sorted(c):
    print(k, c[k])
print("noncontiguous", sum(1 for m in moves if not m["contiguous"] and not m["overlap"]))
print("no infos", sum(1 for m in moves if not m["main"] and not m["overlap"]))
print("aux", sum(1 for m in moves if m["aux"] and not m["overlap"]))
