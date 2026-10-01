"""Reproduce the six search findings in a temporary copy of the audited commit."""

import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile


COMMIT = "64da4619760e2a26b032fad51d8253a18df5714e"
ARTIFACTS = Path(__file__).resolve().parent
REPOSITORY = ARTIFACTS.parents[2]
TEST_MODULE = "search::alphabeta::tests::audit_20260929::"


def main():
    workspace = Path(tempfile.mkdtemp(prefix="minase-search-audit-20260929-"))
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

    regular = subprocess.run(
        [executable, TEST_MODULE, "--test-threads=1", "--nocapture"],
        cwd=workspace, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    (workspace / "regular.log").write_text(regular.stdout)
    print(regular.stdout, flush=True)
    if regular.returncode == 0 or "0 passed; 5 failed; 1 ignored;" not in regular.stdout:
        raise RuntimeError("The five expected ordinary reproduction failures were not observed")

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
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    (workspace / "spawn-failure.log").write_text(injected.stdout)
    print(injected.stdout, flush=True)
    if injected.returncode == 0 or "spawn_failed=true team_stop=false watchdog_expired=true" not in injected.stdout:
        raise RuntimeError("The expected partial-spawn failure was not observed")
    print(f"All six findings reproduced; logs remain in {workspace}", flush=True)


if __name__ == "__main__":
    main()
