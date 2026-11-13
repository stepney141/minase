"""Attribute attackers_to_by samples to their immediate recorded caller."""

import argparse
import collections
import json
from pathlib import Path
import re
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    segments = []
    for line in subprocess.check_output(["readelf", "-lW", str(binary)], text=True).splitlines():
        fields = line.split()
        if fields and fields[0] == "LOAD":
            segments.append(tuple(int(fields[i], 16) for i in (1, 2, 4)))

    header = re.compile(r"\s*(\d+)\s+(\S+):\s*")
    frame = re.compile(r"\s*([0-9a-f]+)\s+(.*?)\s+\((.*)\)")
    total = collections.Counter(samples=0, period=0)
    callers = collections.defaultdict(lambda: collections.Counter(samples=0, period=0))
    unresolved = collections.Counter(samples=0, period=0)

    def record(active, stack):
        if active is None or "cycles" not in active[1]:
            return
        period = active[0]
        total.update(samples=1, period=period)
        if not stack or "position::Position>::attackers_to_by+" not in stack[0][1]:
            return
        if len(stack) < 2:
            unresolved.update(samples=1, period=period)
            return
        mapped = re.fullmatch(r"(.*)\+0x([0-9a-f]+)", stack[1][2])
        if mapped is None or Path(mapped[1]).resolve() != binary:
            unresolved.update(samples=1, period=period)
            return
        offset = int(mapped[2], 16)
        matches = [(start, virtual, size) for start, virtual, size in segments if start <= offset < start + size]
        if len(matches) != 1:
            raise ValueError(f"No unique ELF segment for caller offset {offset:x}")
        start, virtual, _ = matches[0]
        callers[hex(offset - start + virtual)].update(samples=1, period=period)

    command = ["perf", "script", "--no-inline", "-i", str(args.data), "-F", "event,period,ip,sym,symoff,dso,dsoff"]
    with subprocess.Popen(command, stdout=subprocess.PIPE, text=True) as process:
        active = None
        stack = []
        for line in process.stdout:
            match = header.fullmatch(line.rstrip())
            if match is not None:
                record(active, stack)
                active = (int(match[1]), match[2])
                stack = []
            elif line.strip():
                match = frame.fullmatch(line.rstrip())
                if match is None:
                    raise ValueError(f"Unexpected perf frame: {line!r}")
                if len(stack) < 2:
                    stack.append(match.groups())
        record(active, stack)
        if process.wait() != 0:
            raise RuntimeError("perf script failed")
    if not callers:
        raise ValueError("No attackers_to_by samples with resolved callers")

    addresses = sorted(callers)
    # The byte before the return address belongs to the call instruction.
    resolved = subprocess.check_output(
        ["addr2line", "-a", "-f", "-C", "-i", "-e", str(binary)],
        input="\n".join(hex(int(address, 16) - 1) for address in addresses) + "\n", text=True,
    )
    frames = {}
    for line in resolved.splitlines():
        if re.fullmatch(r"0x[0-9a-f]+", line):
            current = hex(int(line, 16))
            frames[current] = []
        else:
            frames[current].append(line)
    rows = []
    for address, counter in sorted(callers.items(), key=lambda item: item[1]["period"], reverse=True):
        call_address = hex(int(address, 16) - 1)
        lines = frames[call_address]
        if len(lines) % 2:
            raise ValueError(f"Unpaired function/location at {address}")
        rows.append({
            "return_address": address,
            "call_address": call_address,
            **counter,
            "period_percent": 100 * counter["period"] / total["period"],
            "frames": [{"function": lines[i], "location": lines[i + 1]} for i in range(0, len(lines), 2)],
        })
    result = {"binary": str(binary), "data": str(args.data), "total_cycles": total, "unresolved": unresolved, "callers": rows}
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"total_cycles": total, "unresolved": unresolved, "caller_samples": sum(row["samples"] for row in rows)}, indent=2))


if __name__ == "__main__":
    main()
