"""lishogiコーパスの人間同士の対局で、成る前の麒麟が持ち主から見た各筋にいた手数の割合を、レーティング帯ごとに出す。"""
import collections, json, sys
sys.argv = [sys.argv[0], "", "", ""] + sys.argv[1:]
src = open("kirin.py").read().split("def main(")[0]
exec(src)
path = sys.argv[4]
bands = collections.defaultdict(lambda: [collections.Counter(), 0, 0])
for line in open(path):
    g = json.loads(line)
    if "initialSfen" in g or not g.get("moves") or g.get("variant") != "chushogi": continue
    ps = g["players"]
    if any(ps[s].get("user", {}).get("title") == "BOT" or "aiLevel" in ps[s] or "user" not in ps[s] for s in ("sente", "gote")): continue
    if len(g["moves"].split()) < 60: continue
    try:
        traj, nply = replay(g)
    except Exception:
        continue
    for side in ("sente", "gote"):
        rating = ps[side].get("rating", 0)
        band = "2000+" if rating >= 2000 else "1700-1999" if rating >= 1700 else "<1700"
        tr = traj[("kirin", side, (7, "k") if side == "sente" else (6, "b"))]
        b = bands[band]; b[1] += 1; hit = False
        for (ply, sq, prom), nxt in zip(tr, tr[1:] + [[nply, None, None]]):
            if sq is None or prom: break
            of = sq[0] if side == "sente" else 13 - sq[0]
            b[0][of] += nxt[0] - ply
            hit |= of <= 2
        b[2] += hit
for band in ("2000+", "1700-1999", "<1700"):
    c, n, hit = bands[band]; tot = sum(c.values())
    print(f"{band:10} {n:5}駒  右端2筋 {(c[1] + c[2]) / tot:5.1%}（到達 {hit / n:5.1%}）  初期筋7 {c[7] / tot:5.1%}  左端2筋 {(c[11] + c[12]) / tot:5.1%}")
