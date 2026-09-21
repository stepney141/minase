#!/usr/bin/env python3
"""Summarize physical and inline perf stacks from the same recording.

Input files must be produced with `perf script -F period,ip,sym,dso`,
first with --no-inline and second with --inline. The measured worker is
selected by its physical call stack; table clearing and startup are excluded.
Each sample is counted once, weighted by its sampling period.
"""
import json
import sys
from collections import Counter
from pathlib import Path

physical = Path(sys.argv[1]).read_text().strip().split("\n\n")
inlined = Path(sys.argv[2]).read_text().strip().split("\n\n")
assert len(physical) == len(inlined)
periods = Counter()
samples = Counter()
sliding_periods = Counter()
sliding_samples = Counter()
total_period = 0
total_samples = 0

for block, inline_block in zip(physical, inlined):
    lines = block.splitlines()
    period = int(lines[0])
    assert period == int(inline_block.splitlines()[0])
    assert lines[1].split()[0] == inline_block.splitlines()[1].split()[0]
    if "minase::search::run_search_team::{closure#3}" not in block:
        continue
    total_period += period
    total_samples += 1
    symbol = " ".join(lines[1].split()[1:]).rsplit(" (", 1)[0]
    periods[symbol] += period
    samples[symbol] += 1
    if any(
        line.split()[1:] == ["sliding_control", "(inlined)"]
        for line in inline_block.splitlines()[1:]
    ):
        sliding_periods[symbol] += period
        sliding_samples[symbol] += 1

assert total_samples > 0
assert sum(sliding_samples.values()) > 0
print(json.dumps({
    "all_samples": len(physical),
    "search_samples": total_samples,
    "search_period_sum": total_period,
    "sliding_samples": sum(sliding_samples.values()),
    "sliding_period_sum": sum(sliding_periods.values()),
    "sliding_percent": 100 * sum(sliding_periods.values()) / total_period,
    "functions": [
        {
            "symbol": symbol,
            "samples": samples[symbol],
            "percent": 100 * period / total_period,
            "sliding_samples": sliding_samples[symbol],
            "sliding_percent": 100 * sliding_periods[symbol] / total_period,
        }
        for symbol, period in periods.most_common()
    ],
}, indent=2))
