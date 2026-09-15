#!/usr/bin/env python3
"""minaseクレートに、リポジトリに固定したPGOプロファイルを適用する。"""

import os
import sys
from pathlib import Path


def main():
    command = sys.argv[1:]
    if not command:
        raise SystemExit("pgo_rustc.py requires a compiler command")
    is_minase = any(
        argument == "--crate-name" and value == "minase"
        for argument, value in zip(command, command[1:])
    )
    if is_minase and os.environ.get("MINASE_PGO_GENERATE") != "1":
        profile = Path(__file__).resolve().parent.parent / "pgo/minase.profdata"
        if not profile.is_file():
            print(
                f"PGO profile is missing: {profile}\n"
                "Run python3 scripts/pgo_profile.py to regenerate it.",
                file=sys.stderr,
            )
            return 1
        command.extend(["-C", f"profile-use={profile}"])
    os.execvp(command[0], command)


if __name__ == "__main__":
    sys.exit(main())
