"""Trend of long games (>= 800 plies) of one cycle: pieces, value, captures by ply."""
import json, sys
from pathlib import Path
import numpy as np
from long_games import games_of, decode

cycle = Path(sys.argv[1])
summary = json.loads((cycle / "selfplay/summary.json").read_text())
marks = [100, 200, 300, 400, 500, 600, 700, 800]
pieces = {m: [] for m in marks}; adv = {m: [] for m in marks}
caps = {(400, 600): [], (600, 800): []}
ahead_wins = []; value_ahead_wins = []
for name in summary["training_files"]:
    if name.endswith(".resigned.mnsd"):
        continue
    for game, plies, boards, sides, values, outcome in games_of(cycle / "selfplay" / name):
        if plies[-1] < 800:
            continue
        p = values / 65535.0
        pb = np.where(sides == 0, p, 1.0 - p)
        cnt = [decode(b) for b in boards]
        tot = np.array([c["b"] + c["w"] for c in cnt])
        for m in marks:
            i = int(np.searchsorted(plies, m))
            if i < len(plies):
                pieces[m].append(tot[i]); adv[m].append(abs(pb[i] - 0.5))
        for lo, hi in caps:
            sel = (plies >= lo) & (plies < hi)
            t = tot[sel]
            if len(t) > 1:
                caps[(lo, hi)].append((t[0] - t[-1]) * 100.0 / (hi - lo))
        i = int(np.searchsorted(plies, 600))
        if outcome != 2 and i < len(plies):
            diff = cnt[i]["b"] - cnt[i]["w"]
            if diff != 0:
                ahead_wins.append((diff > 0) == (outcome == 0))
            if abs(pb[i] - 0.5) > 0.1:
                value_ahead_wins.append((pb[i] > 0.5) == (outcome == 0))
print("games >= 800 plies:", len(pieces[800]))
for m in marks:
    print(f"  ply {m}: pieces median {np.median(pieces[m]):.0f}  |p-0.5| median {np.median(adv[m]):.2f}  (n={len(pieces[m])})")
for k, v in caps.items():
    print(f"  pieces captured per 100 plies in {k}: median {np.median(v):.1f}")
print(f"  side ahead in pieces at ply 600 wins: {np.mean(ahead_wins):.1%} (n={len(ahead_wins)})")
print(f"  side favored by value (|p-0.5|>0.1) at ply 600 wins: {np.mean(value_ahead_wins):.1%} (n={len(value_ahead_wins)})")
