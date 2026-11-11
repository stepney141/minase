"""Send a fixed list of USI commands to an engine and log the exchange.

usage: usi_session.py COMMANDS_FILE -- ENGINE_COMMAND...

COMMANDS_FILE holds one USI command per line. After "usi", "isready", and "go ..." the script
waits for "usiok", "readyok", and "bestmove" before it sends the next command. It sends "quit"
at the end and prints the exit code of the engine. Run it in the environment of
tools/train/zero/env.sh.
"""

import subprocess
import sys
import time
from pathlib import Path

WAIT_FOR = {"usi": "usiok", "isready": "readyok", "go": "bestmove"}


def main() -> None:
    if len(sys.argv) < 4 or sys.argv[2] != "--":
        sys.exit(__doc__)
    commands = [line for line in Path(sys.argv[1]).read_text().splitlines() if line.strip()]
    engine = subprocess.Popen(
        sys.argv[3:], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1
    )
    assert engine.stdin is not None and engine.stdout is not None
    for command in commands:
        print(f"> {command}", flush=True)
        engine.stdin.write(command + "\n")
        engine.stdin.flush()
        wanted = WAIT_FOR.get(command.split()[0])
        if wanted is None:
            continue
        start = time.monotonic()
        while True:
            line = engine.stdout.readline()
            if not line:
                print(f"engine closed its output; exit code {engine.wait()}")
                sys.exit(1)
            line = line.rstrip("\n")
            if line.startswith(wanted):
                print(f"< {line}  [{time.monotonic() - start:.2f}s]", flush=True)
                break
            if line.startswith("info string") or not line.startswith(("info", "id ", "option ")):
                print(f"< {line}", flush=True)
    print("> quit", flush=True)
    engine.stdin.write("quit\n")
    engine.stdin.flush()
    code = engine.wait(timeout=120)
    print(f"exit code {code}")
    sys.exit(0 if code == 0 else 1)


if __name__ == "__main__":
    main()
