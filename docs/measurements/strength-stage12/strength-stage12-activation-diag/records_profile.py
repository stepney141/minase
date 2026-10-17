#!/usr/bin/env python3
"""C5の保存記録から時計を再構成し、候補側の手数帯ごとの着手前残り時間の中央値と思考時間の中央値を出力する。

使い方: records_profile.py <run_dir> <base_ms> <inc_ms>
手数は開始局面の着手を含む盤上の手数（その手を指す前の局面の手数）とする。
"""
import glob, json, statistics, sys
run_dir, base, inc = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
BANDS = [(0, 50), (50, 100), (100, 150), (150, 200), (200, 300), (300, 400), (400, 10**9)]
rem = {b: [] for b in BANDS}; think = []
for f in sorted(glob.glob(f"{run_dir}/pairs/*.json")):
    p = json.load(open(f))
    opening = len(p["opening"]["moves"])
    for g in p["games"]:
        clock = {"black": base, "white": base}
        for i, t in enumerate(g["turns"]):
            ply = opening + i
            side = t["side"]
            before = clock[side]
            ms = t["think_time_ns"] / 1e6
            clock[side] = before - ms + inc
            if side != g["candidate_color"] or t["response"]["kind"] != "move":
                continue
            think.append(ms)
            for b in BANDS:
                if b[0] <= ply < b[1]:
                    rem[b].append(before)
print("think median ms %.1f n %d" % (statistics.median(think), len(think)))
print("remaining medians", [round(statistics.median(v)) if v else None for v in rem.values()])
print("counts", [len(v) for v in rem.values()])
