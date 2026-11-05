"""Summarize inclusive user call chains; inspect coverage before interpreting them."""

import argparse
import collections
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    raw = subprocess.check_output([
        "perf", "script", "-i", str(args.data), "-F",
        "period,ip,sym,symoff,dso", "--max-stack", "64",
    ], text=True)
    terms = [
        "::gives_check", "checking_move_exempts_qsearch_pruning",
        "attackers_to_by", "piece_control_without_special",
        "sliding_control", "ordinary_capturer", "::quiesce",
        "::negamax", "::see::", "::head::",
    ]
    periods = collections.Counter()
    counts = collections.Counter()
    total = 0
    empty = 0
    samples = 0
    for block in raw.strip().split("\n\n"):
        lines = block.splitlines()
        period = int(lines[0])
        trace = "\n".join(lines[1:])
        total += period
        samples += 1
        if len(lines) == 1:
            empty += 1
        for term in terms:
            if term in trace:
                periods[term] += period
                counts[term] += 1
    if total <= 0:
        raise ValueError("No sampled cycles")
    result = {
        "data": str(args.data),
        "samples": samples,
        "period": total,
        "empty_stack_samples": empty,
        "inclusive": [
            {"term": term, "samples": counts[term], "period": value,
             "period_percent": 100 * value / total}
            for term, value in periods.most_common()
        ],
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
