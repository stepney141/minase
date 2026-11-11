"""Run a minase command under gdb with uaf_probe.py, print the probe lines, and judge the run.

usage: probe.py --zero-dir DIR --ort-version VERSION --function HEX --static HEX
                --expect {reached,not-reached} --expect-exit N -- COMMAND...

DIR is a tools/train/zero directory that setup.sh has prepared (it holds .runtime and .venv).
HEX values come from find_registry_reset.py for the same ONNX Runtime version.

The run passes only if gdb finished without an error, gdb stopped at the entry of exit() with the
registry alive, the reset function was reached or not as --expect says, and the process ended
with the exit code --expect-exit. The script exits with 0 on a pass and 1 otherwise.
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

SHOWN = ("PROBE", "#", "corrupted", "[Inferior", "Thread 1")
REACHED = {
    "reached": "PROBE reset function reached after exit began",
    "not-reached": "PROBE reset function not reached",
}


def runtime_environment(zero_dir: Path, ort_version: str) -> dict[str, str]:
    """Environment of tools/train/zero/env.sh, with the ONNX Runtime version chosen by the caller."""
    site = zero_dir / ".venv/lib/python3.13/site-packages"
    library = zero_dir / f".runtime/onnxruntime-linux-x64-gpu_cuda13-{ort_version}/lib/libonnxruntime.so"
    if not library.is_file():
        sys.exit(f"{library} does not exist")
    env = dict(os.environ)
    env.pop("CUDA_VISIBLE_DEVICES", None)
    env["ORT_DYLIB_PATH"] = str(library)
    env["LD_LIBRARY_PATH"] = ":".join(
        str(site / part) for part in ("tensorrt_libs", "nvidia/cu13/lib", "nvidia/cudnn/lib")
    )
    return env


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--zero-dir", type=Path, required=True)
    parser.add_argument("--ort-version", required=True)
    parser.add_argument("--function", required=True)
    parser.add_argument("--static", required=True)
    parser.add_argument("--expect", choices=sorted(REACHED), required=True)
    parser.add_argument("--expect-exit", type=int, required=True)
    parser.add_argument("command", nargs="+")
    options = parser.parse_args()

    env = runtime_environment(options.zero_dir.resolve(), options.ort_version)
    env["PROBE_FUNCTION"] = options.function
    env["PROBE_STATIC"] = options.static
    script = Path(__file__).resolve().parent / "uaf_probe.py"
    result = subprocess.run(
        ["gdb", "-q", "-batch", "-x", str(script), "--args", *options.command],
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )
    lines = (result.stdout + result.stderr).splitlines()
    for line in lines:
        if line.startswith(SHOWN) or "SIGABRT" in line or "Error" in line:
            print(line[:240])

    failures = []
    if result.returncode != 0:
        failures.append(f"gdb exited with {result.returncode}")
    if any("Error" in line and "Python" in line for line in lines):
        failures.append("the gdb script raised an error")
    for wanted in (
        "PROBE at exit: registry alive",
        REACHED[options.expect],
        f"PROBE exit code {options.expect_exit}",
    ):
        if wanted not in lines:
            failures.append(f"missing line: {wanted}")
    unwanted = REACHED["reached" if options.expect == "not-reached" else "not-reached"]
    if unwanted in lines:
        failures.append(f"unexpected line: {unwanted}")
    if failures:
        print("VERDICT fail: " + "; ".join(failures))
        sys.exit(1)
    print("VERDICT pass")


if __name__ == "__main__":
    main()
