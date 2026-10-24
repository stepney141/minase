"""対局ハーネスの実行ディレクトリを再生し、候補側と基準側の麒麟が成る前に持ち主から見た右端2筋にいた手数の割合を出す。

開始局面のランダム手順は数えず、エンジンの着手の区間だけを集計する。
"""
import glob, json, os, sys
sys.argv = [sys.argv[0], "", "", ""] + sys.argv[1:]
exec(open("kirin.py").read().split("def main(")[0])
for run in sys.argv[4:]:
    stat = {"candidate": [0, 0, 0, 0], "baseline": [0, 0, 0, 0]}  # edge手数, 全手数, 到達局, 局
    for f in sorted(glob.glob(os.path.join(run, "pairs", "*.json"))):
        p = json.load(open(f))
        opening = p["opening"]["moves"]
        for game in p["games"]:
            moves = [t["response"]["usi"] for t in game["turns"] if t["response"].get("kind") == "move"]
            g = {"id": f, "moves": " ".join(opening + moves)}
            try:
                traj, nply = replay(g)
            except Exception as e:
                continue
            cand = "sente" if game["candidate_color"] == "black" else "gote"
            for side in ("sente", "gote"):
                who = "candidate" if side == cand else "baseline"
                s = stat[who]; s[3] += 1; hit = False
                tr = traj[("kirin", side, (7, "k") if side == "sente" else (6, "b"))]
                for (ply, sq, prom), nxt in zip(tr, tr[1:] + [[nply, None, None]]):
                    if sq is None or prom: break
                    lo = max(ply, len(opening)); hi = nxt[0]
                    if hi <= lo: continue
                    of = sq[0] if side == "sente" else 13 - sq[0]
                    s[1] += hi - lo
                    if of <= 2: s[0] += hi - lo; hit = True
                s[2] += hit
    m = json.load(open(os.path.join(run, "manifest.json")))
    name = lambda e: e["identity"].get("hash", "")[:8] or os.path.basename(e["identity"].get("program", ""))
    print(os.path.basename(run))
    for who in ("candidate", "baseline"):
        e, t, h, n = stat[who]
        print(f"  {who:9} {name(m[who]):9} 右端2筋 {e / max(t, 1):5.1%}  到達 {h}/{n}局")
