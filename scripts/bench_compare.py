#!/usr/bin/env python3
"""作業ツリーのbenchを照合参照コミットおよび基準コミットと比較する。

照合参照コミットとは局面別のノード数、最善手、探索値を全行で比較し、
不一致があれば終了コード1で報告する。速度は3者（基準、参照、候補）の
`--repetitions`回の中央値NPSで比べ、候補の基準比と参照比を、ノード数比
および経過時間比とともに出力する（docs/plans/movegen-speedup.md「検証」）。

コミットは`git archive`で`target/bench-cache/<完全ハッシュ>/src`へ展開し、
通常のrelease設定でビルドする。参照の局面別出力は1回実行の結果を同じ
ディレクトリへ保存して再利用する。候補は作業ツリーをビルドする。

使い方:
    scripts/bench_compare.py --reference <参照コミット> --baseline <基準コミット>
        [--depth 5] [--repetitions 3] [--threads 1] [--json <出力パス>]
"""

import argparse
import json
import re
import statistics
import subprocess
import sys
from pathlib import Path

POSITION_LINE = re.compile(
    r"^position=(?P<name>\S+) depth=(?P<depth>\d+) nodes=(?P<nodes>\d+)"
    r"(?: best=(?P<best>\S+) score=(?P<score>-?\d+))? elapsed=(?P<elapsed>[\d.]+)s$"
)
MEDIAN_LINE = re.compile(
    r"^median: depth=(?P<depth>\d+) nodes=(?P<nodes>\d+) elapsed=(?P<elapsed>[\d.]+)s nps=(?P<nps>\d+)$"
)


def run(command, cwd=None):
    return subprocess.run(command, cwd=cwd, check=True, capture_output=True, text=True).stdout


def resolve_commit(root, spec):
    return run(["git", "rev-parse", "--verify", f"{spec}^{{commit}}"], cwd=root).strip()


def build_commit(root, commit):
    """コミットをキャッシュへ展開してbenchをビルドし、バイナリのパスを返す。"""
    cache = root / "target" / "bench-cache" / commit
    source = cache / "src"
    binary = source / "target" / "release" / "bench"
    if not binary.exists():
        source.mkdir(parents=True, exist_ok=True)
        archive = subprocess.run(
            ["git", "archive", commit], cwd=root, check=True, capture_output=True
        ).stdout
        subprocess.run(["tar", "-x", "-C", str(source)], input=archive, check=True)
        subprocess.run(["cargo", "build", "--release", "--bin", "bench"], cwd=source, check=True)
    return binary


def build_worktree(root):
    subprocess.run(["cargo", "build", "--release", "--bin", "bench"], cwd=root, check=True)
    return root / "target" / "release" / "bench"


def bench_args(depth, threads, repetitions):
    return [
        "--depth",
        str(depth),
        "--threads",
        str(threads),
        "--repetitions",
        str(repetitions),
    ]


def parse_positions(output):
    positions = []
    for line in output.splitlines():
        match = POSITION_LINE.match(line)
        if match is None:
            continue
        positions.append(
            {
                "name": match["name"],
                "depth": int(match["depth"]),
                "nodes": int(match["nodes"]),
                "best": match["best"],
                "score": None if match["score"] is None else int(match["score"]),
            }
        )
    if not positions:
        raise SystemExit("bench output contains no position lines")
    return positions


def parse_median(output):
    for line in output.splitlines():
        match = MEDIAN_LINE.match(line)
        if match is not None:
            return {
                "depth": int(match["depth"]),
                "nodes": int(match["nodes"]),
                "elapsed": float(match["elapsed"]),
                "nps": int(match["nps"]),
            }
    raise SystemExit("bench output contains no median line")


def positions_of(binary, depth, threads, cache_file=None):
    if cache_file is not None and cache_file.exists():
        return parse_positions(cache_file.read_text())
    output = run([str(binary), *bench_args(depth, threads, 1)])
    if cache_file is not None:
        cache_file.write_text(output)
    return parse_positions(output)


def compare_positions(reference, candidate):
    mismatches = []
    if [p["name"] for p in reference] != [p["name"] for p in candidate]:
        raise SystemExit("reference and candidate bench position lists differ")
    for expected, actual in zip(reference, candidate):
        for field in ("depth", "nodes", "best", "score"):
            if expected[field] != actual[field]:
                mismatches.append(
                    {
                        "position": expected["name"],
                        "field": field,
                        "reference": expected[field],
                        "candidate": actual[field],
                    }
                )
    return mismatches


def ratio(numerator, denominator):
    return numerator / denominator


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--reference", required=True, help="局面別一致の参照コミット")
    parser.add_argument("--baseline", required=True, help="速度比の基準コミット")
    parser.add_argument("--depth", type=int, default=5)
    parser.add_argument("--repetitions", type=int, default=3)
    parser.add_argument("--threads", type=int, default=1)
    parser.add_argument("--json", type=Path, help="結果をJSONで保存するパス")
    arguments = parser.parse_args()

    root = Path(run(["git", "rev-parse", "--show-toplevel"]).strip())
    reference_commit = resolve_commit(root, arguments.reference)
    baseline_commit = resolve_commit(root, arguments.baseline)
    reference_binary = build_commit(root, reference_commit)
    baseline_binary = build_commit(root, baseline_commit)
    candidate_binary = build_worktree(root)

    reference_cache = (
        reference_binary.parents[2]
        / f"bench-depth{arguments.depth}-threads{arguments.threads}.txt"
    )
    reference_positions = positions_of(
        reference_binary, arguments.depth, arguments.threads, reference_cache
    )
    candidate_positions = positions_of(candidate_binary, arguments.depth, arguments.threads)
    mismatches = compare_positions(reference_positions, candidate_positions)

    speed = {}
    for label, binary in (
        ("baseline", baseline_binary),
        ("reference", reference_binary),
        ("candidate", candidate_binary),
    ):
        output = run([str(binary), *bench_args(arguments.depth, arguments.threads, arguments.repetitions)])
        speed[label] = parse_median(output)

    result = {
        "reference_commit": reference_commit,
        "baseline_commit": baseline_commit,
        "depth": arguments.depth,
        "threads": arguments.threads,
        "repetitions": arguments.repetitions,
        "mismatches": mismatches,
        "positions": candidate_positions,
        "speed": speed,
        "candidate_vs_baseline": {
            "nps": ratio(speed["candidate"]["nps"], speed["baseline"]["nps"]),
            "nodes": ratio(speed["candidate"]["nodes"], speed["baseline"]["nodes"]),
            "elapsed": ratio(speed["candidate"]["elapsed"], speed["baseline"]["elapsed"]),
        },
        "candidate_vs_reference": {
            "nps": ratio(speed["candidate"]["nps"], speed["reference"]["nps"]),
            "nodes": ratio(speed["candidate"]["nodes"], speed["reference"]["nodes"]),
            "elapsed": ratio(speed["candidate"]["elapsed"], speed["reference"]["elapsed"]),
        },
    }

    if mismatches:
        print(f"MISMATCH: {len(mismatches)} field(s) differ from reference {reference_commit[:7]}")
        for item in mismatches:
            print(
                f"  {item['position']} {item['field']}: reference={item['reference']} candidate={item['candidate']}"
            )
    else:
        print(
            f"MATCH: {len(candidate_positions)} positions agree with reference {reference_commit[:7]}"
            " in depth, nodes, best move, and score"
        )
    for label in ("baseline", "reference", "candidate"):
        item = speed[label]
        print(
            f"{label:9} nodes={item['nodes']} elapsed={item['elapsed']:.6f}s nps={item['nps']}"
        )
    for label, key in (("baseline", "candidate_vs_baseline"), ("reference", "candidate_vs_reference")):
        item = result[key]
        print(
            f"candidate/{label}: nps={item['nps']:.4f} nodes={item['nodes']:.4f} elapsed={item['elapsed']:.4f}"
        )
    if arguments.json is not None:
        arguments.json.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
