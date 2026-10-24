"""traj.jsonから、minase側と相手側の麒麟・鳳凰について自陣から見た筋の分布を比べる。

own_file は持ち主から見た筋で、1が持ち主の右端、12が左端（初期位置は麒麟7、鳳凰6）。
"""
import collections
import json
import sys

d = json.load(open(sys.argv[1]))
since = int(sys.argv[2]) if len(sys.argv) > 2 else 0


def own_file(side, f):
    return f if side == "sente" else 13 - f


def summarize(kind, who):
    weight = collections.Counter()  # own_file -> 手数（その駒が成る前に盤上にいた手数）
    reach = collections.Counter()  # 成る前に到達した最も右の筋
    final = collections.Counter()
    n = 0
    edge_games = []
    for g in d:
        if g["created"] < since or g["initial"]:
            continue
        for key, tr in g["traj"].items():
            k, side, *_ = key.split("|")
            if k != kind:
                continue
            mine = side == g["me"]
            if (who == "minase") != mine:
                continue
            n += 1
            # 成る前の区間だけを見る
            segs = []
            for (ply, sq, prom), nxt in zip(tr, tr[1:] + [[g["nply"], None, None]]):
                if sq is None or prom:
                    break
                segs.append((own_file(side, sq[0]), nxt[0] - ply))
            for of, w in segs:
                weight[of] += w
            if segs:
                reach[min(of for of, _ in segs)] += 1
                final[segs[-1][0]] += 1
                if segs[-1][0] <= 2:
                    edge_games.append(g["id"])
    tot = sum(weight.values())
    print(f"== {kind} / {who}: {n}駒")
    print("  滞在手数の割合(own_file):", " ".join(f"{f}:{weight[f] / tot:.1%}" for f in range(1, 13)))
    print("  最右到達:", " ".join(f"{f}:{reach[f]}" for f in range(1, 13)))
    print("  成る前の最終位置:", " ".join(f"{f}:{final[f]}" for f in range(1, 13)))
    print("  最終位置が右端2筋の局:", len(edge_games))
    return edge_games


for kind in ("kirin", "phoenix"):
    for who in ("minase", "opp"):
        summarize(kind, who)
