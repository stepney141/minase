#!/usr/bin/env python3
"""Parse the Lishogi-Bot log into per-search records (JSON lines).

Each own-move search (go btime/wtime ... or go ponder + ponderhit) becomes one record.
Engine output lines carry no process id; with challenge concurrency 2 two engines may
interleave. A search is flagged `overlap` if another search was open at any time
between its go and its bestmove; such records are excluded from timing analyses.
"""
import json
import re
import sys
from datetime import datetime

LOG = sys.argv[1]
OUT = sys.argv[2]
SINCE = "2026-09-21"

line_re = re.compile(r"^(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d,\d{3}) (\S+) (\w+) (.*)$")


def ts(s):
    return datetime.strptime(s, "%Y-%m-%d %H:%M:%S,%f").timestamp()


games = {}  # id -> dict(clock, color, versions, start)
last_idname = None
open_searches = []
records = []
pending_position = None
pending_clock = None
pending_post = []  # own-move searches waiting for POST attribution
sid = 0


def kv(tokens, key):
    if key in tokens:
        i = tokens.index(key)
        if i + 1 < len(tokens):
            return tokens[i + 1]
    return None


with open(LOG, errors="replace") as f:
    for raw in f:
        m = line_re.match(raw.rstrip("\n"))
        if not m:
            continue
        stamp, logger, level, msg = m.groups()
        if stamp[:10] < SINCE:
            # still track version lines etc. but skip work
            if "id name" in msg:
                last_idname = msg.split("id name", 1)[1].strip()
            continue
        if logger == "engine_ctrl.usi" and level == "DEBUG":
            if msg.startswith(">> id name"):
                last_idname = msg[len(">> id name"):].strip()
                continue
            if msg.startswith("<< position"):
                toks = msg.split()
                mv = toks[toks.index("moves") + 1:] if "moves" in toks else []
                pending_position = mv
                continue
            if msg.startswith("<< go"):
                toks = msg.split()[1:]
                t = ts(stamp)
                sid += 1
                s = {
                    "sid": sid, "go_ts": t, "go_stamp": stamp, "ponder": "ponder" in toks,
                    "btime": kv(toks, "btime"), "wtime": kv(toks, "wtime"),
                    "byoyomi": kv(toks, "byoyomi"), "binc": kv(toks, "binc"),
                    "winc": kv(toks, "winc"), "movetime": kv(toks, "movetime"),
                    "moves": pending_position, "clock_pre": pending_clock, "infos": [], "hit_ts": None,
                    "stop_sent_ts": None, "reason": None, "bestmove": None,
                    "bm_ts": None, "overlap": len(open_searches) > 0, "game": None,
                }
                pending_clock = None
                for o in open_searches:
                    o["overlap"] = True
                open_searches.append(s)
                continue
            if msg.startswith("<< ponderhit"):
                cands = [o for o in open_searches if o["ponder"] and o["hit_ts"] is None and o["stop_sent_ts"] is None]
                if len(cands) >= 1:
                    cands[-1]["hit_ts"] = ts(stamp)
                    if len(cands) > 1:
                        for o in cands:
                            o["overlap"] = True
                continue
            if msg.startswith("<< stop"):
                cands = [o for o in open_searches if o["stop_sent_ts"] is None]
                if cands:
                    # prefer an unhit ponder search
                    pc = [o for o in cands if o["ponder"] and o["hit_ts"] is None]
                    (pc or cands)[-1]["stop_sent_ts"] = ts(stamp)
                continue
            if msg.startswith(">> info depth"):
                if len(open_searches) == 1:
                    toks = msg.split()[1:]
                    d = int(kv(toks, "depth"))
                    sc = None
                    if "score" in toks:
                        i = toks.index("score")
                        if toks[i + 1] == "cp":
                            sc = int(toks[i + 2])
                        else:
                            mv_ = toks[i + 2]
                            sc = 100000 if not mv_.startswith("-") else -100000
                    pv = toks[toks.index("pv") + 1:] if "pv" in toks else []
                    open_searches[0]["infos"].append(
                        (d, int(kv(toks, "time")), sc, pv[0] if pv else None, ts(stamp), int(kv(toks, "nodes"))))
                else:
                    for o in open_searches:
                        o["overlap"] = True
                continue
            if msg.startswith(">> info string stop"):
                r = msg.split()[-1]
                if len(open_searches) == 1:
                    open_searches[0]["reason"] = r
                else:
                    for o in open_searches:
                        o["overlap"] = True
                    # attribute to a search that can end now
                    cands = [o for o in open_searches if not (o["ponder"] and o["hit_ts"] is None and o["stop_sent_ts"] is None)]
                    if cands:
                        cands[0]["reason"] = r
                continue
            if msg.startswith(">> bestmove"):
                if not open_searches:
                    continue
                cands = [o for o in open_searches if not (o["ponder"] and o["hit_ts"] is None and o["stop_sent_ts"] is None)]
                s = cands[0] if cands else open_searches[0]
                if len(open_searches) > 1:
                    for o in open_searches:
                        o["overlap"] = True
                open_searches.remove(s)
                s["bestmove"] = msg.split()[2] if len(msg.split()) > 2 else None
                s["bm_ts"] = ts(stamp)
                own = (not s["ponder"]) or (s["hit_ts"] is not None)
                if own:
                    pending_post.append(s)
                records.append(s)
                continue
            if msg.startswith(">> usiok") or msg.startswith("<< usi"):
                continue
        elif logger == "engine_wrapper" and msg.startswith("Starting engine"):
            # a freshly started engine: any search still open from a crashed engine of the
            # same process cannot be identified; drop searches open for > 1 h
            now = ts(stamp)
            open_searches = [o for o in open_searches if now - o["go_ts"] < 3600]
        elif logger == "lishogi" and msg.startswith("POST https://lishogi.org/api/bot/game/") and "/move/" in msg:
            mm = re.search(r"/game/(\w+)/move/(\S+)", msg)
            if mm:
                gid, mv = mm.group(1), mm.group(2)
                t = ts(stamp)
                for s in list(pending_post):
                    if s["bestmove"] == mv and t - s["bm_ts"] < 5:
                        s["game"] = gid
                        pending_post.remove(s)
                        break
                pending_post = [s for s in pending_post if t - s["bm_ts"] < 30]
        elif logger == "__main__":
            if msg.startswith("Searching for btime") or (msg.startswith("Pondering ") and " for btime " in msg):
                mm = re.search(r"btime (\d+) wtime (\d+)", msg)
                if mm:
                    pending_clock = [int(mm.group(1)), int(mm.group(2))]
                continue
            if msg.startswith("{'id': '") and "'clock':" in msg and "gameFull" in msg:
                gid = re.match(r"\{'id': '(\w+)'", msg).group(1)
                cm = re.search(r"'clock': \{'initial': (\d+), 'increment': (\d+), 'byoyomi': (\d+), 'periods': (\d+)\}", msg)
                sm = re.search(r"'sente': \{'id': '([^']*)'", msg)
                g = games.setdefault(gid, {"versions": [], "first": stamp})
                if cm:
                    g["clock"] = [int(x) for x in cm.groups()]
                if sm:
                    g["color"] = "sente" if sm.group(1) == "minase-bot" else "gote"
                g["last"] = stamp
            elif msg.startswith("+++ Playing https://lishogi.org/"):
                mm = re.match(r"\+\+\+ Playing https://lishogi.org/(\w+)/(\w+)", msg)
                gid, color = mm.groups()
                g = games.setdefault(gid, {"versions": [], "first": stamp})
                g["color"] = color
                if last_idname and last_idname not in g["versions"]:
                    g["versions"].append(last_idname)
                g["last"] = stamp
            elif "Game over" in msg:
                mm = re.search(r"lishogi.org/(\w+)/", msg)
                if mm and mm.group(1) in games:
                    games[mm.group(1)]["over"] = stamp

with open(OUT, "w") as fo:
    for s in records:
        fo.write(json.dumps(s) + "\n")
with open(OUT + ".games.json", "w") as fo:
    json.dump(games, fo)
print(len(records), "searches;", sum(1 for s in records if s["overlap"]), "overlap;", len(games), "games")
