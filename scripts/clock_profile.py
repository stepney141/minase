#!/usr/bin/env python3
"""match_runnerの実行ディレクトリから、手数帯ごとの時間消費と到達深さを集計する。

各局の`turns`にある実測思考時間から、ハーネスと同じ規則で両側の時計を
再構成し、手数帯ごとに平均到達深さ、思考時間と着手前の残り時間の中央値、
停止理由の分布、および最後のinfo以後の時間の中央値を出力する。
停止理由または最後のinfoの経過時間がない記録は欠測として扱い、推定しない。

使い方:
    scripts/clock_profile.py data/matches/<測定名> --budget quadruple-soft [--role candidate|baseline]
"""

import argparse
import json
import statistics
from collections import Counter
from pathlib import Path
from typing import Any

import clock_budget_stats as budgets

PLY_BANDS = [(0, 50), (50, 100), (100, 150), (150, 200), (200, 300), (300, 400), (400, None)]


def band_of(ply: int) -> tuple[int, int | None]:
    """通算plyを既存の手数帯へ分類する。"""
    for band in PLY_BANDS:
        low, high = band
        if ply >= low and (high is None or ply < high):
            return band
    raise AssertionError("ply bands must cover all plies")


def band_label(band: tuple[int, int | None]) -> str:
    """手数帯を表示用文字列にする。"""
    low, high = band
    return f"{low}-{high}" if high is not None else f"{low}-"


def main() -> None:
    """保存記録を再構成し、手数帯と役割・局面ごとの集計を出力する。"""
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("run_dir", type=Path, help="match_runnerの実行ディレクトリ")
    parser.add_argument(
        "--role",
        choices=("candidate", "baseline"),
        help="指定した側の手だけを集計する（省略時は両側）",
    )
    parser.add_argument("--budget", choices=budgets.FORMULAS, required=True)
    parser.add_argument("--json", type=Path, help="丸め前の集計JSONの保存先")
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
    after_last_info_ms = {band: [] for band in PLY_BANDS}
    game_lengths = []
    roles = (args.role,) if args.role is not None else ("candidate", "baseline")
    records_by_role: dict[str, list[dict[str, Any]]] = {role: [] for role in roles}

    for pair_path in sorted((args.run_dir / "pairs").glob("*.json")):
        pair = json.loads(pair_path.read_text())
        opening_plies = len(pair["opening"]["moves"])
        for game_index, game in enumerate(pair["games"]):
            candidate_color = game["candidate_color"]
            other_color = "white" if candidate_color == "black" else "black"
            role_of = {candidate_color: "candidate", other_color: "baseline"}
            game_lengths.append(opening_plies + len(game["turns"]))
            records = budgets.replay_game(
                game["turns"],
                {color: limits[role_of[color]] for color in budgets.COLORS},
                opening_plies,
                args.budget,
                game_id=f"{pair_path.stem}/game-{game_index + 1}",
            )
            for record in records:
                role = role_of[record["side"]]
                if role not in roles:
                    continue
                records_by_role[role].append(record)
                for turn in record["turns"]:
                    band = band_of(turn["ply"])
                    elapsed = turn["think_time_ns"] / budgets.NS_PER_MS
                    remaining_ms[band].append(turn["remaining_ns"] / budgets.NS_PER_MS)
                    think_ms[band].append(elapsed)
                    evaluation = turn.get("evaluation")
                    if evaluation is not None and evaluation.get("depth") is not None:
                        depth[band].append(evaluation["depth"])
                    reason = turn.get("stop_reason")
                    stop_reasons[band]["欠測" if reason is None else reason] += 1
                    if turn.get("completed_time_ms") is not None:
                        after_last_info_ms[band].append(elapsed - turn["completed_time_ms"])

    if not game_lengths:
        raise SystemExit("対局記録がない")
    print(
        f"games={len(game_lengths)} mean_plies={statistics.mean(game_lengths):.0f} "
        f"median_plies={statistics.median(game_lengths):.0f}"
    )
    print(
        "ply-band   turns  mean-depth  median-think-ms  median-remaining-ms  "
        "median-after-last-info-ms  stop-reasons"
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
        after_last_info = (
            f"{statistics.median(after_last_info_ms[band]):.0f}"
            if after_last_info_ms[band]
            else "欠測"
        )
        print(
            f"{band_label(band):<9} {len(think_ms[band]):>7} {mean_depth:>11.2f} "
            f"{statistics.median(think_ms[band]):>16.0f} {statistics.median(remaining_ms[band]):>20.0f} "
            f"{after_last_info:>25}  {reasons}"
        )

    summary = {
        "run_dir": str(args.run_dir.resolve()),
        "budget": args.budget,
        "roles": {role: budgets.summarize(records) for role, records in records_by_role.items()},
    }
    for role, stats in summary["roles"].items():
        print(f"\n{role} budget={args.budget}")
        print(budgets.format_summary(stats))
    if args.json is not None:
        args.json.write_text(json.dumps(summary, ensure_ascii=False, indent=2, allow_nan=False) + "\n")


if __name__ == "__main__":
    main()
