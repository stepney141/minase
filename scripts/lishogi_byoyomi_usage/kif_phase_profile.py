#!/usr/bin/env python3
"""lishogiの公開棋譜（KIF）から、Botの手の思考時間を持ち時間期と秒読み期に分けて集計する。

使い方: kif_phase_profile.py <対局ID>.kif ...
棋譜は https://lishogi.org/game/export/<対局ID>?clocks=true から取得する。
初手（Botは固定1秒で指す）と、持ち時間をまたぐ手は除く。
"""
import re
import statistics
import sys

BOT = "minase-bot"


def profile(path):
    text = open(path, encoding="utf-8").read()
    clock = re.search(r"^持ち時間：(\d+)分\+(\d+)秒", text, re.M)
    main_s, byoyomi_s = int(clock.group(1)) * 60, int(clock.group(2))
    side = 0 if re.search(rf"^先手：.*{BOT}", text, re.M) else 1
    rows = re.findall(
        r"^\s*(\d+)手目.*\((\d+):(\d+)/(\d+):(\d+):(\d+)\)", text, re.M
    )
    main, byoyomi = [], []
    for ply, mm, ss, hh, cm, cs in rows:
        ply = int(ply)
        if (ply - 1) % 2 != side or ply <= 2:
            continue
        think = int(mm) * 60 + int(ss)
        total = int(hh) * 3600 + int(cm) * 60 + int(cs)
        if total < main_s:
            main.append(think)
        elif total - think >= main_s:
            byoyomi.append(think)
    return main_s, byoyomi_s, main, byoyomi


def summary(times, byoyomi_s):
    if not times:
        return "   -"
    mean = sum(times) / len(times) / byoyomi_s
    median = statistics.median(times) / byoyomi_s
    return f"{len(times):4d} {mean:5.2f} {median:5.2f}"


print("game       main/B   B | main: n mean/B med/B | byoyomi: n mean/B med/B")
for path in sys.argv[1:]:
    main_s, byoyomi_s, main, byoyomi = profile(path)
    game = path.rsplit("/", 1)[-1].removesuffix(".kif")
    print(
        f"{game:10} {main_s / byoyomi_s:6.0f} {byoyomi_s:3d} | "
        f"{summary(main, byoyomi_s)} | {summary(byoyomi, byoyomi_s)}"
    )
