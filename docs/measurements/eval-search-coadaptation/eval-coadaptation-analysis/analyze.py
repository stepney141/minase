#!/usr/bin/env python3
"""共通ペアによる共適応の判定。設計: docs/plans/eval-search-coadaptation.md。

有限でない点推定と計算できない信頼区間は JSON の null で表す。
採否測定の条件は判定と別に出力し、信頼区間が計算できなければ null とする。
"""

import argparse
import json
import math
from pathlib import Path
import random
import sys


BOOTSTRAP_REPLICATES = 10_000
BOOTSTRAP_SEED = 20261004
MANIFEST_FIELDS = (
    "seed",
    "candidate.limit",
    "baseline.limit",
    "concurrency",
    "hash_mb.candidate",
    "hash_mb.baseline",
    "baseline.identity.sha256",
)


def read_json(path):
    with path.open(encoding="utf-8") as stream:
        value = json.load(stream)
    if not isinstance(value, dict):
        raise ValueError(f"{path}: JSON オブジェクトが必要です")
    return value


def manifest_conditions(directory):
    path = directory / "manifest.json"
    manifest = read_json(path)
    conditions = {}
    for field in MANIFEST_FIELDS:
        value = manifest
        for key in field.split("."):
            if not isinstance(value, dict) or key not in value:
                raise ValueError(f"{path}: {field} がありません")
            value = value[key]
        if value is None:
            raise ValueError(f"{path}: {field} が null です")
        conditions[field] = value
    return conditions


def load_pairs(directory):
    paths = sorted((directory / "pairs").glob("*.json"))
    if not paths:
        raise ValueError(f"{directory / 'pairs'}: ペアの JSON ファイルがありません")
    pairs = {}
    for path in paths:
        pair = read_json(path)
        if "pair_number" not in pair or "category" not in pair:
            raise ValueError(f"{path}: pair_number と category が必要です")
        number, category = pair["pair_number"], pair["category"]
        if type(number) is not int:
            raise ValueError(f"{path}: pair_number は整数である必要があります")
        if category is not None and (
            type(category) is not int or not 0 <= category <= 4
        ):
            raise ValueError(f"{path}: category は整数 0..4 または null が必要です")
        if number in pairs:
            raise ValueError(f"{path}: pair_number {number} が重複しています")
        pairs[number] = category
    return pairs


def elo(total, count):
    if count == 0 or total == 0 or total == 4 * count:
        return None
    score = total / (4 * count)
    return 400 * math.log10(score / (1 - score))


def statistics(totals, count):
    values = {"E0": elo(totals[0], count)}
    if len(totals) >= 2:
        values["E1"] = elo(totals[1], count)
        values["R"] = values["D1"] = None
        if values["E0"] is not None and values["E1"] is not None:
            values["R"] = values["E1"] - values["E0"]
            values["D1"] = values["E1"] - 0.5 * values["E0"]
    if len(totals) == 3:
        values["G"] = elo(totals[2], count)
        values["D2"] = None
        if values["D1"] is not None and values["G"] is not None:
            values["D2"] = values["D1"] - values["G"]
    return values


def bootstrap(rows, names):
    """同じペア番号の行を全測定で共有し、非有限な標本も数える。"""
    rng = random.Random(BOOTSTRAP_SEED)
    distributions = {name: [] for name in names}
    nonfinite = 0
    for _ in range(BOOTSTRAP_REPLICATES):
        sample = rng.choices(rows, k=len(rows))
        values = statistics([sum(column) for column in zip(*sample)], len(rows))
        if any(value is None for value in values.values()):
            nonfinite += 1
        else:
            for name, value in values.items():
                distributions[name].append(value)
    # 非有限な標本を捨てた分布で区間を作ってはならない。
    if nonfinite:
        return {name: None for name in names}, nonfinite
    lower = math.floor(0.025 * BOOTSTRAP_REPLICATES)
    upper = math.ceil(0.975 * BOOTSTRAP_REPLICATES) - 1
    intervals = {}
    for name, values in distributions.items():
        values.sort()
        intervals[name] = [values[lower], values[upper]]
    return intervals, nonfinite


def classify(stage, intervals):
    if stage == "stage0":
        return (
            "負けが大きい"
            if intervals["E0"][1] < -50
            else "所定の大きさの負けを確認できない"
        )
    if stage == "stage1":
        if intervals["D1"][0] >= 0:
            return "回復が大きい"
        if intervals["R"][0] > 0:
            return "回復が部分的である"
        return "回復を示せない"
    return (
        "共適応を支持する"
        if intervals["D2"][0] >= 0
        else "共適応に固有とは示せない"
    )


def analyze(stage, directories):
    conditions = {
        name: manifest_conditions(directory) for name, directory in directories.items()
    }
    mismatches = [
        {"measurement": name, "field": field,
         "before": conditions["before"][field], "actual": fields[field]}
        for name, fields in conditions.items()
        if name != "before"
        for field in MANIFEST_FIELDS
        if fields[field] != conditions["before"][field]
    ]
    verification = {
        "status": "不一致" if mismatches else (
            "対象外（測定が1つ）" if stage == "stage0" else "一致"
        ),
        "conditions": conditions,
        "mismatches": mismatches,
    }
    if mismatches:
        return {"stage": stage, "error": "測定条件が一致しません",
                "manifest_verification": verification}, 1

    measurements = {}
    completed = {}
    for name, directory in directories.items():
        pairs = load_pairs(directory)
        complete = {number: value for number, value in pairs.items() if value is not None}
        completed[name] = complete
        own_elo = elo(sum(complete.values()), len(complete))
        measurements[name] = {
            "directory": str(directory.resolve()),
            "completed_pairs": len(complete),
            "discarded_pair_numbers": sorted(n for n, c in pairs.items() if c is None),
            "elo": own_elo,
            "elo_unavailable_reason": (
                "完走ペアがありません" if not complete else
                "正規化得点が0または1で、Eloが有限ではありません"
            ) if own_elo is None else None,
        }
    numbers = sorted(set.intersection(*(set(pairs) for pairs in completed.values())))
    rows = [tuple(pairs[number] for pairs in completed.values()) for number in numbers]
    totals = [sum(pairs[number] for number in numbers) for pairs in completed.values()]
    point = statistics(totals, len(numbers))
    reasons = []
    if len(numbers) < 200:
        reasons.append(f"共通ペアが200未満です（{len(numbers)}ペア）")
    if stage != "stage0" and point["E0"] is not None and point["E0"] >= 0:
        reasons.append("共通ペアのE0の点推定が0以上です")
    if numbers and any(value is None for value in point.values()):
        reasons.append("共通ペアの点推定の正規化得点が0または1で、Eloが有限ではありません")

    intervals = {name: None for name in point}
    nonfinite = 0
    executed = 0
    if numbers and all(value is not None for value in point.values()):
        intervals, nonfinite = bootstrap(rows, point)
        executed = BOOTSTRAP_REPLICATES
        if nonfinite:
            reasons.append(
                f"復元標本の正規化得点が0または1で、Eloが有限ではありません（{nonfinite}標本）"
            )

    references = {}
    if stage != "stage0":
        references["recovery_rate"] = (
            point["R"] / abs(point["E0"])
            if point["E0"] is not None and point["E0"] < 0 and point["R"] is not None
            else None
        )
    if stage == "stage2":
        references["control_adjusted_recovery_rate"] = (
            (point["R"] - point["G"]) / abs(point["E0"])
            if references["recovery_rate"] is not None and point["G"] is not None
            else None
        )
    result = {
        "stage": stage,
        "manifest_verification": verification,
        "measurements": measurements,
        "common_pairs": {"count": len(numbers), "pair_numbers": numbers},
        "statistics": {name: {"estimate": value, "ci95": intervals[name]}
                       for name, value in point.items()},
        "reference_estimates": references,
        "bootstrap": {
            "replicates": BOOTSTRAP_REPLICATES,
            "executed_replicates": executed,
            "seed": BOOTSTRAP_SEED,
            "sampling": "ペア番号の昇順の行を random.Random(seed).choices で復元抽出し、全測定で共有",
            "percentile_definition": "昇順に整列したB個の値の0始まりの添字 floor(0.025*B) と ceil(0.975*B)-1",
            "percentile_indices_zero_based": [250, 9749],
            "nonfinite_replicates": nonfinite,
        },
        "classification": "判定不能" if reasons else classify(stage, intervals),
        "indeterminate_reasons": reasons,
    }
    if stage == "stage1":
        ci = intervals["E1"]
        result["adoption_measurement_condition"] = ci[0] > 0 if ci is not None else None
    if stage == "stage2":
        ci = intervals["G"]
        result["control_ci_lower_gt_zero"] = ci[0] > 0 if ci is not None else None
    return result, 0


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    stages = parser.add_subparsers(dest="stage", required=True)
    for number in range(3):
        subparser = stages.add_parser(f"stage{number}")
        subparser.add_argument("--before", type=Path, required=True)
        if number >= 1:
            subparser.add_argument("--after", type=Path, required=True)
        if number == 2:
            subparser.add_argument("--control", type=Path, required=True)
    args = parser.parse_args()
    directories = {name: path for name, path in vars(args).items() if name != "stage"}
    try:
        result, status = analyze(args.stage, directories)
    except (OSError, ValueError) as error:
        result, status = {"stage": args.stage, "error": str(error)}, 1
    print(json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False))
    return status


if __name__ == "__main__":
    sys.exit(main())
