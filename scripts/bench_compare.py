#!/usr/bin/env python3
"""作業ツリーのbenchを照合参照コミット、基準コミット、および親コミットと比較する。

照合参照コミットとは局面別のノード数、最善手、探索値を全行で比較し、
不一致があれば終了コード1で報告する。速度は4者（基準、参照、親、候補）を
`--rounds`回の交互測定にかけ、各回は`--repetitions`回実行の中央値NPSを
とり、回ごとの中央値の中央値で比べる。候補の基準比（累積の速度比）、
参照比、および親比（段階の採否）を、ノード数比と経過時間比とともに出力し、
親比には交互測定における親の中央値の幅（最大と最小の差の中央値に対する
割合）を分解能として併記する（docs/plans/movegen-speedup-2.md「段階1」）。

コミットは`git archive`で`target/bench-cache/<完全ハッシュ>/src`へ展開し、
通常のrelease設定でビルドする。参照の局面別出力は1回実行の結果を同じ
ディレクトリへ保存して再利用する。候補は作業ツリーをビルドする。

使い方:
    scripts/bench_compare.py --reference <参照コミット> --baseline <基準コミット>
        --parent <親コミット> [--depth 5] [--repetitions 3] [--rounds 3]
        [--threads 1] [--json <出力パス>]
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
LABELS = ("baseline", "reference", "parent", "candidate")


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


def measure_rounds(binaries, depth, threads, repetitions, rounds):
    """全バイナリを交互に`rounds`回測り、ラベルごとの回別中央値を返す。"""
    samples = {label: [] for label in binaries}
    for _ in range(rounds):
        for label, binary in binaries.items():
            output = run([str(binary), *bench_args(depth, threads, repetitions)])
            samples[label].append(parse_median(output))
    return samples


def summarize(samples):
    """回別中央値の中央値と、NPSの幅（最大と最小の差の中央値に対する割合）を返す。"""
    summary = {}
    for label, rounds in samples.items():
        nps_values = [item["nps"] for item in rounds]
        median_nps = statistics.median(nps_values)
        summary[label] = {
            "depth": rounds[0]["depth"],
            "nodes": rounds[0]["nodes"],
            "elapsed": statistics.median(item["elapsed"] for item in rounds),
            "nps": median_nps,
            "nps_width": (max(nps_values) - min(nps_values)) / median_nps,
        }
    return summary


def comparison(speed, label):
    return {
        "nps": ratio(speed["candidate"]["nps"], speed[label]["nps"]),
        "nodes": ratio(speed["candidate"]["nodes"], speed[label]["nodes"]),
        "elapsed": ratio(speed["candidate"]["elapsed"], speed[label]["elapsed"]),
    }


def main():
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--reference", required=True, help="局面別一致の参照コミット")
    parser.add_argument("--baseline", required=True, help="累積の速度比の基準コミット")
    parser.add_argument("--parent", required=True, help="段階の採否を判定する直前の採用版")
    parser.add_argument("--depth", type=int, default=5)
    parser.add_argument("--repetitions", type=int, default=3, help="1回の測定内のbench反復回数")
    parser.add_argument("--rounds", type=int, default=3, help="交互測定の回数")
    parser.add_argument("--threads", type=int, default=1)
    parser.add_argument("--json", type=Path, help="結果をJSONで保存するパス")
    arguments = parser.parse_args()

    root = Path(run(["git", "rev-parse", "--show-toplevel"]).strip())
    commits = {
        "reference": resolve_commit(root, arguments.reference),
        "baseline": resolve_commit(root, arguments.baseline),
        "parent": resolve_commit(root, arguments.parent),
    }
    binaries = {
        "baseline": build_commit(root, commits["baseline"]),
        "reference": build_commit(root, commits["reference"]),
        "parent": build_commit(root, commits["parent"]),
        "candidate": build_worktree(root),
    }

    reference_cache = (
        binaries["reference"].parents[2]
        / f"bench-depth{arguments.depth}-threads{arguments.threads}.txt"
    )
    reference_positions = positions_of(
        binaries["reference"], arguments.depth, arguments.threads, reference_cache
    )
    candidate_positions = positions_of(binaries["candidate"], arguments.depth, arguments.threads)
    mismatches = compare_positions(reference_positions, candidate_positions)

    samples = measure_rounds(
        binaries, arguments.depth, arguments.threads, arguments.repetitions, arguments.rounds
    )
    speed = summarize(samples)
    parent_width = speed["parent"]["nps_width"]
    candidate_increment = comparison(speed, "parent")["nps"] - 1.0

    result = {
        "reference_commit": commits["reference"],
        "baseline_commit": commits["baseline"],
        "parent_commit": commits["parent"],
        "depth": arguments.depth,
        "threads": arguments.threads,
        "repetitions": arguments.repetitions,
        "rounds": arguments.rounds,
        "mismatches": mismatches,
        "positions": candidate_positions,
        "rounds_by_label": samples,
        "speed": speed,
        "candidate_vs_baseline": comparison(speed, "baseline"),
        "candidate_vs_reference": comparison(speed, "reference"),
        "candidate_vs_parent": comparison(speed, "parent"),
        "parent_nps_width": parent_width,
        "candidate_nps_increment_vs_parent": candidate_increment,
        "increment_exceeds_parent_width": candidate_increment > parent_width,
    }

    if mismatches:
        print(
            f"MISMATCH: {len(mismatches)} field(s) differ from reference {commits['reference'][:7]}"
        )
        for item in mismatches:
            print(
                f"  {item['position']} {item['field']}: reference={item['reference']} candidate={item['candidate']}"
            )
    else:
        print(
            f"MATCH: {len(candidate_positions)} positions agree with reference {commits['reference'][:7]}"
            " in depth, nodes, best move, and score"
        )
    for label in LABELS:
        item = speed[label]
        rounds_text = ", ".join(f"{round_item['nps']}" for round_item in samples[label])
        print(
            f"{label:9} nodes={item['nodes']} elapsed={item['elapsed']:.6f}s nps={item['nps']:.0f}"
            f" width={item['nps_width'] * 100:+.2f}% rounds=[{rounds_text}]"
        )
    for label in ("baseline", "reference", "parent"):
        item = result[f"candidate_vs_{label}"]
        print(
            f"candidate/{label}: nps={item['nps']:.4f} nodes={item['nodes']:.4f} elapsed={item['elapsed']:.4f}"
        )
    verdict = "exceeds" if result["increment_exceeds_parent_width"] else "does not exceed"
    print(
        f"candidate nps increment vs parent {candidate_increment * 100:+.2f}%"
        f" {verdict} parent width {parent_width * 100:.2f}%"
    )
    if arguments.json is not None:
        arguments.json.write_text(json.dumps(result, indent=2, ensure_ascii=False) + "\n")
    return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
