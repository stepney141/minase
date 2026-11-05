"""Interleave single-worker teacher-generation measurements on the same CPU."""

import argparse
import hashlib
import json
from pathlib import Path
import statistics
import subprocess


EVENTS = [
    "cpu_core/cycles/u", "cpu_core/instructions/u",
    "cpu_core/mem_load_retired.l1_miss/u", "cpu_core/mem_load_retired.l2_miss/u",
    "cpu_core/mem_load_retired.l3_miss/u", "cpu_core/branch-misses/u",
]
TIMING_FIELDS = {
    "elapsed_seconds", "searched_positions_per_second", "recorded_positions_per_second",
}


def read_summary(path):
    return dict(line.split(": ", 1) for line in path.read_text().splitlines() if ": " in line)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--versions", nargs="+", required=True)
    parser.add_argument("--rounds", type=int, required=True)
    args = parser.parse_args()
    root = args.directory.resolve()
    results = root / "results"
    samples = []
    expected = None
    for round_index in range(args.rounds):
        # Reverse the order each round to reduce systematic time-order bias.
        order = args.versions if round_index % 2 == 0 else list(reversed(args.versions))
        for version in order:
            stem = f"{args.tag}-{version}-{round_index + 1}"
            paths = {ext: results / f"{stem}.{ext}" for ext in ["txt", "stderr", "stat", "mnsd"]}
            if any(path.exists() for path in paths.values()):
                raise FileExistsError(stem)
            binary = root / "bin" / version
            command = [
                "taskset", "-c", "3", "perf", "stat", "-x,", "-o", str(paths["stat"]),
                "-e", ",".join(EVENTS), "--", str(binary), "data", "selfplay", "generate",
                "--output", str(paths["mnsd"]), "--seed", "9010200000", "--games", "2",
                "--concurrency", "1", "--nodes", "20000", "--random-moves", "0",
                "--max-ply", "200", "--hash-mb", "16", "--allow-dirty",
            ]
            with paths["txt"].open("w") as out, paths["stderr"].open("w") as err:
                subprocess.run(command, cwd=root / version, stdout=out, stderr=err, check=True)
            summary = read_summary(paths["txt"])
            deterministic = {key: value for key, value in summary.items() if key not in TIMING_FIELDS}
            if expected is None:
                expected = deterministic
            elif deterministic != expected:
                raise ValueError(f"Generation summaries differ for {stem}")
            counters = {}
            scheduled_percent = {}
            for line in paths["stat"].read_text().splitlines():
                fields = line.split(",")
                if len(fields) >= 5 and fields[2] in EVENTS:
                    counters[fields[2]] = int(fields[0])
                    scheduled_percent[fields[2]] = float(fields[4])
            if set(counters) != set(EVENTS):
                raise ValueError(f"Missing counters for {stem}: {counters}")
            nodes = int(summary["searched_nodes_total"])
            elapsed = float(summary["elapsed_seconds"])
            sample = {
                "version": version, "round": round_index + 1, "nodes": nodes,
                "elapsed_seconds": elapsed, "nps": nodes / elapsed,
                "counters_per_node": {key: value / nodes for key, value in counters.items()},
                "counter_scheduled_percent": scheduled_percent,
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "command": command, "cwd": str(root / version),
            }
            samples.append(sample)
            (results / f"{args.tag}-samples.json").write_text(json.dumps(samples, indent=2) + "\n")
            print(f"round={round_index + 1} version={version} seconds={elapsed:.6f} nps={sample['nps']:.0f}", flush=True)
    aggregates = {}
    for version in args.versions:
        rows = [row for row in samples if row["version"] == version]
        speeds = [row["nps"] for row in rows]
        aggregates[version] = {
            "nps_median": statistics.median(speeds), "nps_min": min(speeds), "nps_max": max(speeds),
            "nps_width": (max(speeds) - min(speeds)) / statistics.median(speeds),
            "counters_per_node_median": {
                event: statistics.median(row["counters_per_node"][event] for row in rows)
                for event in EVENTS
            },
        }
    baseline = aggregates[args.versions[0]]["nps_median"]
    for aggregate in aggregates.values():
        aggregate["nps_ratio_to_first"] = aggregate["nps_median"] / baseline
    (results / f"{args.tag}-summary.json").write_text(json.dumps(aggregates, indent=2) + "\n")
    print(json.dumps(aggregates, indent=2), flush=True)


if __name__ == "__main__":
    main()
