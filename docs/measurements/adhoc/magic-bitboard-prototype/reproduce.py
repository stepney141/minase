#!/usr/bin/env python3
"""Rebuild the isolated prototypes and repeat the three search comparisons."""
import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

BASE = "1633f534cf0e8c96738b11158e68f31f0f144f01"
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--work-dir", type=Path, required=True)
parser.add_argument("--cpu", type=int, required=True)
args = parser.parse_args()
artifacts = Path(__file__).resolve().parent
repository = artifacts.parents[2]
work = args.work_dir.resolve()
work.mkdir(parents=True, exist_ok=False)
(work / "bin").mkdir()
archive = subprocess.run(
    ["git", "archive", BASE], cwd=repository, capture_output=True, check=True
).stdout
variants = {
    "baseline": [],
    "horizontal": ["horizontal.patch"],
    "diagonal": ["diagonal.patch"],
    "baseline_force": ["force-inline.patch"],
    "horizontal_force": ["horizontal.patch", "force-inline.patch"],
    "diagonal_force": ["diagonal.patch", "force-inline.patch"],
}
env = os.environ.copy()
env["CARGO_TARGET_DIR"] = str(work / "target")
env["RUSTFLAGS"] = "-C target-cpu=native"
for name, patches in variants.items():
    source = work / name
    source.mkdir()
    subprocess.run(["tar", "-x", "-C", str(source)], input=archive, check=True)
    for patch in patches:
        subprocess.run(["git", "apply", str(artifacts / patch)], cwd=source, check=True)
    subprocess.run(
        ["cargo", "build", "--release", "--locked", "--bin", "bench"],
        cwd=source, env=env, check=True,
    )
    shutil.copy2(work / "target/release/bench", work / "bin" / name)

for label, depth, rounds, names in [
    ("depth5-initial", 5, 6, ["baseline", "horizontal", "diagonal"]),
    ("depth5-forced", 5, 8, ["baseline", "baseline_force", "horizontal_force", "diagonal_force"]),
    ("depth6-forced", 6, 6, ["baseline", "baseline_force", "diagonal_force"]),
]:
    command = [
        sys.executable, str(artifacts / "compare.py"),
        "--repository", str(work / "baseline"), "--depth", str(depth),
        "--rounds", str(rounds), "--repetitions", "5", "--cpu", str(args.cpu),
        "--output", str(work / label),
    ]
    for name in names:
        command += ["--binary", f"{name}={work / 'bin' / name}"]
    subprocess.run(command, check=True)
