#!/usr/bin/env python3
"""Extract the changelog section for a single release tag.

Reads a git-cliff changelog file whose release sections start with
`## <tag> (<date>)` and prints the body of the matching section to stdout.
Exits with a non-zero status if the heading is missing, duplicated, or empty.
"""

from __future__ import annotations

import pathlib
import sys


def extract_release_notes(changelog_text: str, tag: str) -> str:
    """Return the markdown body for `tag` without the top-level heading."""
    if not tag or any(ch.isspace() for ch in tag):
        raise ValueError(f"invalid tag name: {tag!r}")

    prefix = f"## {tag} ("
    lines = changelog_text.splitlines()
    matches: list[int] = [
        idx
        for idx, line in enumerate(lines)
        if line.startswith(prefix) and line.endswith(")")
    ]

    if not matches:
        raise ValueError(f"release section for tag {tag!r} not found")
    if len(matches) > 1:
        raise ValueError(f"multiple release sections found for tag {tag!r}")

    start = matches[0] + 1
    end = len(lines)
    for idx in range(start, len(lines)):
        if lines[idx].startswith("## "):
            end = idx
            break

    body = "\n".join(lines[start:end]).strip()
    if not body:
        raise ValueError(f"release section for tag {tag!r} is empty")
    return body + "\n"


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(
            f"Usage: {argv[0]} <changelog-path> <tag>",
            file=sys.stderr,
        )
        return 2

    changelog_path = pathlib.Path(argv[1])
    tag = argv[2]

    try:
        text = changelog_path.read_text(encoding="utf-8")
        notes = extract_release_notes(text, tag)
    except (OSError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    sys.stdout.write(notes)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
