#!/usr/bin/env python3
"""固定時間の対局記録から、候補と基準の思考時間と完了深さを集計する。

使い方: smoke_profile.py <実行ディレクトリ>
時間、停止理由、評価値の境界、および途中結果率は、異常や投了を含む全応答を
対象とする。完了深さは報告のある応答だけを数え、lowerの場合だけ1を引く。
超過量は思考時間からmovetimeを引いた符号つきの値とし、99パーセンタイルは
statistics.quantilesのinclusive方式で求める。標本が1つなら、その値を使う。
標本がない統計量はn/aと表示し、深さのnは深さが報告された応答数を表す。
"""

import argparse
from collections import Counter
import json
from pathlib import Path
import statistics


ROLES = ("candidate", "baseline")
BANDS = ("0-49", "50-99", "100-199", "200+")


def nonnegative_int(value, name):
    if type(value) is not int or value < 0:
        raise ValueError(f"{name} must be a nonnegative integer")
    return value


def limit_of(manifest):
    limits = []
    for role in ROLES:
        limit = manifest[role]["limit"]
        if limit["kind"] != "movetime":
            raise ValueError(f"{role}.limit.kind must be movetime")
        movetime_ms = nonnegative_int(limit["movetime_ms"], f"{role}.movetime_ms")
        timemargin_ms = nonnegative_int(limit["timemargin_ms"], f"{role}.timemargin_ms")
        if movetime_ms == 0:
            raise ValueError(f"{role}.movetime_ms must be positive")
        limits.append((movetime_ms, timemargin_ms))
    if limits[0] != limits[1]:
        raise ValueError("candidate and baseline movetime limits must match")
    return limits[0]


def band_of(ply):
    if ply < 50:
        return BANDS[0]
    if ply < 100:
        return BANDS[1]
    if ply < 200:
        return BANDS[2]
    return BANDS[3]


def print_role(role, values, movetime_ms):
    responses = values["responses"]
    count = len(values["think_ns"])
    other = {kind: n for kind, n in responses.items() if kind != "move"}
    print(f"{role} turns={count} moves={responses['move']} "
          f"other_responses={json.dumps(other, sort_keys=True)}")
    if count:
        movetime_ns = movetime_ms * 1_000_000
        ratios = [ns / movetime_ns for ns in values["think_ns"]]
        overshoots = [(ns - movetime_ns) / 1_000_000 for ns in values["think_ns"]]
        p99 = (statistics.quantiles(overshoots, n=100, method="inclusive")[98]
               if count > 1 else overshoots[0])
        print(f"{role} think/movetime median={statistics.median(ratios):.6f} "
              f"mean={statistics.mean(ratios):.6f} min={min(ratios):.6f}")
        print(f"{role} overshoot_ms max={max(overshoots):.3f} p99={p99:.3f} "
              f"over_movetime={sum(ns > movetime_ns for ns in values['think_ns'])}/{count}")
        lower_percent = f"{values['bounds']['lower'] / count:.3%}"
    else:
        print(f"{role} think/movetime median=n/a mean=n/a min=n/a")
        print(f"{role} overshoot_ms max=n/a p99=n/a over_movetime=0/0")
        lower_percent = "n/a"
    print(f"{role} stop_reasons={json.dumps(values['stops'], sort_keys=True)}")
    print(f"{role} bounds={json.dumps(values['bounds'], sort_keys=True)} "
          f"lower={values['bounds']['lower']}/{count} ({lower_percent})")
    bands = []
    for band in BANDS:
        depths = values["depths"][band]
        mean = f"{statistics.mean(depths):.3f}" if depths else "n/a"
        bands.append(f"{band}:mean={mean}(n={len(depths)})")
    print(f"{role} completed_depth by ply band: {' '.join(bands)}")


def profile(run):
    manifest = json.loads((run / "manifest.json").read_text(encoding="utf-8"))
    movetime_ms, timemargin_ms = limit_of(manifest)
    paths = sorted((run / "pairs").glob("*.json"))
    if not paths:
        raise ValueError(f"no pair records found in {run / 'pairs'}")
    stats = {
        role: {
            "responses": Counter(), "think_ns": [], "stops": Counter(),
            "bounds": Counter(), "depths": {band: [] for band in BANDS},
        }
        for role in ROLES
    }
    pair_times = []
    for path in paths:
        record = json.loads(path.read_text(encoding="utf-8"))
        opening_plies = len(record["opening"]["moves"])
        wall_time_ns = 0
        for game in record["games"]:
            candidate_color = game["candidate_color"]
            if candidate_color not in ("black", "white"):
                raise ValueError(f"{path}: invalid candidate_color {candidate_color!r}")
            wall_time_ns += nonnegative_int(game["wall_time_ns"], f"{path}: wall_time_ns")
            for index, turn in enumerate(game["turns"]):
                context = f"{path}: candidate_color={candidate_color}, turn={index}"
                side = turn["side"]
                if side not in ("black", "white"):
                    raise ValueError(f"{context}: invalid side {side!r}")
                role = "candidate" if side == candidate_color else "baseline"
                values = stats[role]
                values["responses"][turn["response"]["kind"]] += 1
                values["think_ns"].append(
                    nonnegative_int(turn["think_time_ns"], f"{context}: think_time_ns")
                )
                stop = turn["stop_reason"]
                values["stops"]["null" if stop is None else stop] += 1
                evaluation = turn["evaluation"]
                if evaluation is None:
                    values["bounds"]["null"] += 1
                    continue
                bound = evaluation["bound"]
                if bound not in ("exact", "lower", "upper"):
                    raise ValueError(f"{context}: invalid evaluation.bound {bound!r}")
                values["bounds"][bound] += 1
                depth = evaluation["depth"]
                if depth is not None:
                    depth = nonnegative_int(depth, f"{context}: evaluation.depth")
                    if bound == "lower":
                        if depth == 0:
                            raise ValueError(f"{context}: lower-bound depth must be positive")
                        depth -= 1
                    values["depths"][band_of(opening_plies + index)].append(depth)
        pair_times.append(wall_time_ns / 1_000_000_000)

    print(f"limit: movetime={movetime_ms},timemargin={timemargin_ms} (ms)")
    print("statistics: all recorded turns; depth n excludes null depths; p99=inclusive")
    for role in ROLES:
        print_role(role, stats[role], movetime_ms)
    print(f"pairs={len(pair_times)} wall_time_s "
          f"mean={statistics.mean(pair_times):.3f} max={max(pair_times):.3f}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_dir", type=Path)
    args = parser.parse_args()
    try:
        profile(args.run_dir)
    except KeyError as error:
        parser.error(f"missing required field: {error}")
    except (OSError, ValueError, TypeError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
