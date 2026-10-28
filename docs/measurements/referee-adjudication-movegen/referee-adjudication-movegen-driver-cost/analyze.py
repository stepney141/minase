"""driver_costの交互測定の出力を照合し、再生1手あたりに正規化した探索時間を集計する。

使い方: python3 -I analyze.py --before before-1.json ... --after after-1.json ...

照合は、時間の項目（各探索のelapsed_ms、全体のsummary_ms、snapshots）を除いた出力が、
同じ版の各回の間と、比較元と候補の間で一致することを確かめる。
正規化は、各（局面, 探索方式）について各版の反復の中央値を取り、再生した着手数で割ってから、
局面の分類と探索方式ごとに中央値を取る。分位点は昇順の第ceil(p*n)標本とする。
"""

import argparse
import json
import math
import sys

TIME_KEYS_TOP = ("summary_ms", "snapshots")
TIME_KEYS_SEARCH = ("elapsed_ms",)


def nearest_rank(values, p):
    ordered = sorted(values)
    return ordered[math.ceil(p * len(ordered)) - 1]


def strip_time(document):
    stripped = {key: value for key, value in document.items() if key not in TIME_KEYS_TOP}
    stripped["searches"] = [
        {key: value for key, value in row.items() if key not in TIME_KEYS_SEARCH}
        for row in document["searches"]
    ]
    return stripped


def load(paths):
    documents = []
    for path in paths:
        with open(path, encoding="utf-8") as file:
            documents.append(json.load(file))
    return documents


def first_difference(left, right, path="$"):
    if type(left) is not type(right):
        return path
    if isinstance(left, dict):
        for key in sorted(set(left) | set(right)):
            if key not in left or key not in right:
                return f"{path}.{key}"
            found = first_difference(left[key], right[key], f"{path}.{key}")
            if found:
                return found
        return None
    if isinstance(left, list):
        if len(left) != len(right):
            return f"{path}[len]"
        for index, (a, b) in enumerate(zip(left, right)):
            found = first_difference(a, b, f"{path}[{index}]")
            if found:
                return found
        return None
    return None if left == right else path


def check_identical(documents, label):
    reference = strip_time(documents[0])
    for index, document in enumerate(documents[1:], start=2):
        difference = first_difference(reference, strip_time(document))
        if difference:
            sys.exit(f"{label}: round {index} differs from round 1 at {difference}")
    return reference


def per_search_medians(documents):
    rows = []
    for index, row in enumerate(documents[0]["searches"]):
        times = [document["searches"][index]["elapsed_ms"] for document in documents]
        rows.append(
            {
                "category": row["input"]["category"],
                "mode": row["mode"],
                "replayed_moves": row["replayed_moves"],
                "median_ms": nearest_rank(times, 0.5),
            }
        )
    return rows


def summarize(rows):
    groups = {}
    for row in rows:
        for key in ((row["mode"], row["category"]), (row["mode"], "all")):
            groups.setdefault(key, []).append(row)
    summary = {}
    for (mode, category), members in sorted(groups.items()):
        replayed = [row for row in members if row["replayed_moves"] > 0]
        summary[f"{mode}/{category}"] = {
            "searches": len(members),
            "searches_with_replay": len(replayed),
            "median_ms": nearest_rank([row["median_ms"] for row in members], 0.5),
            "median_us_per_replayed_move": (
                nearest_rank(
                    [row["median_ms"] * 1000.0 / row["replayed_moves"] for row in replayed], 0.5
                )
                if replayed
                else None
            ),
        }
    return summary


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--before", nargs="+", required=True)
    parser.add_argument("--after", nargs="+", required=True)
    arguments = parser.parse_args()

    before = load(arguments.before)
    after = load(arguments.after)
    before_reference = check_identical(before, "before")
    after_reference = check_identical(after, "after")
    difference = first_difference(before_reference, after_reference)
    if difference:
        sys.exit(f"before and after differ outside the time fields at {difference}")

    before_summary = summarize(per_search_medians(before))
    after_summary = summarize(per_search_medians(after))
    result = {}
    for key, before_values in before_summary.items():
        after_values = after_summary[key]
        ratio = None
        if before_values["median_us_per_replayed_move"]:
            ratio = (
                after_values["median_us_per_replayed_move"]
                / before_values["median_us_per_replayed_move"]
            )
        result[key] = {"before": before_values, "after": after_values, "ratio_per_replayed_move": ratio}
    print(
        json.dumps(
            {
                "rounds": {"before": len(before), "after": len(after)},
                "identical_outside_time_fields": True,
                "groups": result,
            },
            indent=2,
            ensure_ascii=False,
        )
    )


if __name__ == "__main__":
    main()
