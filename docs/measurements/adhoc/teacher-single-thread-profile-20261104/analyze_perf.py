"""Attribute perf samples through ELF offsets and DWARF inline frames."""

import argparse
import collections
import json
from pathlib import Path
import re
import subprocess


def classify(frames, symbol):
    functions = "\n".join(frame["function"] for frame in frames)
    locations = "\n".join(frame["location"] for frame in frames)
    trace = functions + "\n" + symbol
    if "::nnue::head::" in trace and "output" in trace:
        return "nnue_head"
    if "refresh_accumulator" in trace or "recompute_accumulator" in trace:
        return "nnue_refresh"
    if any(name in trace for name in ("update_accumulator_after_move", "update_accumulator_after_null", "::add_hidden", "::add_feature")):
        return "nnue_update"
    if "ContinuationCorrectionHistory" in trace and "::new" in trace:
        return "history_initialization"
    if "ContinuationCorrectionHistory" in trace:
        return "continuation_correction_values"
    if "continuation_correction_index" in trace or "continuation_correction_indices" in trace:
        return "continuation_correction_index"
    if "ContinuationHistory" in trace and any(name in trace for name in ("::read", "::update")):
        return "continuation_values"
    if "continuation_index" in trace:
        return "continuation_index"
    if "continuation_value" in trace or "update_continuation" in trace:
        return "continuation_other"
    if "HistoryTables" in trace and any(name in trace for name in ("::new", "::clear")):
        return "history_initialization"
    if "PawnHistory" in trace:
        return "pawn_history"
    if "CorrectionValues" in trace:
        return "other_correction_values"
    if "correction_weighted_sum_at" in trace or "apply_correction_bonus" in trace or "update_correction" in trace:
        return "correction_other"
    if "toggle_piece_square_keys" in trace or "piece_keys_after_move" in trace:
        return "correction_keys"
    if "TranspositionTable" in trace:
        return "transposition_table"
    if "capture_history" in trace:
        return "capture_history"
    if "new_searcher" in trace or "run_search_team" in trace:
        return "search_setup"
    if "::eval::nnue::" in trace:
        return "nnue_other"
    if "::movegen::" in trace or "::attacks::" in trace or "::attackers_to_by" in trace:
        return "movegen_attacks"
    if "::see::" in trace or "::see_prunes" in trace:
        return "see"
    if "ordering.rs:" in locations or "::ordering::" in trace:
        return "ordering_other"
    if "::sort::" in trace:
        return "sorting"
    if "::position::" in trace:
        return "position"
    if "mi_" in trace:
        return "allocator"
    if "::search::" in trace:
        return "search_other"
    return "unclassified"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    headers = subprocess.check_output(["readelf", "-lW", str(binary)], text=True)
    segments = []
    for line in headers.splitlines():
        fields = line.split()
        if fields and fields[0] == "LOAD":
            segments.append(tuple(int(fields[i], 16) for i in (1, 2, 4)))
    raw = subprocess.check_output(["perf", "script", "-i", str(args.data), "-F", "event,period,ip,sym,symoff,dso,dsoff"], text=True)
    pattern = re.compile(r"\s*(\d+)\s+(\S+):\s+([0-9a-f]+)\s+(.*?)\s+\((.*)\)$")
    samples = []
    addresses = set()
    for line in raw.splitlines():
        if not line.strip():
            continue
        match = pattern.fullmatch(line)
        if match is None:
            raise ValueError(f"Unexpected perf sample: {line!r}")
        period, event, ip, symbol, dso_field = match.groups()
        mapped = re.fullmatch(r"(.*)\+0x([0-9a-f]+)", dso_field)
        if mapped is not None:
            dso, offset = mapped.groups()
        elif dso_field == "[unknown]":
            dso, offset = dso_field, None
        else:
            raise ValueError(f"Unexpected DSO field: {dso_field!r}")
        address = None
        if Path(dso).resolve() == binary:
            offset = int(offset, 16)
            matches = [(start, virtual, size) for start, virtual, size in segments if start <= offset < start + size]
            if len(matches) != 1:
                raise ValueError(f"No unique ELF segment for file offset {offset:x}")
            start, virtual, size = matches[0]
            address = hex(offset - start + virtual)
            addresses.add(address)
        samples.append((event, int(period), symbol, address, dso))
    if not addresses:
        raise ValueError("No samples resolved to the requested binary")
    resolved = subprocess.check_output(["addr2line", "-a", "-C", "-f", "-i", "-e", str(binary)], input="\n".join(sorted(addresses)) + "\n", text=True)
    lines_by_address = {}
    current = None
    for line in resolved.splitlines():
        if re.fullmatch(r"0x[0-9a-f]+", line):
            current = hex(int(line, 16))
            lines_by_address[current] = []
        elif current is not None:
            lines_by_address[current].append(line)
    frames = {}
    for address, lines in lines_by_address.items():
        if len(lines) % 2:
            raise ValueError(f"Unpaired DWARF function/location at {address}")
        frames[address] = [{"function": lines[i], "location": lines[i + 1]} for i in range(0, len(lines), 2)]
    totals = collections.defaultdict(lambda: collections.Counter(samples=0, period=0))
    categories = collections.defaultdict(lambda: collections.defaultdict(lambda: collections.Counter(samples=0, period=0)))
    symbols = collections.defaultdict(collections.Counter)
    details = collections.defaultdict(lambda: collections.defaultdict(lambda: collections.Counter(samples=0, period=0)))
    for event, period, symbol, address, dso in samples:
        trace = frames[address] if address is not None else []
        category = classify(trace, symbol)
        for counter in (totals[event], categories[event][category]):
            counter["samples"] += 1
            counter["period"] += period
        symbols[event][re.sub(r"\+0x[0-9a-f]+$", "", symbol)] += period
        if address is not None:
            details[address][event]["samples"] += 1
            details[address][event]["period"] += period
    events = {}
    for event, total in totals.items():
        rows = []
        for category, counter in categories[event].items():
            rows.append({"category": category, **counter, "period_percent": 100 * counter["period"] / total["period"]})
        rows.sort(key=lambda row: row["period"], reverse=True)
        assert sum(row["period"] for row in rows) == total["period"]
        events[event] = {"total": total, "categories": rows, "top_symbols": [{"symbol": key, "period": count, "period_percent": 100 * count / total["period"]} for key, count in symbols[event].most_common(50)]}
    args.output.write_text(json.dumps({"binary": str(binary), "data": str(args.data), "events": events}, indent=2) + "\n")
    args.output.with_suffix(".addresses.json").write_text(json.dumps({address: {"frames": frames[address], "events": value} for address, value in details.items()}, indent=2) + "\n")
    for event, result in events.items():
        print(event, result["total"])
        for row in result["categories"]:
            print(f'{row["period_percent"]:7.3f}% {row["category"]}')


if __name__ == "__main__":
    main()
