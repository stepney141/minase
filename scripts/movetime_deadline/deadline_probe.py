#!/usr/bin/env python3
"""同じエンジンと局面で2つのgo引数の完了深さと途中結果を比べる。

局面数は既定200、抽出のシードは1とする。
第6引数に出力先を指定すると、1始まりの局面番号、手数、および
A・Bのdepth、partial、changedを局面ごとにJSON Linesで書く。
"""

import argparse
from contextlib import ExitStack
import json
from pathlib import Path
import statistics
import sys

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR.parent / "lishogi_byoyomi_usage"))
import paired_depth_probe


def close_engine(engine):
    """quitを送り、プロセスの終了を待ってパイプを閉じる。"""
    with engine.proc:
        engine.send("quit")
        engine.proc.wait()


def main():
    parser = argparse.ArgumentParser(
        description=__doc__,
        usage=("%(prog)s <実行ディレクトリ> <エンジンのパス> <go引数A> "
               "<go引数B> [局面数 [JSON Lines出力先]]"),
    )
    parser.add_argument("run_dir")
    parser.add_argument("engine_path")
    parser.add_argument("go_a")
    parser.add_argument("go_b")
    parser.add_argument("count", nargs="?", type=int, default=200)
    parser.add_argument("jsonl", nargs="?")
    args = parser.parse_args()
    if args.count < 2:
        parser.error("標本標準偏差の計算には局面数が2以上必要です")

    picked = paired_depth_probe.positions(args.run_dir, args.count)
    rows = []
    with ExitStack() as stack:
        output = None
        if args.jsonl is not None:
            output = stack.enter_context(open(args.jsonl, "w", encoding="utf-8"))
        engines = {}
        for name in ("A", "B"):
            engine = paired_depth_probe.Engine(args.engine_path)
            stack.callback(close_engine, engine)
            engines[name] = engine

        for index, moves in enumerate(picked, start=1):
            row = {"position": index, "ply": len(moves)}
            for name, go in (("A", args.go_a), ("B", args.go_b)):
                depth, partial, changed = engines[name].search(moves, go)
                row[name] = {"depth": depth, "partial": partial, "changed": changed}
            rows.append(row)
            if output is not None:
                output.write(json.dumps(row) + "\n")

    count = len(rows)
    print(f"go A: {args.go_a}")
    print(f"go B: {args.go_b}")
    print(f"positions: {count}")
    print(f"engine: {args.engine_path}")
    for name in ("A", "B"):
        depths = [row[name]["depth"] for row in rows]
        partial = sum(row[name]["partial"] for row in rows)
        changed = sum(row[name]["changed"] for row in rows)
        print(f"{name} mean depth={statistics.mean(depths):.3f} "
              f"partial={partial}/{count} ({partial / count:.1%}) "
              f"changed={changed}/{count} ({changed / count:.1%})")

    diffs = [row["B"]["depth"] - row["A"]["depth"] for row in rows]
    increased = sum(diff > 0 for diff in diffs)
    decreased = sum(diff < 0 for diff in diffs)
    unchanged = sum(diff == 0 for diff in diffs)
    mean = statistics.mean(diffs)
    se = statistics.stdev(diffs) / count ** 0.5
    print(f"paired depth diff (B - A): "
          f"increased={increased}/{count} ({increased / count:.1%}) "
          f"decreased={decreased}/{count} ({decreased / count:.1%}) "
          f"unchanged={unchanged}")
    print(f"paired depth diff (B - A): mean={mean:+.3f} se={se:.3f}")
    regression = mean < 0 and abs(mean) >= 2 * se
    print(f"depth_regression: {'yes' if regression else 'no'}")


if __name__ == "__main__":
    main()
