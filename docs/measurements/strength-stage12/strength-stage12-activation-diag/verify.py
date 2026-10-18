#!/usr/bin/env python3
"""基準と計測の有効/無効を局面ごとに照合し、診断計数の包含関係を検査する。"""

import argparse
import json
import os
from pathlib import Path
import subprocess


def check_counts(counts):
    assert all(isinstance(value, int) and value >= 0 for value in counts.values())
    assert counts["asp_failhigh_iterations"] == sum(
        counts[f"asp_failhigh_hist_{n}"] for n in [1, 2, 3, "4plus"]
    )
    assert counts["asp_failhigh_iterations"] <= counts["asp_iterations"]
    assert counts["asp_iterations_aborted"] <= counts["asp_iterations"]
    for prefix, denominator in [("bad_capture", "capture_stage_nodes"),
                                ("capture_history", "capture_history_nodes")]:
        assert counts[f"{prefix}_reached"] <= counts[f"{prefix}_order_changed"] <= counts[denominator]
    assert counts["capture_history_nonzero_refs"] <= counts["capture_history_refs"]
    assert counts["capture_history_nodes"] <= counts["capture_stage_nodes"]
    assert counts["q_good"] <= counts["q_searched"]
    assert counts["q_nodes"] <= counts["q_searched"]
    previous = None
    for limit in [1, 2, 3, 4, 6]:
        lost = counts[f"q_lost_{limit}"]
        nodes = counts[f"q_prune_nodes_{limit}"]
        moves = counts[f"q_pruned_moves_{limit}"]
        assert lost <= counts["q_good"]
        assert nodes <= counts["q_nodes"]
        assert max(lost, nodes) <= moves <= counts["q_searched"]
        if previous is not None:
            assert all(current <= earlier for current, earlier in zip((lost, nodes, moves), previous))
        previous = (lost, nodes, moves)
    assert max(counts["null_skip_pv"], counts["null_skip_eval"]) <= counts["null_skip_any"] <= counts["null_tried"]
    assert counts["null_skip_any"] <= counts["null_skip_pv"] + counts["null_skip_eval"]
    for kind in ["negamax", "quiesce"]:
        assert counts[f"pv_tt_cut_{kind}"] <= counts[f"pv_nodes_{kind}"]


def rows(text):
    return [dict(token.split("=", 1) for token in line.split() if not token.startswith("elapsed="))
            for line in text.splitlines() if line.startswith("position=")]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    args.out_dir.mkdir(parents=True, exist_ok=True)
    environment = {key: value for key, value in os.environ.items() if key != "MINASE_STAGE12_DIAG"}
    outputs = {}
    for name, binary in [("baseline", args.baseline), ("disabled", args.binary), ("enabled", args.binary)]:
        env = dict(environment)
        if name == "enabled":
            env["MINASE_STAGE12_DIAG"] = str((args.out_dir / "bench-counters.json").resolve())
        result = subprocess.run([str(binary.resolve()), "--depth", "6", "--threads", "1", "--repetitions", "1"],
                                env=env, text=True, capture_output=True, check=True, timeout=60)
        (args.out_dir / f"bench-{name}.txt").write_text(result.stdout)
        outputs[name] = rows(result.stdout)
    assert len(outputs["baseline"]) == 15
    assert outputs["baseline"] == outputs["disabled"] == outputs["enabled"]
    total = sum(int(row["nodes"]) for row in outputs["baseline"])
    assert total == 1677944, total
    check_counts(json.loads((args.out_dir / "bench-counters.json").read_text()))
    report = {"baseline": str(args.baseline.resolve()), "binary": str(args.binary.resolve()),
              "positions": 15, "total_nodes": total, "matching_fields": list(outputs["baseline"][0]),
              "all_equal": True, "counter_invariants": "passed", "rows": outputs}
    (args.out_dir / "passivity.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"15 positions: baseline == disabled == enabled; nodes={total}; counter invariants passed")


if __name__ == "__main__":
    main()
