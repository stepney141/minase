#!/usr/bin/env python3
"""match_runnerの実行ディレクトリから、手数帯ごとの時間消費と到達深さを集計する。

各局の`turns`にある実測思考時間から、ハーネスと同じ規則で両側の時計を
再構成し、手数帯ごとに平均到達深さ、思考時間の中央値、着手前の残り時間の
中央値、停止理由の分布を出力する。停止理由の記録がない旧形式では、
段階2着手前の予算式（soft = remaining/50 + 0.7·inc + 0.8·byoyomi、
hard = min(4·soft, remaining/4 + 0.8·byoyomi)）でhardを求め、思考時間が
hard以上の手の割合を推定hard打ち切り率として出す。この推定列は、
停止理由の記録が導入された時点で除く。

使い方:
    scripts/clock_profile.py data/matches/<測定名>
"""

import argparse
import json
import statistics
from collections import Counter
from pathlib import Path

PLY_BANDS = [(0, 50), (50, 100), (100, 150), (150, 200), (200, 300), (300, 400), (400, None)]


def band_of(ply):
    for band in PLY_BANDS:
        low, high = band
        if ply >= low and (high is None or ply < high):
            return band
    raise AssertionError("ply bands must cover all plies")


def legacy_hard_ms(remaining_ms, limit):
    """段階2着手前の予算式でhard limitを求める。"""
    increment = limit["increment_ms"]
    byoyomi_share = limit["byoyomi_ms"] * 8 // 10
    soft = remaining_ms // 50 + increment * 7 // 10 + byoyomi_share
    raw_hard = min(soft * 4, remaining_ms // 4 + byoyomi_share)
    safe_hard = max(remaining_ms + limit["byoyomi_ms"] - 30, 1)
    return min(max(raw_hard, 1), safe_hard)


def band_label(band):
    low, high = band
    return f"{low}-{high}" if high is not None else f"{low}-"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("run_dir", type=Path, help="match_runnerの実行ディレクトリ")
    args = parser.parse_args()

    manifest = json.loads((args.run_dir / "manifest.json").read_text())
    limits = {
        "candidate": manifest["candidate"]["limit"],
        "baseline": manifest["baseline"]["limit"],
    }
    for name, limit in limits.items():
        if limit.get("kind") != "time":
            raise SystemExit(f"{name}の思考制限が時間制御ではない: {limit}")

    depth = {band: [] for band in PLY_BANDS}
    think_ms = {band: [] for band in PLY_BANDS}
    remaining_ms = {band: [] for band in PLY_BANDS}
    stop_reasons = {band: Counter() for band in PLY_BANDS}
    estimated_hard_cuts = {band: 0 for band in PLY_BANDS}
    game_lengths = []

    for pair_path in sorted((args.run_dir / "pairs").glob("*.json")):
        pair = json.loads(pair_path.read_text())
        opening_plies = len(pair["opening"]["moves"])
        for game in pair["games"]:
            candidate_color = game["candidate_color"]
            role_of = {candidate_color: "candidate", ("white" if candidate_color == "black" else "black"): "baseline"}
            clocks = {color: limits[role_of[color]]["base_ms"] for color in ("black", "white")}
            game_lengths.append(opening_plies + len(game["turns"]))
            for index, turn in enumerate(game["turns"]):
                ply = opening_plies + index
                band = band_of(ply)
                color = turn["side"]
                limit = limits[role_of[color]]
                elapsed = turn["think_time_ns"] / 1e6
                remaining_ms[band].append(clocks[color])
                think_ms[band].append(elapsed)
                evaluation = turn.get("evaluation") or {}
                if "depth" in evaluation:
                    depth[band].append(evaluation["depth"])
                if "stop_reason" in turn:
                    stop_reasons[band][turn["stop_reason"] or "null"] += 1
                elif elapsed >= legacy_hard_ms(int(clocks[color]), limit):
                    estimated_hard_cuts[band] += 1
                clocks[color] = max(clocks[color] - elapsed, 0) + limit["increment_ms"]

    print(f"games={len(game_lengths)} mean_plies={statistics.mean(game_lengths):.0f} median_plies={statistics.median(game_lengths):.0f}")
    print("ply-band   turns  mean-depth  median-think-ms  median-remaining-ms  est-hard-cut%  stop-reasons")
    for band in PLY_BANDS:
        if not think_ms[band]:
            continue
        reasons = " ".join(f"{reason}={count}" for reason, count in sorted(stop_reasons[band].items())) or "欠測"
        estimated = 100 * estimated_hard_cuts[band] / len(think_ms[band])
        mean_depth = statistics.mean(depth[band]) if depth[band] else float("nan")
        print(
            f"{band_label(band):<9} {len(think_ms[band]):>7} {mean_depth:>11.2f} "
            f"{statistics.median(think_ms[band]):>16.0f} {statistics.median(remaining_ms[band]):>20.0f} "
            f"{estimated:>13.1f}  {reasons}"
        )


if __name__ == "__main__":
    main()
