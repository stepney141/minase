#!/usr/bin/env python3
"""PROTOTYPE: compare separate binaries, with no runtime feature switches."""
import argparse
import hashlib
import importlib.util
import json
import statistics
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--repository", type=Path, required=True)
parser.add_argument("--binary", action="append", required=True, help="name=/absolute/path")
parser.add_argument("--depth", type=int, required=True)
parser.add_argument("--rounds", type=int, required=True)
parser.add_argument("--repetitions", type=int, required=True)
parser.add_argument("--cpu", type=int, required=True)
parser.add_argument("--output", type=Path, required=True)
args = parser.parse_args()
spec = importlib.util.spec_from_file_location("existing_bench", args.repository / "scripts/bench_compare.py")
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)
binaries = dict(item.split("=", 1) for item in args.binary)
assert len(binaries) == len(args.binary)
assert next(iter(binaries)) == "baseline"
args.output.mkdir(parents=True, exist_ok=False)

def run(binary, repetitions):
    return subprocess.run([
        "taskset", "-c", str(args.cpu), binary,
        "--depth", str(args.depth), "--threads", "1", "--repetitions", str(repetitions),
    ], check=True, capture_output=True, text=True).stdout

positions = {}
for name, binary in binaries.items():
    output = run(binary, 1)
    (args.output / f"{name}-positions.txt").write_text(output)
    positions[name] = bench.parse_positions(output)
    assert len(positions[name]) == 15
    assert not bench.compare_positions(positions["baseline"], positions[name]), name
    print(f"positions_equal={name}", flush=True)

samples = {name: [] for name in binaries}
orders = []
for round_index in range(args.rounds):
    names = list(binaries)
    rotation = round_index % len(names)
    names = names[rotation:] + names[:rotation]
    if round_index % 2:
        names.reverse()
    orders.append(names)
    for name in names:
        output = run(binaries[name], args.repetitions)
        (args.output / f"round-{round_index + 1}-{name}.txt").write_text(output)
        value = bench.parse_median(output)
        assert value["nodes"] == sum(p["nodes"] for p in positions["baseline"])
        samples[name].append(value)
        print(f"round={round_index + 1} variant={name} nps={value['nps']}", flush=True)

summary = bench.summarize(samples)
ratios = {}
for name in binaries:
    paired = [row["nps"] / base["nps"] for base, row in zip(samples["baseline"], samples[name])]
    ratios[name] = {
        "median_nps_ratio": summary[name]["nps"] / summary["baseline"]["nps"],
        "paired_median_ratio": statistics.median(paired),
        "paired_min_ratio": min(paired),
        "paired_max_ratio": max(paired),
    }
result = {
    "depth": args.depth, "rounds": args.rounds, "repetitions": args.repetitions,
    "cpu": args.cpu, "orders": orders, "binaries": binaries,
    "sha256": {name: hashlib.sha256(Path(path).read_bytes()).hexdigest() for name, path in binaries.items()},
    "positions_equal": True, "positions": positions["baseline"],
    "samples": samples, "summary": summary, "ratios": ratios,
}
(args.output / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps({"summary": summary, "ratios": ratios}, indent=2), flush=True)
