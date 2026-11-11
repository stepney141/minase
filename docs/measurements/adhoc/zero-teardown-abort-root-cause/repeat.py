"""Run a minase command repeatedly and count its exit codes.

usage: repeat.py --zero-dir DIR --ort-version VERSION --runs N -- COMMAND...

DIR is a tools/train/zero directory that setup.sh has prepared (it holds .runtime and .venv).
A negative exit code is the number of the signal that ended the process (-6 is SIGABRT).
"""

import argparse
import collections
import subprocess
from pathlib import Path

from probe import runtime_environment


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--zero-dir", type=Path, required=True)
    parser.add_argument("--ort-version", required=True)
    parser.add_argument("--runs", type=int, required=True)
    parser.add_argument("command", nargs="+")
    options = parser.parse_args()

    env = runtime_environment(options.zero_dir.resolve(), options.ort_version)
    counts: collections.Counter[int] = collections.Counter()
    for index in range(options.runs):
        result = subprocess.run(options.command, env=env, capture_output=True, text=True, check=False)
        counts[result.returncode] += 1
        last = result.stderr.strip().splitlines()[-1:] or [""]
        print(f"run {index}: exit={result.returncode} {last[0][:120]}", flush=True)
    print("exit codes:", dict(sorted(counts.items())))


if __name__ == "__main__":
    main()
