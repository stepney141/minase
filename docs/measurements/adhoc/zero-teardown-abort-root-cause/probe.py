"""Run a minase command under gdb with uaf_probe.py and print the probe lines.

usage: probe.py --zero-dir DIR --ort-version VERSION --function HEX --static HEX -- COMMAND...

DIR is a tools/train/zero directory that setup.sh has prepared (it holds .runtime and .venv).
HEX values come from find_registry_reset.py for the same ONNX Runtime version.
"""

import argparse
import os
import subprocess
import sys
from pathlib import Path

PREFIXES = ("PROBE", "#", "corrupted", "[Inferior", "Thread 1")


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
    for line in (result.stdout + result.stderr).splitlines():
        if line.startswith(PREFIXES) or "SIGABRT" in line or "Error" in line:
            print(line[:240])


if __name__ == "__main__":
    main()
