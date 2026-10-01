"""Reproduce two termination defects in a temporary copy of the audited commit."""

import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile


COMMIT = "c76eaeae1b419f22e72e267a8ef0cb9ab92a81eb"
ARTIFACTS = Path(__file__).resolve().parent
REPOSITORY = ARTIFACTS.parents[2]
TEST_MODULE = "search::alphabeta::tests::control_audit_20260929::"


def main():
    workspace = Path(tempfile.mkdtemp(prefix="minase-search-control-audit-20260929-"))
    print(f"Audited commit: {COMMIT}", flush=True)
    print(f"Temporary source and logs: {workspace}", flush=True)
    archive = subprocess.run(
        ["git", "archive", COMMIT, "src", "nets", "tests", ".cargo", "Cargo.toml", "Cargo.lock"],
        cwd=REPOSITORY, check=True, capture_output=True,
    ).stdout
    with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as contents:
        contents.extractall(workspace, filter="data")
    subprocess.run(["git", "apply", str(ARTIFACTS / "repro.patch")], cwd=workspace, check=True)

    environment = os.environ.copy()
    environment["CARGO_PROFILE_DEV_OPT_LEVEL"] = "0"
    build = subprocess.run(
        ["nice", "-n", "10", "cargo", "test", "--locked", "--lib", "--no-run", "-j", "1", "--message-format=json"],
        cwd=workspace, env=environment, text=True, capture_output=True,
    )
    (workspace / "build.jsonl").write_text(build.stdout)
    (workspace / "build.log").write_text(build.stderr)
    build.check_returncode()
    executables = []
    for line in build.stdout.splitlines():
        message = json.loads(line)
        if message["reason"] == "compiler-artifact" and message["executable"] is not None:
            executables.append(message["executable"])
    if len(executables) != 1:
        raise RuntimeError(f"Expected one library test executable, found {executables}")
    executable = executables[0]

    aspiration = subprocess.run(
        [executable, TEST_MODULE + "audit_positive_pawn_values_allow_depth_five_to_complete", "--exact", "--test-threads=1", "--nocapture"],
        cwd=workspace, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        timeout=60,
    )
    (workspace / "aspiration.log").write_text(aspiration.stdout)
    print(aspiration.stdout, flush=True)
    if aspiration.returncode != 101 or "0 passed; 1 failed;" not in aspiration.stdout:
        raise RuntimeError("The expected aspiration reproduction failure was not observed")
    for pawn in (1, 2):
        observation = f"pawn={pawn}: depth=4, nodes=10000, delta=0, widened=(0, 0, 0)"
        if observation not in aspiration.stdout:
            raise RuntimeError(f"Missing observation: {observation}")

    comparison = subprocess.run(
        [executable, TEST_MODULE + "review_extra_pawn_values", "--exact", "--test-threads=1", "--nocapture"],
        cwd=workspace, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
        timeout=60,
    )
    (workspace / "review-extra.log").write_text(comparison.stdout)
    print(comparison.stdout, flush=True)
    comparison.check_returncode()
    observations = [
        "extra pawn=3 cap=10000: depth=5 nodes=103 delta=1 d4_score=0 d4_nodes=64",
        "extra pawn=4 cap=10000: depth=5 nodes=103 delta=1 d4_score=0 d4_nodes=64",
        "extra pawn=1 cap=100000: depth=4 nodes=100000 delta=0 d4_score=0 d4_nodes=64",
        "extra pawn=100 cap=100000: depth=5 nodes=132 delta=46 d4_score=0 d4_nodes=78",
    ]
    for observation in observations:
        if observation not in comparison.stdout:
            raise RuntimeError(f"Missing comparison observation: {observation}")

    injector = workspace / "fail_pthread_create.so"
    subprocess.run(
        ["cc", "-shared", "-fPIC", "-o", str(injector), str(ARTIFACTS / "fail_pthread_create.c"), "-ldl"],
        cwd=workspace, check=True,
    )
    injected_environment = os.environ.copy()
    injected_environment["LD_PRELOAD"] = str(injector)
    injected_environment["MINASE_REVIEW_FAIL_PTHREAD_AT"] = "3"
    injected = subprocess.run(
        [executable, TEST_MODULE + "audit_partial_spawn_failure_stops_existing_auxiliary", "--exact", "--ignored", "--test-threads=1", "--nocapture"],
        cwd=workspace, env=injected_environment, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=60,
    )
    (workspace / "spawn-failure.log").write_text(injected.stdout)
    print(injected.stdout, flush=True)
    if injected.returncode != 101 or "spawn_failed=true team_stop=false watchdog_expired=true" not in injected.stdout:
        raise RuntimeError("The expected partial-spawn failure was not observed")
    print(f"Both defects reproduced; logs remain in {workspace}", flush=True)


if __name__ == "__main__":
    main()
