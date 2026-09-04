#!/usr/bin/env python3
"""match_runnerの実行ディレクトリから、対局コストを再集計する。

実行時間、対局時間、手数とその分布、並列利用率、手数帯ごとの思考時間、
長手数局の割合、および評価値による仮想投了の効果を出力する。

手数帯、手数分布、および1,000手超の判定はopeningを含む0始まりの通し番号で
数え、平均手数と手数中央値はエンジンが指した手（openingを除く）だけを数える。
後者は加算時間の累積を決める手数であり、設計書の表と同じ定義である。
仮想投了では`cp`を先手
視点へ正規化し、`mate_in`は評価視点側、`mated_in`は反対側への支持とする。
評価が欠測した手は支持の連続を切る。同じ側への支持が6手続いた手を裁定手と
し、その次の手以降の思考時間だけを節約量へ含める。節約率の分母は、勝者
未確定局を含む全局の総思考時間である。

使い方:
    scripts/match_cost_profile.py data/matches/<測定名> [...]
"""

import argparse
import json
import statistics
from collections import Counter
from pathlib import Path

THRESHOLDS = (5000, 3000, 2000, 1000)
THINK_BANDS = ((0, 100), (100, 300), (300, None))


def new_profile(label):
    return {
        "label": label,
        "pairs": 0,
        "active_wall_ns": 0,
        "capacity_ns": 0,
        "games": 0,
        "game_wall_ns": 0,
        "think_ns": 0,
        "lengths": [],
        "length_counts": Counter(),
        "band_think_ns": {band: 0 for band in THINK_BANDS},
        "long_games": 0,
        "long_game_wall_ns": 0,
        "max_pairs": 0,
        "max_pair_wall_ns": 0,
        "decided_games": 0,
        "resignations": {
            threshold: {"fired": 0, "mismatch": 0, "saved_ns": 0}
            for threshold in THRESHOLDS
        },
    }


def think_band(ply):
    for band in THINK_BANDS:
        low, high = band
        if ply >= low and (high is None or ply < high):
            return band
    raise AssertionError("思考時間帯が全手数を覆っていない")


def supported_side(evaluation, threshold):
    if evaluation is None:
        return None
    perspective = evaluation["perspective"]
    if perspective not in ("black", "white"):
        raise ValueError(f"未知の評価視点: {perspective}")

    score = evaluation["score"]
    kind = score["kind"]
    if kind == "cp":
        value = score["value"]
        black_value = value if perspective == "black" else -value
        if black_value >= threshold:
            return "black"
        if black_value <= -threshold:
            return "white"
        return None
    if kind == "mate_in":
        return perspective
    if kind == "mated_in":
        return "white" if perspective == "black" else "black"
    raise ValueError(f"未知の評価値種別: {kind}")


def virtual_resignations(game):
    states = {
        threshold: {"side": None, "count": 0, "result": None}
        for threshold in THRESHOLDS
    }
    remaining_ns = sum(turn["think_time_ns"] for turn in game["turns"])

    for turn in game["turns"]:
        remaining_ns -= turn["think_time_ns"]
        for threshold, state in states.items():
            if state["result"] is not None:
                continue
            side = supported_side(turn["evaluation"], threshold)
            if side is None:
                state["side"] = None
                state["count"] = 0
            elif side == state["side"]:
                state["count"] += 1
            else:
                state["side"] = side
                state["count"] = 1
            if state["count"] == 6:
                state["result"] = (side, remaining_ns)

    return {threshold: state["result"] for threshold, state in states.items()}


def load_profile(run_dir):
    manifest = json.loads((run_dir / "manifest.json").read_text())
    summary = json.loads((run_dir / "summary.json").read_text())
    profile = new_profile(run_dir.name)
    profile["active_wall_ns"] = summary["active_wall_time_ns"]
    profile["capacity_ns"] = profile["active_wall_ns"] * manifest["concurrency"]
    max_ply = manifest["max_ply"]

    pair_paths = sorted((run_dir / "pairs").glob("*.json"))
    profile["pairs"] = len(pair_paths)
    for pair_path in pair_paths:
        pair = json.loads(pair_path.read_text())
        opening_plies = len(pair["opening"]["moves"])
        pair_wall_ns = 0
        pair_reached_limit = False

        for game in pair["games"]:
            wall_ns = game["wall_time_ns"]
            length = opening_plies + len(game["turns"])
            think_ns = sum(turn["think_time_ns"] for turn in game["turns"])
            pair_wall_ns += wall_ns
            profile["games"] += 1
            profile["game_wall_ns"] += wall_ns
            profile["think_ns"] += think_ns
            profile["lengths"].append(len(game["turns"]))
            profile["length_counts"][(length // 100) * 100] += 1

            if length > 1000:
                profile["long_games"] += 1
                profile["long_game_wall_ns"] += wall_ns
            if game["termination"]["kind"] == "cutoff" or length >= max_ply:
                pair_reached_limit = True

            for index, turn in enumerate(game["turns"]):
                ply = opening_plies + index
                profile["band_think_ns"][think_band(ply)] += turn["think_time_ns"]

            winner = game["termination"].get("winner")
            if winner is None:
                continue
            if winner not in ("black", "white"):
                raise ValueError(f"未知の勝者: {winner}")
            profile["decided_games"] += 1
            for threshold, result in virtual_resignations(game).items():
                if result is None:
                    continue
                predicted_winner, saved_ns = result
                profile["resignations"][threshold]["fired"] += 1
                profile["resignations"][threshold]["mismatch"] += (
                    predicted_winner != winner
                )
                profile["resignations"][threshold]["saved_ns"] += saved_ns

        if pair_reached_limit:
            profile["max_pairs"] += 1
            profile["max_pair_wall_ns"] += pair_wall_ns

    if profile["games"] == 0:
        raise ValueError(f"対局記録がない: {run_dir}")
    return profile


def merge_profiles(profiles):
    merged = new_profile("入力全体")
    for profile in profiles:
        for key in (
            "pairs",
            "active_wall_ns",
            "capacity_ns",
            "games",
            "game_wall_ns",
            "think_ns",
            "long_games",
            "long_game_wall_ns",
            "max_pairs",
            "max_pair_wall_ns",
            "decided_games",
        ):
            merged[key] += profile[key]
        merged["lengths"].extend(profile["lengths"])
        merged["length_counts"].update(profile["length_counts"])
        for band in THINK_BANDS:
            merged["band_think_ns"][band] += profile["band_think_ns"][band]
        for threshold in THRESHOLDS:
            for key in ("fired", "mismatch", "saved_ns"):
                merged["resignations"][threshold][key] += profile["resignations"][threshold][key]
    return merged


def print_histogram(profile):
    print("手数分布（100手幅）")
    entries = []
    for low, count in sorted(profile["length_counts"].items()):
        entries.append(f"{low:>4}-{low + 99:<4}={count:>4}")
    for start in range(0, len(entries), 4):
        print("  " + "  ".join(entries[start : start + 4]))


def band_label(band):
    low, high = band
    if high is None:
        return f"{low}手目以降"
    return f"{low}〜{high - 1}手目"


def ratio(numerator, denominator):
    return numerator / denominator if denominator else 0


def print_profile(profile):
    hours = profile["active_wall_ns"] / 1e9 / 3600
    mean_seconds = profile["game_wall_ns"] / profile["games"] / 1e9
    print(f"== {profile['label']} ==")
    print("実行ペア  実行時間  ペア/時  局数  1局平均  平均手数  手数中央値（手数は開始手順を除く）")
    print(
        f"{profile['pairs']:>8}  {hours:>6.2f}時間  "
        f"{profile['pairs'] / hours:>7.0f}  {profile['games']:>4}  "
        f"{mean_seconds:>6.0f}秒  {statistics.mean(profile['lengths']):>8.0f}  "
        f"{statistics.median(profile['lengths']):>10.0f}"
    )
    print_histogram(profile)
    print(
        f"並列利用率={ratio(profile['game_wall_ns'], profile['capacity_ns']):.2f}  "
        f"思考時間/局時間={ratio(profile['think_ns'], profile['game_wall_ns']):.2f}"
    )
    print("手数帯        思考時間比")
    for band in THINK_BANDS:
        print(
            f"{band_label(band):<12} "
            f"{ratio(profile['band_think_ns'][band], profile['think_ns']):>10.0%}"
        )
    print(
        f"1,000手超: {profile['long_games']}局 "
        f"（局数比={ratio(profile['long_games'], profile['games']):.0%}, "
        f"対局時間比={ratio(profile['long_game_wall_ns'], profile['game_wall_ns']):.0%}）"
    )
    print(
        f"上限到達: {profile['max_pairs']}ペア "
        f"（対局時間比={ratio(profile['max_pair_wall_ns'], profile['game_wall_ns']):.0%}）"
    )
    print(f"仮想投了（勝者確定={profile['decided_games']}局）")
    print("閾値(cp)  発火局数  不一致局数（発火比）  思考時間節約率")
    for threshold in THRESHOLDS:
        result = profile["resignations"][threshold]
        print(
            f"{threshold:>8,}  {result['fired']:>8}  {result['mismatch']:>8} "
            f"（{ratio(result['mismatch'], result['fired']):>5.1%}）"
            f"{ratio(result['saved_ns'], profile['think_ns']):>16.0%}"
        )
    print()


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument(
        "run_dirs",
        nargs="+",
        type=Path,
        metavar="RUN_DIR",
        help="match_runnerの実行ディレクトリ",
    )
    args = parser.parse_args()

    profiles = [load_profile(run_dir) for run_dir in args.run_dirs]
    for profile in profiles:
        print_profile(profile)
    if len(profiles) > 1:
        print_profile(merge_profiles(profiles))


if __name__ == "__main__":
    main()
