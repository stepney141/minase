"""Check fixed-position search results and complete teacher games against the baseline."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--versions", nargs="+", required=True)
    args = parser.parse_args()
    root = args.directory.resolve()
    results = root / "results"
    baseline_positions = [
        line.split(" elapsed=")[0]
        for line in (results / "base-bench9.txt").read_text().splitlines()
        if line.startswith("position=")
    ]
    assert len(baseline_positions) == 15
    expected_hash = hashlib.sha256((results / "base-pilot.mnsd").read_bytes()).hexdigest()
    expected_nodes = int(re.search(r"^searched_nodes_total: (\d+)$", (results / "base-pilot.txt").read_text(), re.M)[1])
    records = []
    for version in args.versions:
        binary = root / "bin" / version
        bench_output = subprocess.check_output([
            "taskset", "-c", "3", str(binary), "dev", "bench",
            "--depth", "9", "--threads", "1", "--repetitions", "1",
        ], cwd=root / version, text=True)
        (results / f"{version}-verify-bench9.txt").write_text(bench_output)
        positions = [line.split(" elapsed=")[0] for line in bench_output.splitlines() if line.startswith("position=")]
        assert positions == baseline_positions, version
        output_path = results / f"{version}-verify.mnsd"
        if output_path.exists():
            raise FileExistsError(output_path)
        command = [
            "taskset", "-c", "3", str(binary), "data", "selfplay", "generate",
            "--output", str(output_path), "--seed", "9010200000", "--games", "2",
            "--concurrency", "1", "--nodes", "20000", "--random-moves", "0",
            "--max-ply", "4000", "--hash-mb", "16", "--allow-dirty",
        ]
        with (results / f"{version}-verify.txt").open("w") as out, (results / f"{version}-verify.stderr").open("w") as err:
            subprocess.run(command, cwd=root / version, stdout=out, stderr=err, check=True)
        digest = hashlib.sha256(output_path.read_bytes()).hexdigest()
        assert digest == expected_hash, (version, digest, expected_hash)
        summary = (results / f"{version}-verify.txt").read_text()
        nodes = int(re.search(r"^searched_nodes_total: (\d+)$", summary, re.M)[1])
        assert nodes == expected_nodes, (version, nodes, expected_nodes)
        record = {
            "version": version, "bench_positions_equal": len(positions), "bench_depth": 9,
            "teacher_nodes": nodes, "teacher_bytes": output_path.stat().st_size,
            "teacher_sha256": digest, "teacher_matches_base": True, "command": command,
        }
        records.append(record)
        (results / "verification.json").write_text(json.dumps(records, indent=2) + "\n")
        print(f"verified {version}: {len(positions)} bench positions, {nodes} teacher nodes, identical MNSD", flush=True)


if __name__ == "__main__":
    main()
