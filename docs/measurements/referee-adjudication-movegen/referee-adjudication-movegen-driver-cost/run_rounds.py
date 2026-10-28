"""比較元と候補のdriver_costを同じCPUで交互に実行し、出力と実行環境を保存する。

使い方: python3 -I run_rounds.py --before <比較元の実行ファイル> --after <候補の実行ファイル>
        --out <出力先> [--rounds 5] [--cpu 2] [--seed 20261028] [--games 32]

各回の出力は<出力先>/before-<回>.jsonと<出力先>/after-<回>.jsonに、
各実行の開始・終了時刻、開始時の負荷平均、全CPUと固定したCPUの稼働率は
<出力先>/environment.jsonに保存する。
"""

import argparse
import datetime
import json
import os
import pathlib
import subprocess


def cpu_times():
    times = {}
    with open("/proc/stat", encoding="ascii") as file:
        for line in file:
            if line.startswith("cpu"):
                name, *values = line.split()
                times[name] = [int(value) for value in values]
    return times


def utilization(start, end, name):
    total = sum(end[name]) - sum(start[name])
    # idleとiowaitを非稼働として数える。
    idle = (end[name][3] + end[name][4]) - (start[name][3] + start[name][4])
    return 100.0 * (total - idle) / total if total else None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--before", required=True)
    parser.add_argument("--after", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--rounds", type=int, default=5)
    parser.add_argument("--cpu", type=int, default=2)
    parser.add_argument("--seed", default="20261028")
    parser.add_argument("--games", default="32")
    arguments = parser.parse_args()

    out = pathlib.Path(arguments.out)
    out.mkdir(parents=True, exist_ok=False)
    runs = []
    for round_index in range(1, arguments.rounds + 1):
        for label, binary in (("before", arguments.before), ("after", arguments.after)):
            path = out / f"{label}-{round_index}.json"
            load = os.getloadavg()
            start_times = cpu_times()
            started = datetime.datetime.now().astimezone()
            with open(path, "w", encoding="utf-8") as output:
                subprocess.run(
                    [
                        "nice", "-n", "19", "taskset", "-c", str(arguments.cpu), binary,
                        "--seed", arguments.seed, "--games", arguments.games,
                        "--adjudication", "after",
                    ],
                    stdout=output,
                    check=True,
                )
            finished = datetime.datetime.now().astimezone()
            end_times = cpu_times()
            runs.append(
                {
                    "label": label,
                    "round": round_index,
                    "output": path.name,
                    "started": started.isoformat(timespec="seconds"),
                    "finished": finished.isoformat(timespec="seconds"),
                    "loadavg_at_start": load,
                    "cpu_all_percent": utilization(start_times, end_times, "cpu"),
                    f"cpu{arguments.cpu}_percent": utilization(
                        start_times, end_times, f"cpu{arguments.cpu}"
                    ),
                }
            )
    with open(out / "environment.json", "w", encoding="utf-8") as file:
        json.dump(
            {"cpu": arguments.cpu, "seed": arguments.seed, "games": arguments.games, "runs": runs},
            file,
            indent=2,
        )


if __name__ == "__main__":
    main()
