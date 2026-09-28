import json, sys
sys.path.insert(0, '.')
from analyze import Engine, games
res = {(r['pair'], r['ply']): r for r in json.load(open('results.json'))}
e = Engine(threads=1, hash_mb=16)
def owner(pc): return 'white' if pc.startswith('^') else 'black'
def name(pc): return pc.replace('^', '').replace(' ', '')
out = []
for g in games():
    k = g['drop'][1][0]; r = res[(g['pair'], k)]; mv = g['moves']; me = g['color']
    seq = mv[:k] + [mv[k], mv[k+1]] + r['deep_p2']['pv'][:8]
    prev, side = e.board(seq[:k]); log = []
    for i in range(k, len(seq)):
        cur, nside = e.board(seq[:i+1])
        mover = side
        caps = [name(pc) for sq, pc in prev.items() if owner(pc) != mover and (sq not in cur or owner(cur[sq]) == mover)]
        who = 'M' if mover == me else 'H'
        log.append(f"{who}:{seq[i]}" + (f"x{'+'.join(caps)}" if caps else ""))
        prev, side = cur, nside
    royals = [f"{sq}{name(pc)}" for sq, pc in prev.items() if owner(pc) == me and name(pc).lstrip('+') in ('王', '玉', '太')]
    print(f"pair{g['pair']:3d} ply{k} {me}: " + " ".join(log[:2]) + " | deep PV: " + " ".join(log[2:]) + f" | royals {royals}")
e.quit()
