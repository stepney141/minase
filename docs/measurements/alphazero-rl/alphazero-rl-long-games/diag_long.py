"""Games of the diagnostic run (3000-ply limit) that went past 1000 plies."""
import json, sys
from pathlib import Path
import numpy as np
from long_games import games_of, decode

cycle = Path(sys.argv[1])
summary = json.loads((cycle / "selfplay/summary.json").read_text())
lengths, longs = [], []
for name in summary["training_files"]:
    if name.endswith(".resigned.mnsd"):
        continue
    for game, plies, boards, sides, values, outcome in games_of(cycle / "selfplay" / name):
        L = int(plies[-1]); lengths.append(L)
        if L < 1000:
            continue
        p = values / 65535.0
        pb = np.where(sides == 0, p, 1.0 - p)
        cnt = [decode(b) for b in boards]
        i = int(np.searchsorted(plies, 1000))
        c = cnt[i]
        tot = np.array([x["b"] + x["w"] for x in cnt])
        after = tot[i:]
        caps = (after[0] - after[-1]) * 100.0 / max(1, L - 1000)
        keys = [bytes(b) + bytes([s]) for b, s in zip(boards[i:], sides[i:])]
        rep = 1 - len(set(keys)) / len(keys)
        longs.append(dict(game=game, length=L, outcome=outcome, b=c["b"], w=c["w"], bs=c["b_strong"], ws=c["w_strong"],
                          bl=c["b_lion"], wl=c["w_lion"], p1000=float(pb[i]), pend=float(pb[-1]), caps100=caps, rep=rep))
lengths = np.array(lengths)
print(f"games {len(lengths)}  mean {lengths.mean():.0f}  >1000: {np.sum(lengths >= 1000)} ({np.mean(lengths >= 1000):.1%})")
for r in sorted(longs, key=lambda r: r["length"]):
    res = {0: "black", 1: "white", 2: "draw"}[r["outcome"]]
    lead = "black" if r["p1000"] > 0.5 else "white"
    print(f"  len {r['length']:4d} result {res:5s} | at ply1000 pieces b{r['b']}/w{r['w']} strong b{r['bs']}/w{r['ws']} lions b{r['bl']}/w{r['wl']}"
          f" p_black {r['p1000']:.2f} ({lead}) | captures/100 after {r['caps100']:.1f} | repeated sampled positions {r['rep']:.2f} | p_black at end {r['pend']:.2f}")
L = np.array([r["length"] for r in longs])
dr = [r for r in longs if r["outcome"] == 2]
dec = [r for r in longs if r["outcome"] != 2]
fav = [r for r in dec if abs(r["p1000"] - 0.5) > 0.1]
print(f"AGG over {len(longs)} games >=1000 plies: length median {np.median(L):.0f} p90 {np.percentile(L,90):.0f} max {L.max()}")
print(f"AGG draws {len(dr)} ({len(dr)/len(longs):.0%}), decisive {len(dec)}")
print(f"AGG pieces at 1000 median {np.median([r['b']+r['w'] for r in longs]):.0f}, strong {np.median([r['bs']+r['ws'] for r in longs]):.0f}, games with any lion {np.mean([(r['bl']+r['wl'])>0 for r in longs]):.0%}")
print(f"AGG captures per 100 plies after 1000: median {np.median([r['caps100'] for r in longs]):.1f}")
print(f"AGG decisive games where side favored at 1000 (|p-0.5|>0.1) won: {np.mean([(r['p1000']>0.5)==(r['outcome']==0) for r in fav]):.0%} (n={len(fav)})")
print(f"AGG |p-0.5| at ply1000 median {np.median([abs(r['p1000']-0.5) for r in longs]):.2f}; among draws {np.median([abs(r['p1000']-0.5) for r in dr]):.2f}")
