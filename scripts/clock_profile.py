#!/usr/bin/env python3
"""match_runnerの実行ディレクトリから、手数帯ごとの時間消費と到達深さを集計する。

各局の`turns`にある実測思考時間から、ハーネスと同じ規則で両側の時計を
再構成し、手数帯ごとに平均到達深さ、思考時間と着手前の残り時間の中央値、
停止理由の分布、および完了反復後に捨てられた計算時間の中央値を出力する。
停止理由または完了反復の経過時間がない記録は欠測として扱い、推定しない。

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


def band_label(band):
    low, high = band
    return f"{low}-{high}" if high is not None else f"{low}-"


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
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
    discarded_ms = {band: [] for band in PLY_BANDS}
    game_lengths = []

    for pair_path in sorted((args.run_dir / "pairs").glob("*.json")):
        pair = json.loads(pair_path.read_text())
        opening_plies = len(pair["opening"]["moves"])
        for game in pair["games"]:
            candidate_color = game["candidate_color"]
            other_color = "white" if candidate_color == "black" else "black"
            role_of = {candidate_color: "candidate", other_color: "baseline"}
            clocks = {
                color: limits[role_of[color]]["base_ms"]
                for color in ("black", "white")
            }
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
                if evaluation.get("depth") is not None:
                    depth[band].append(evaluation["depth"])
                if "stop_reason" in turn:
                    stop_reasons[band][turn["stop_reason"] or "null"] += 1
                else:
                    stop_reasons[band]["欠測"] += 1
                if turn.get("completed_time_ms") is not None:
                    discarded_ms[band].append(elapsed - turn["completed_time_ms"])
                clocks[color] = max(clocks[color] - elapsed, 0) + limit["increment_ms"]

    print(
        f"games={len(game_lengths)} mean_plies={statistics.mean(game_lengths):.0f} "
        f"median_plies={statistics.median(game_lengths):.0f}"
    )
    print(
        "ply-band   turns  mean-depth  median-think-ms  median-remaining-ms  "
        "median-discarded-ms  stop-reasons"
    )
    for band in PLY_BANDS:
        if not think_ms[band]:
            continue
        if set(stop_reasons[band]) == {"欠測"}:
            reasons = "欠測"
        else:
            reasons = " ".join(
                f"{reason}={count}"
                for reason, count in sorted(stop_reasons[band].items())
            )
        mean_depth = statistics.mean(depth[band]) if depth[band] else float("nan")
        discarded = (
            f"{statistics.median(discarded_ms[band]):.0f}"
            if discarded_ms[band]
            else "欠測"
        )
        print(
            f"{band_label(band):<9} {len(think_ms[band]):>7} {mean_depth:>11.2f} "
            f"{statistics.median(think_ms[band]):>16.0f} {statistics.median(remaining_ms[band]):>20.0f} "
            f"{discarded:>19}  {reasons}"
        )


if __name__ == "__main__":
    main()
