"""Summarize the archived teacher-generation observations without rerunning them."""

import collections
import json
from pathlib import Path
import re
import statistics


ROOT = Path(__file__).resolve().parent
cases = {}
for path in sorted((ROOT / "runs").glob("*.summary.json")):
    name = path.name.removesuffix(".summary.json")
    summary = json.loads(path.read_text())
    cases[name] = summary
    process_path = ROOT / "runs" / (name + ".process.json")
    if process_path.exists():
        process = json.loads(process_path.read_text())
        rows = process["rows"]
        seconds_by_threads = collections.Counter()
        for before, after in zip(rows, rows[1:]):
            seconds_by_threads[before["threads"]] += (
                after["elapsed_seconds"] - before["elapsed_seconds"]
            )
        cpu = (
            rows[-1]["user_ticks"] + rows[-1]["system_ticks"]
            - rows[0]["user_ticks"] - rows[0]["system_ticks"]
        ) / process["clock_ticks"]
        elapsed = rows[-1]["elapsed_seconds"] - rows[0]["elapsed_seconds"]
        summary["process_samples"] = {
            "sampled_cpu_seconds": cpu,
            "sampled_elapsed_seconds": elapsed,
            "requested_cpu_utilization_percent": (
                100 * cpu / elapsed / int(summary["concurrency"])
            ),
            "seconds_by_thread_count": dict(seconds_by_threads),
            "peak_rss_mib": max(row["rss_pages"] for row in rows)
            * process["page_size"] / 1024**2,
        }

    stderr = (ROOT / "runs" / (name + ".stderr")).read_text()
    games = []
    for line in stderr.splitlines():
        if line.startswith("[TEACHER_PROFILE_20261113_GAME]"):
            games.append({key: int(value) for key, value in re.findall(r"(\w+)=(\d+)", line)})
    if games:
        total = sum(game["total_ns"] for game in games)
        stages = {
            "games": games,
            "summed_game_seconds": total / 1e9,
            "percent_of_summed_game_wall_time": {
                key: 100 * sum(game[key] for game in games) / total
                for key in ("setup_ns", "snapshot_ns", "search_ns")
            },
        }
        for label in ("MERGE", "FINISH"):
            matches = [line for line in stderr.splitlines()
                       if line.startswith("[TEACHER_PROFILE_20261113_" + label + "]")]
            assert len(matches) == 1
            stages[label.lower()] = {
                key: int(value) for key, value in re.findall(r"(\w+)=(\d+)", matches[0])
            }
        summary["stages"] = stages

topdown = {}
for name in ("topdown-c1", "topdown-c4"):
    values = {}
    for line in (ROOT / "runs" / (name + ".stat")).read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        fields = line.split(",")
        assert fields[4] == "100.00", fields
        values[fields[2]] = int(fields[0])
    slots = values["cpu_core/slots/u"]
    percentages = {event: 100 * value / slots for event, value in values.items()
                   if "/topdown-" in event}
    percentages["core_bound_derived"] = (
        percentages["cpu_core/topdown-be-bound/u"]
        - percentages["cpu_core/topdown-mem-bound/u"]
    )
    topdown[name] = {"counters": values, "slot_percent": percentages,
                     "cycles_per_node": values["cpu_core/cycles/u"]
                     / int(cases[name]["searched_nodes_total"])}

profile = json.loads((ROOT / "validated-samples-summary.json").read_text())
cycles = profile["events"]["cpu_core/cycles,period=4000037/u"]
categories = {row["category"]: row["period_percent"] for row in cycles["categories"]}
groups = {
    "movegen_attacks_bitboard": ["movegen_attacks", "bitboard"],
    "ordering_and_history": ["ordering_other", "sorting", "capture_history",
                             "continuation_values", "continuation_index",
                             "continuation_other", "pawn_history"],
    "nnue": ["nnue_other", "nnue_head", "nnue_update", "nnue_refresh", "nnue_search_adapter"],
    "transposition_table": ["transposition_table"],
    "correction_history": ["correction_keys", "correction_other", "other_correction_values",
                           "continuation_correction_index", "continuation_correction_values"],
    "search_other": ["search_other"],
    "position": ["position"],
    "see": ["see"],
    "other": ["unclassified", "allocator", "search_setup", "history_initialization", "teacher_management"],
}
members = [category for group in groups.values() for category in group]
assert len(set(members)) == len(members)
assert set(members) == set(categories)
group_percent = {group: sum(categories[category] for category in members)
                 for group, members in groups.items()}
assert abs(sum(group_percent.values()) - 100) < 1e-8

control_times = [float(cases[name]["elapsed_seconds"])
                 for name in ("pilot", "baseline-2", "baseline-3")]
result = {"cases": cases, "topdown": topdown, "cycle_group_percent": group_percent,
          "baseline_2game_elapsed_median": statistics.median(control_times),
          "baseline_2game_elapsed_range": [min(control_times), max(control_times)]}
(ROOT / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps({"cycle_group_percent": group_percent,
                  "topdown": topdown}, indent=2))
