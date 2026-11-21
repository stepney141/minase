#!/usr/bin/env python3
"""Unit tests for scripts/extract_release_notes.py."""

from __future__ import annotations

import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import extract_release_notes  # noqa: E402

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


class ExtractReleaseNotesTests(unittest.TestCase):
    def test_extracts_first_prerelease_section(self) -> None:
        notes = extract_release_notes.extract_release_notes(
            SAMPLE_CHANGELOG, "v3.0.0-alpha.1"
        )
        self.assertEqual(
            notes,
            "Summary line for alpha.\n\n### Features\n\n- **eval:** Read the NNUE network from EvalFile\n",
        )

    def test_extracts_middle_section(self) -> None:
        notes = extract_release_notes.extract_release_notes(SAMPLE_CHANGELOG, "v2.6.0")
        self.assertEqual(
            notes,
            "### Bug fixes\n\n- **search:** Fix deadline handling\n",
        )

    def test_extracts_last_prefixed_crate_section(self) -> None:
        notes = extract_release_notes.extract_release_notes(
            SAMPLE_CHANGELOG, "minase-core-v0.3.0"
        )
        self.assertEqual(
            notes,
            "### Performance\n\n- **movegen:** Speed up slider lookups\n",
        )

    def test_rejects_prefix_collision_tag(self) -> None:
        with self.assertRaisesRegex(ValueError, "not found"):
            extract_release_notes.extract_release_notes(SAMPLE_CHANGELOG, "v3.0.0")

    def test_rejects_missing_tag(self) -> None:
        with self.assertRaisesRegex(ValueError, "not found"):
            extract_release_notes.extract_release_notes(SAMPLE_CHANGELOG, "v9.9.9")

    def test_rejects_empty_section(self) -> None:
        empty_changelog = "## v1.0.0 (2026-09-20)\n\n## v0.9.0 (2026-09-01)\n\n- Init\n"
        with self.assertRaisesRegex(ValueError, "empty"):
            extract_release_notes.extract_release_notes(empty_changelog, "v1.0.0")

    def test_rejects_duplicate_tag(self) -> None:
        dup_changelog = (
            "## v1.0.0 (2026-09-20)\n\n- First\n\n"
            "## v1.0.0 (2026-09-21)\n\n- Second\n"
        )
        with self.assertRaisesRegex(ValueError, "multiple"):
            extract_release_notes.extract_release_notes(dup_changelog, "v1.0.0")

    def test_rejects_invalid_tag_with_whitespace(self) -> None:
        with self.assertRaisesRegex(ValueError, "invalid tag"):
            extract_release_notes.extract_release_notes(SAMPLE_CHANGELOG, "v1.0.0 extra")

    def test_cli_succeeds_and_fails_fast(self) -> None:
        script = pathlib.Path(__file__).resolve().parent / "extract_release_notes.py"
        with tempfile.TemporaryDirectory() as tmp:
            changelog = pathlib.Path(tmp) / "CHANGELOG.md"
            changelog.write_text(SAMPLE_CHANGELOG, encoding="utf-8")

            ok = subprocess.run(
                [sys.executable, str(script), str(changelog), "v2.6.0"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(ok.returncode, 0, msg=ok.stderr)
            self.assertIn("Fix deadline handling", ok.stdout)

            missing = subprocess.run(
                [sys.executable, str(script), str(changelog), "v2.7.0"],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(missing.returncode, 1)
            self.assertIn("not found", missing.stderr)

            bad_args = subprocess.run(
                [sys.executable, str(script), str(changelog)],
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(bad_args.returncode, 2)


if __name__ == "__main__":
    unittest.main()
