"""計測用ビルドのdriver_costの出力から、詰み判定で試した候補手の数を集計する。

使い方: python3 -I count_summary.py <名前>=<出力のJSON> ...

各出力の`mate_candidates`を探索方式と局面の分類ごとに合計し、再生した着手数の合計で割った
再生1手あたりの値を併記する。時間の値は計測用ビルドのものなので読まない。
"""

import json
import sys


def main():
    table = {}
    for argument in sys.argv[1:]:
        name, path = argument.split("=", 1)
        with open(path, encoding="utf-8") as file:
            document = json.load(file)
        for row in document["searches"]:
            for category in (row["input"]["category"], "all"):
                key = f'{row["mode"]}/{category}'
                entry = table.setdefault(key, {}).setdefault(
                    name, {"mate_candidates": 0, "replayed_moves": 0}
                )
                entry["mate_candidates"] += row["mate_candidates"]
                entry["replayed_moves"] += row["replayed_moves"]
    for entries in table.values():
        for entry in entries.values():
            replayed = entry["replayed_moves"]
            entry["per_replayed_move"] = entry["mate_candidates"] / replayed if replayed else None
    print(json.dumps(dict(sorted(table.items())), indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
