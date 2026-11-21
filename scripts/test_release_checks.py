#!/usr/bin/env python3
"""Unit tests for scripts/release_checks.py."""

from __future__ import annotations

import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import release_checks  # noqa: E402

SAMPLE_CHANGELOG = """\
## v3.0.0-alpha.1 (2026-11-20)

Summary line for alpha.

### Features

- **eval:** Read the NNUE network from EvalFile

## v2.6.0 (2026-10-25)

### Bug fixes

- **search:** Fix deadline handling

## minase-core-v0.3.0 (2026-11-20)

### Performance

- **movegen:** Speed up slider lookups
"""


def _write_fake_workspace(root: pathlib.Path, engine_ver: str, core_ver: str) -> None:
    (root / "crates/minase").mkdir(parents=True)
    (root / "crates/minase-core").mkdir(parents=True)
    (root / "crates/minase/Cargo.toml").write_text(
        f'[package]\nname = "minase"\nversion = "{engine_ver}"\n',
        encoding="utf-8",
    )
    (root / "crates/minase-core/Cargo.toml").write_text(
        f'[package]\nname = "minase-core"\nversion = "{core_ver}"\n',
        encoding="utf-8",
    )


class ResolveTagMetadataTests(unittest.TestCase):
    def test_resolves_engine_prerelease_tag(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            _write_fake_workspace(root, "3.0.0-alpha.1", "0.3.0")
            meta = release_checks.resolve_tag_metadata("v3.0.0-alpha.1", root)
            self.assertEqual(
                meta,
                {
                    "crate": "minase",
                    "version": "3.0.0-alpha.1",
                    "changelog": "CHANGELOG.md",
                    "prerelease": "true",
                    "title": "minase v3.0.0-alpha.1",
                },
            )

    def test_resolves_core_stable_tag(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            _write_fake_workspace(root, "3.0.0", "0.3.1")
            meta = release_checks.resolve_tag_metadata("minase-core-v0.3.1", root)
            self.assertEqual(
                meta,
                {
                    "crate": "minase-core",
                    "version": "0.3.1",
                    "changelog": "crates/minase-core/CHANGELOG.md",
                    "prerelease": "false",
                    "title": "minase-core v0.3.1",
                },
            )

    def test_rejects_version_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            _write_fake_workspace(root, "3.0.0-alpha.1", "0.3.0")
            with self.assertRaisesRegex(ValueError, "expects minase version 3.0.0"):
                release_checks.resolve_tag_metadata("v3.0.0", root)

    def test_rejects_unsupported_prefix_and_empty_version(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            _write_fake_workspace(root, "3.0.0", "0.3.0")
            with self.assertRaisesRegex(ValueError, "unsupported release tag"):
                release_checks.resolve_tag_metadata("net-nnue-p-20261121", root)
            with self.assertRaisesRegex(ValueError, "empty version"):
                release_checks.resolve_tag_metadata("v", root)


class CheckPackageNetsTests(unittest.TestCase):
    def test_accepts_exact_allowlist_with_other_crate_files(self) -> None:
        files = [
            "Cargo.toml",
            "src/lib.rs",
            *sorted(release_checks.EXPECTED_MINASE_NETS),
        ]
        release_checks.check_nets_list(files)

    def test_rejects_unexpected_production_net(self) -> None:
        files = [
            *sorted(release_checks.EXPECTED_MINASE_NETS),
            "nets/minase-nnue-p-jcsa-20261118-37054c9e.nnue",
        ]
        with self.assertRaisesRegex(ValueError, "unexpected files under nets/"):
            release_checks.check_nets_list(files)

    def test_rejects_missing_expected_net(self) -> None:
        files = sorted(release_checks.EXPECTED_MINASE_NETS)[:-1]
        with self.assertRaisesRegex(ValueError, "unexpected files under nets/"):
            release_checks.check_nets_list(files)


class ExtractReleaseNotesTests(unittest.TestCase):
    def test_extracts_first_prerelease_section(self) -> None:
        notes = release_checks.extract_release_notes(
            SAMPLE_CHANGELOG, "v3.0.0-alpha.1"
        )
        self.assertEqual(
            notes,
            "Summary line for alpha.\n\n### Features\n\n- **eval:** Read the NNUE network from EvalFile\n",
        )

    def test_extracts_middle_section(self) -> None:
        notes = release_checks.extract_release_notes(SAMPLE_CHANGELOG, "v2.6.0")
        self.assertEqual(
            notes,
            "### Bug fixes\n\n- **search:** Fix deadline handling\n",
        )

    def test_extracts_last_prefixed_crate_section(self) -> None:
        notes = release_checks.extract_release_notes(
            SAMPLE_CHANGELOG, "minase-core-v0.3.0"
        )
        self.assertEqual(
            notes,
            "### Performance\n\n- **movegen:** Speed up slider lookups\n",
        )

    def test_rejects_prefix_collision_tag(self) -> None:
        with self.assertRaisesRegex(ValueError, "not found"):
            release_checks.extract_release_notes(SAMPLE_CHANGELOG, "v3.0.0")

    def test_rejects_missing_empty_duplicate_and_whitespace_tags(self) -> None:
        with self.assertRaisesRegex(ValueError, "not found"):
            release_checks.extract_release_notes(SAMPLE_CHANGELOG, "v9.9.9")

        empty_changelog = "## v1.0.0 (2026-09-20)\n\n## v0.9.0 (2026-09-01)\n\n- Init\n"
        with self.assertRaisesRegex(ValueError, "empty"):
            release_checks.extract_release_notes(empty_changelog, "v1.0.0")

        dup_changelog = (
            "## v1.0.0 (2026-09-20)\n\n- First\n\n"
            "## v1.0.0 (2026-09-21)\n\n- Second\n"
        )
        with self.assertRaisesRegex(ValueError, "multiple"):
            release_checks.extract_release_notes(dup_changelog, "v1.0.0")

        with self.assertRaisesRegex(ValueError, "invalid tag"):
            release_checks.extract_release_notes(SAMPLE_CHANGELOG, "v1.0.0 extra")


class CliTests(unittest.TestCase):
    def test_resolve_tag_and_extract_notes_cli(self) -> None:
        script = pathlib.Path(__file__).resolve().parent / "release_checks.py"
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            _write_fake_workspace(root, "2.6.0", "0.2.0")
            gh_out = root / "gh_output.txt"
            changelog = root / "CHANGELOG.md"
            changelog.write_text(SAMPLE_CHANGELOG, encoding="utf-8")

            res = subprocess.run(
                [
                    sys.executable,
                    str(script),
                    "resolve-tag",
                    "v2.6.0",
                    "--repo-root",
                    str(root),
                    "--github-output",
                    str(gh_out),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(res.returncode, 0, msg=res.stderr)
            self.assertIn("crate=minase\n", gh_out.read_text(encoding="utf-8"))
            self.assertIn("prerelease=false\n", gh_out.read_text(encoding="utf-8"))

            notes_ok = subprocess.run(
                [sys.executable, str(script), "extract-notes", str(changelog), "v2.6.0"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(notes_ok.returncode, 0, msg=notes_ok.stderr)
            self.assertIn("Fix deadline handling", notes_ok.stdout)

            notes_missing = subprocess.run(
                [sys.executable, str(script), "extract-notes", str(changelog), "v2.7.0"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(notes_missing.returncode, 1)
            self.assertIn("not found", notes_missing.stderr)


if __name__ == "__main__":
    unittest.main()
