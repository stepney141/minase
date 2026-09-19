#!/usr/bin/env python3
"""hit_rate.pyの出力から、予想手の出力率と的中率を集計する。"""
import gzip, json, statistics, sys

BANDS = [(0, 100), (100, 200), (200, 400), (400, 10**9)]


def main(path):
    opener = gzip.open if path.endswith(".gz") else open
    games = json.load(opener(path, "rt"))["games"]
    rows = [t for g in games for t in g["turns"] if t["next"] is not None]
    n = len(rows)
    predicted = [t for t in rows if t["predicted"] is not None]
    hits = [t for t in predicted if t["predicted"] == t["next"]]
    print(f"games {len(games)} plies {[g['plies'] for g in games]}")
    print(f"status {sorted({g['status'] for g in games})}")
    print(f"moves {n} predicted {len(predicted)} ({len(predicted)/n:.1%}) "
          f"hits {len(hits)} hit/moves {len(hits)/n:.1%} hit/predicted {len(hits)/len(predicted):.1%}")
    print(f"pv[0]!=bestmove {sum(not t['pv_matches_best'] for t in rows)}")
    for lo, hi in BANDS:
        band = [t for t in rows if lo <= t["ply"] < hi]
        if band:
            h = sum(t["predicted"] is not None and t["predicted"] == t["next"] for t in band)
            p = sum(t["predicted"] is not None for t in band)
            print(f"ply {lo}-{hi}: moves {len(band)} predicted {p/len(band):.1%} hit/moves {h/len(band):.1%}")
    per_game = [sum(t["predicted"] == t["next"] and t["predicted"] is not None for t in g["turns"] if t["next"])
                / max(1, sum(1 for t in g["turns"] if t["next"])) for g in games]
    print(f"per-game hit/moves min {min(per_game):.1%} median {statistics.median(per_game):.1%} max {max(per_game):.1%}")
    print(f"elapsed_ms median {statistics.median(t['elapsed_ms'] for t in rows)} "
          f"depth median {statistics.median(t['depth'] for t in rows)}")


if __name__ == "__main__":
    main(sys.argv[1])
