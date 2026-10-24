#!/usr/bin/env python3
"""Summarize correspondence searches (go movetime 300000) in the Lishogi-Bot log.

Usage: analyze.py <lishogi-bot.log> <since> <until>

`since` and `until` are UTC timestamps such as "2026-10-01 00:00:00"; searches
whose go falls in [since, until) are counted. Log timestamps are UTC.

Engine output lines carry no process id and two games may interleave.
An info line is attributed to an open correspondence search when its `time`
field matches the wall clock since that search's go (log lag -0.05..1.5 s) and
its depth does not decrease. A line that fits two open searches is attributed
to neither, and both searches are marked ambiguous.
A bestmove carries a stop reason only when the engine line right before it is
`info string stop <reason>`; otherwise its reason is unknown.
Each bestmove ends at most one search. A search first tries the bestmove with
reason soft within 0.3 s of its last attributed line that plays that line's move,
then the bestmove with reason hard 299.5..301.5 s after go.

After a hard stop, minase writes one more info line when the selected result is
deeper than the last progress line: a partial result (with `lowerbound`, after
`info string partial`) or a deeper helper worker's result. In a hard-stopped
search, attributed lines with time >= 299.5 s are that line, because a main
worker iteration that completes after 197 s is followed by a soft stop.
"""
import bisect
import re
import statistics
import sys
from collections import defaultdict
from datetime import datetime

CAP_S = 300.0
FINAL_LINE_S = 299.5

TS = re.compile(r"^(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d,\d{3}) ")
INFO = re.compile(r">> info depth (\d+) .*?time (\d+) pv (\S+)")
BEST = re.compile(r">> bestmove (\S+)")
VERSION = re.compile(r">> id name minase (\S+)")
STOP = re.compile(r">> info string stop (\w+)")


def ts(line):
    return datetime.strptime(TS.match(line).group(1), "%Y-%m-%d %H:%M:%S,%f").timestamp()


def parse(path, since, until):
    searches, bests, active = [], [], []
    version, previous_output = None, ""
    with open(path, errors="replace") as log:
        for line in log:
            if "engine_ctrl.usi DEBUG >> " not in line and "INFO go movetime 300000" not in line:
                continue
            if ">> id name minase" in line:
                version = VERSION.search(line).group(1)
            elif "INFO go movetime 300000" in line:
                if since <= line[:19] < until:
                    search = {"t0": ts(line), "version": version, "infos": [], "ambiguous": False}
                    searches.append(search)
                    active.append(search)
                continue
            elif ">> info depth" in line and active:
                m = INFO.search(line)
                if m:
                    t = ts(line)
                    depth, time_s, move = int(m.group(1)), int(m.group(2)) / 1000, m.group(3)
                    active = [s for s in active if t - s["t0"] <= CAP_S + 2]
                    fits = [s for s in active
                            if -0.05 <= t - s["t0"] - time_s <= 1.5
                            and (not s["infos"] or depth >= s["infos"][-1]["depth"])]
                    if len(fits) > 1:
                        for search in fits:
                            search["ambiguous"] = True
                    elif fits:
                        fits[0]["infos"].append({"time": time_s, "depth": depth, "move": move,
                                                 "t": t, "lower": " lowerbound " in line})
            elif ">> bestmove" in line:
                m = STOP.search(previous_output)
                bests.append({"t": ts(line), "move": BEST.search(line).group(1),
                              "stop": m.group(1) if m else None, "used": False})
            previous_output = line
    return searches, bests


def match_end(search, bests, best_times):
    last = search["infos"][-1]
    i = bisect.bisect_left(best_times, last["t"] - 0.01)
    while i < len(bests) and bests[i]["t"] <= last["t"] + 0.3:
        best = bests[i]
        if not best["used"] and best["stop"] == "soft" and best["move"] == last["move"]:
            return best
        i += 1
    i = bisect.bisect_left(best_times, search["t0"] + CAP_S - 0.5)
    while i < len(bests) and bests[i]["t"] <= search["t0"] + CAP_S + 1.5:
        best = bests[i]
        if not best["used"] and best["stop"] == "hard":
            return best
        i += 1
    return None


def classify(search):
    """Split a matched search into main worker iterations and the final line after a hard stop."""
    hard = search["end"]["stop"] == "hard"
    search["main"] = [x for x in search["infos"] if not (hard and x["time"] >= FINAL_LINE_S)]
    final = [x for x in search["infos"] if hard and x["time"] >= FINAL_LINE_S]
    search["partial"] = any(x["lower"] for x in final)
    search["helper"] = bool(final) and not search["partial"]


def best_at(infos, limit_s):
    best = None
    for info in infos:
        if info["time"] <= limit_s:
            best = info["move"]
    return best


def summarize(label, rows):
    if not rows:
        print(f"  {label}: none")
        return
    hard = [s for s in rows if s["end"]["stop"] == "hard"]
    soft = [s for s in rows if s["end"]["stop"] == "soft"]
    used = {id(s): CAP_S if s["end"]["stop"] == "hard" else s["end"]["t"] - s["t0"] for s in rows}
    total_used = sum(used.values())
    main_last = [s["main"][-1]["time"] for s in rows]
    partial = [s for s in rows if s["partial"]]
    partial_changed = sum(1 for s in partial if s["end"]["move"] != s["main"][-1]["move"])
    past_150 = [s for s in rows if used[id(s)] > 150]
    changed_150 = sum(1 for s in past_150 if best_at(s["main"], 150) != s["end"]["move"])
    print(f"  {label}: {len(rows)} searches, hard {len(hard)} ({len(hard) / len(rows):.0%}),"
          f" soft {len(soft)}")
    print(f"    used time: mean {total_used / len(rows):.0f} s")
    print(f"    main worker's last completed iteration: mean {statistics.mean(main_last):.0f} s,"
          f" median {statistics.median(main_last):.0f} s,"
          f" cap / mean {CAP_S / statistics.mean(main_last):.2f}")
    if soft:
        unused = [CAP_S - used[id(s)] for s in soft]
        print(f"    soft stops: mean unused {statistics.mean(unused):.0f} s,"
              f" per search {sum(unused) / len(rows):.0f} s")
    if hard:
        after_main = [CAP_S - s["main"][-1]["time"] for s in hard]
        helper = [s for s in hard if s["helper"]]
        main_played = [CAP_S - s["main"][-1]["time"] for s in hard if not s["helper"] and not s["partial"]]
        print(f"    hard stops: mean time after main worker's last completed iteration"
              f" {statistics.mean(after_main):.0f} s ({statistics.mean(after_main) / CAP_S:.0%} of cap);"
              f" share of total used time {sum(after_main) / total_used:.0%}")
        print(f"    hard stops: partial results {len(partial)}"
              f" (move differs from main worker's last completed iteration: {partial_changed}),"
              f" deeper helper results {len(helper)} (completion time unknown),"
              f" neither {len(main_played)}"
              + (f" (mean time after main worker's last completed iteration"
                 f" {statistics.mean(main_played):.0f} s)" if main_played else ""))
    print(f"    searches past 150 s: {len(past_150)}; main worker's best at 150 s differs from"
          f" played move: {changed_150} ({changed_150 / max(1, len(past_150)):.1%})")


def main():
    path, since, until = sys.argv[1], sys.argv[2], sys.argv[3]
    searches, bests = parse(path, since, until)
    best_times = [b["t"] for b in bests]
    groups = defaultdict(list)
    excluded = defaultdict(lambda: defaultdict(int))
    for search in searches:
        end = match_end(search, bests, best_times) if search["infos"] else None
        if end is None:
            reason = ("ambiguous lines" if search["ambiguous"]
                      else "no attributed line" if not search["infos"] else "no matching end")
            excluded[search["version"]][reason] += 1
            continue
        end["used"] = True
        search["end"] = end
        classify(search)
        if not search["main"]:
            excluded[search["version"]]["no main worker iteration"] += 1
            continue
        groups[search["version"]].append(search)
    print(f"searches with go in [{since}, {until}) UTC: {len(searches)}")
    for version in sorted(set(groups) | set(excluded), key=lambda v: tuple(int(x) for x in v.split("."))):
        rows = groups[version]
        clean = [s for s in rows if not s["ambiguous"]]
        print(f"\nminase {version}: excluded {dict(excluded[version])},"
              f" matched with ambiguous lines {len(rows) - len(clean)}")
        summarize("all matched", rows)
        summarize("without ambiguous lines", clean)


if __name__ == "__main__":
    main()
