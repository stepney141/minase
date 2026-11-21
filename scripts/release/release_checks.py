#!/usr/bin/env python3
"""Release verification helpers invoked by GitHub Actions and local checks.

Subcommands:
  resolve-tag <tag> [--repo-root DIR] [--github-output FILE]
      Verify that <tag> matches the target crate's Cargo.toml version and emit
      crate metadata as key=value pairs.
  check-package-nets [--repo-root DIR] [--allow-dirty]
      Verify that `cargo package -p minase --list` contains only the tracked
      non-production files under nets/.
  extract-notes <changelog-path> <tag>
      Extract the markdown body for <tag> from a git-cliff changelog file.
"""

from __future__ import annotations

import argparse
import pathlib
import subprocess
import sys
import tomllib
from collections.abc import Iterable

EXPECTED_MINASE_NETS: frozenset[str] = frozenset(
    {
        "nets/nnue-epoch0-pst.bin",
        "nets/pst-init.bin",
        "nets/pst.bin",
        "nets/test-layer-stack.bin",
    }
)


def resolve_tag_metadata(tag: str, repo_root: pathlib.Path) -> dict[str, str]:
    """Parse a crate release tag and verify it against Cargo.toml."""
    if not tag or any(ch.isspace() for ch in tag):
        raise ValueError(f"invalid tag name: {tag!r}")

    if tag.startswith("minase-core-v"):
        crate = "minase-core"
        version = tag.removeprefix("minase-core-v")
        manifest = repo_root / "crates/minase-core/Cargo.toml"
        changelog = "crates/minase-core/CHANGELOG.md"
    elif tag.startswith("v"):
        crate = "minase"
        version = tag.removeprefix("v")
        manifest = repo_root / "crates/minase/Cargo.toml"
        changelog = "CHANGELOG.md"
    else:
        raise ValueError(f"unsupported release tag: {tag!r}")

    if not version:
        raise ValueError(f"tag {tag!r} has an empty version")

    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    manifest_version = data["package"]["version"]
    if manifest_version != version:
        raise ValueError(
            f"tag {tag} expects {crate} version {version}, "
            f"but {manifest} has {manifest_version}"
        )

    return {
        "crate": crate,
        "version": version,
        "changelog": changelog,
        "prerelease": "true" if "-" in version else "false",
        "title": f"{crate} v{version}",
    }


def check_nets_list(package_files: Iterable[str]) -> None:
    """Verify that the packaged nets/ files exactly match the allowlist."""
    actual_nets = frozenset(
        line.strip()
        for line in package_files
        if line.strip().startswith("nets/")
    )
    if actual_nets != EXPECTED_MINASE_NETS:
        raise ValueError(
            "unexpected files under nets/ in minase package:\n"
            f"  expected: {sorted(EXPECTED_MINASE_NETS)}\n"
            f"  actual:   {sorted(actual_nets)}"
        )


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


def _cmd_resolve_tag(args: argparse.Namespace) -> int:
    metadata = resolve_tag_metadata(args.tag, pathlib.Path(args.repo_root))
    rendered = "".join(f"{key}={value}\n" for key, value in metadata.items())
    if args.github_output is not None:
        with pathlib.Path(args.github_output).open("a", encoding="utf-8") as out:
            out.write(rendered)
    else:
        sys.stdout.write(rendered)
    return 0


def _cmd_check_package_nets(args: argparse.Namespace) -> int:
    cmd = ["cargo", "package", "-p", "minase", "--list"]
    if args.allow_dirty:
        cmd.append("--allow-dirty")
    proc = subprocess.run(
        cmd,
        cwd=args.repo_root,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            f"cargo package --list failed (exit {proc.returncode}):\n{proc.stderr.strip()}"
        )
    check_nets_list(proc.stdout.splitlines())
    return 0


def _cmd_extract_notes(args: argparse.Namespace) -> int:
    text = pathlib.Path(args.changelog).read_text(encoding="utf-8")
    sys.stdout.write(extract_release_notes(text, args.tag))
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Release verification helpers for minase."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    resolve_p = subparsers.add_parser(
        "resolve-tag",
        help="Validate a release tag against Cargo.toml and print metadata.",
    )
    resolve_p.add_argument("tag", help="Git tag name (vX.Y.Z or minase-core-vX.Y.Z)")
    resolve_p.add_argument(
        "--repo-root",
        default=".",
        help="Path to the repository root (default: current directory)",
    )
    resolve_p.add_argument(
        "--github-output",
        default=None,
        help="Append key=value outputs to this file instead of stdout",
    )
    resolve_p.set_defaults(func=_cmd_resolve_tag)

    nets_p = subparsers.add_parser(
        "check-package-nets",
        help="Verify nets/ files included by `cargo package -p minase --list`.",
    )
    nets_p.add_argument(
        "--repo-root",
        default=".",
        help="Path to the repository root (default: current directory)",
    )
    nets_p.add_argument(
        "--allow-dirty",
        action="store_true",
        help="Pass --allow-dirty to cargo package",
    )
    nets_p.set_defaults(func=_cmd_check_package_nets)

    notes_p = subparsers.add_parser(
        "extract-notes",
        help="Extract the markdown section for a tag from a changelog file.",
    )
    notes_p.add_argument("changelog", help="Path to CHANGELOG.md")
    notes_p.add_argument("tag", help="Git tag name to extract")
    notes_p.set_defaults(func=_cmd_extract_notes)

    return parser


def main(argv: list[str]) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except (OSError, ValueError, RuntimeError, KeyError, tomllib.TOMLDecodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
