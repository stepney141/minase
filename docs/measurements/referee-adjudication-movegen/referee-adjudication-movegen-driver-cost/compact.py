"""交互測定の全出力から、各探索の時間と回数の表と、出力ファイルのSHA-256を作る。

使い方: python3 -I compact.py <交互測定の出力先> <結果のJSON>

表の各行は、局面の分類、シード、探索方式、再生した着手数、および比較元と候補の各回の
elapsed_msを持つ。時間以外の項目はanalyze.pyが全回で一致することを確かめている。
"""

import hashlib
import json
import pathlib
import sys


def main():
    directory = pathlib.Path(sys.argv[1])
    labels = ("before", "after")
    documents = {
        label: [
            json.loads((directory / f"{label}-{index}.json").read_text(encoding="utf-8"))
            for index in range(1, 6)
        ]
        for label in labels
    }
    rows = []
    for index, row in enumerate(documents["before"][0]["searches"]):
        rows.append(
            {
                "category": row["input"]["category"],
                "seed": row["input"]["seed"],
                "plies": row["input"]["plies"],
                "mode": row["mode"],
                "replayed_moves": row["replayed_moves"],
                "elapsed_ms": {
                    label: [document["searches"][index]["elapsed_ms"] for document in documents[label]]
                    for label in labels
                },
            }
        )
    hashes = {
        path.name: hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(directory.glob("*-*.json"))
    }
    with open(sys.argv[2], "w", encoding="utf-8") as file:
        json.dump({"output_sha256": hashes, "searches": rows}, file, indent=1)


if __name__ == "__main__":
    main()
