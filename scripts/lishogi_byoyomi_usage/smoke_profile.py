#!/usr/bin/env python3
"""秒読みつき条件の対局記録から、候補と基準の時計の使い方を集計する。

使い方: smoke_profile.py <実行ディレクトリ>
manifest.jsonの時間制御から各手の着手前の持ち時間を再現し、持ち時間期
（着手前の持ち時間が正）と秒読み期（0）に分けて、思考時間/秒読み、
完了深さ、途中結果（評価値の境界が下界）の割合を出力する。
時計の規則は対局ハーネスと同じく、持ち時間から思考時間を引いて0で止める。
"""
import json
import pathlib
import statistics
import sys
from collections import defaultdict

BANDS = [(0, 50), (50, 100), (100, 200), (200, 10**9)]


def clock_of(manifest):
    limit = manifest["candidate"]["limit"]
    if limit.get("kind") != "time" or not limit.get("byoyomi_ms"):
        raise SystemExit("time control with byoyomi not found in manifest.json")
    return limit["base_ms"], limit["increment_ms"], limit["byoyomi_ms"]


def band_of(ply):
    for low, high in BANDS:
        if low <= ply < high:
            return f"{low}-{high if high < 10**9 else ''}"
    raise AssertionError(ply)


def main(run_dir):
    run = pathlib.Path(run_dir)
    base_ms, increment_ms, byoyomi_ms = clock_of(json.loads((run / "manifest.json").read_text()))
    stats = defaultdict(lambda: defaultdict(list))
    exhaustion = defaultdict(list)
    total_think = defaultdict(list)
    for path in sorted((run / "pairs").glob("*.json")):
        record = json.loads(path.read_text())
        opening_plies = len(record["opening"]["moves"])
        for game in record["games"]:
            candidate_color = game["candidate_color"]
            remaining = {}
            spent = defaultdict(int)
            exhausted_at = {}
            for index, turn in enumerate(game["turns"]):
                ply = opening_plies + index
                side = turn["side"]
                role = "candidate" if side == candidate_color else "baseline"
                before = remaining.setdefault(side, base_ms)
                think_ms = turn["think_time_ns"] / 1e6
                spent[role] += think_ms
                phase = "main" if before > 0 else "byoyomi"
                if phase == "byoyomi" and role not in exhausted_at:
                    exhausted_at[role] = ply
                evaluation = turn.get("evaluation") or {}
                bound = evaluation.get("bound")
                depth = evaluation.get("depth")
                key = (role, phase)
                stats[key]["think_over_b"].append(think_ms / byoyomi_ms)
                stats[key]["partial"].append(1 if bound not in (None, "exact", "Exact") else 0)
                stats[key]["bounds"].append(str(bound))
                if depth is not None:
                    stats[(role, band_of(ply))]["depth"].append(depth)
                remaining[side] = max(0.0, before - think_ms) + increment_ms
            for role in ("candidate", "baseline"):
                exhaustion[role].append(exhausted_at.get(role))
                total_think[role].append(spent[role] / 1000)

    print(f"clock: time={base_ms}+{increment_ms},byoyomi={byoyomi_ms}")
    for role in ("candidate", "baseline"):
        for phase in ("main", "byoyomi"):
            values = stats[(role, phase)]
            if not values["think_over_b"]:
                continue
            bound_counts = defaultdict(int)
            for bound in values["bounds"]:
                bound_counts[bound] += 1
            print(
                f"{role:9} {phase:7} moves={len(values['think_over_b']):5d} "
                f"think/B median={statistics.median(values['think_over_b']):.3f} "
                f"mean={statistics.mean(values['think_over_b']):.3f} "
                f"bounds={dict(bound_counts)}"
            )
        bands = []
        for low, high in BANDS:
            depths = stats[(role, band_of(low))]["depth"]
            if depths:
                bands.append(f"{band_of(low)}:{statistics.mean(depths):.2f}(n={len(depths)})")
        exhausted = [ply for ply in exhaustion[role] if ply is not None]
        print(f"{role:9} depth by ply band: {' '.join(bands)}")
        print(
            f"{role:9} exhaustion ply median={statistics.median(exhausted) if exhausted else None} "
            f"not exhausted={sum(ply is None for ply in exhaustion[role])}/{len(exhaustion[role])} "
            f"total think per game median={statistics.median(total_think[role]):.1f}s"
        )


if __name__ == "__main__":
    main(sys.argv[1])
