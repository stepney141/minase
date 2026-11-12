"""環境の入力のバイト比較と、比較に成功した場合だけの同期を検証する。"""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class EnvInputsTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.environment = self.root / "environment with spaces"
        self.source = self.root / "worktree with spaces/tools/train/zero"
        self.environment.mkdir()
        self.source.mkdir(parents=True)
        self.inputs = ("setup.sh", "env.sh", "pyproject.toml", "uv.lock", ".python-version")
        for name in self.inputs:
            content = f"contents of {name}\n".encode()
            (self.environment / name).write_bytes(content)
            (self.source / name).write_bytes(content)
        scripts = Path(__file__).resolve().parents[1]
        for name in ("check-env-inputs.sh", "sync-env.sh"):
            shutil.copy2(scripts / name, self.source / name)

        executable = self.root / "bin/uv"
        executable.parent.mkdir()
        executable.write_text('#!/usr/bin/env bash\nset -euo pipefail\nprintf "%s\\n" "$@" >> "$UV_CALLS"\n')
        executable.chmod(0o755)
        self.calls = self.root / "uv-calls"
        self.env = {**os.environ,
                    "UV_PROJECT_ENVIRONMENT": str(self.environment / ".venv"),
                    "PATH": str(executable.parent) + os.pathsep + os.environ["PATH"],
                    "UV_CALLS": str(self.calls)}

    def run_script(self, name, *args):
        return subprocess.run([str(self.source / name), *args], cwd=self.root, env=self.env,
                              capture_output=True, text=True, timeout=5)

    def assert_mismatches_reported(self, stderr, *names):
        lines = stderr.splitlines()
        self.assertEqual(len(lines) - 1, len(names))
        self.assertIn("作り直す必要があります", lines[-1])
        environment = Path(self.env["UV_PROJECT_ENVIRONMENT"]).parent
        for name in self.inputs:
            if name in names:
                self.assertIn(str(self.source / name), stderr)
                self.assertIn(str(environment / name), stderr)
            else:
                self.assertNotIn(str(self.source / name), stderr)
                self.assertNotIn(str(environment / name), stderr)

    def test_identical_inputs_succeed_silently(self):
        result = self.run_script("check-env-inputs.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((result.stdout, result.stderr), ("", ""))

    def test_one_byte_change_reports_only_the_changed_input(self):
        for name in self.inputs:
            with self.subTest(name=name):
                path = self.source / name
                original = path.read_bytes()
                path.write_bytes(b"C" + original[1:])
                result = self.run_script("check-env-inputs.sh")
                path.write_bytes(original)
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assert_mismatches_reported(result.stderr, name)

    def test_missing_source_input_fails(self):
        (self.source / ".python-version").unlink()
        result = self.run_script("check-env-inputs.sh")
        self.assertEqual(result.returncode, 1)
        self.assert_mismatches_reported(result.stderr, ".python-version")

    def test_missing_environment_input_fails(self):
        (self.environment / "uv.lock").unlink()
        result = self.run_script("check-env-inputs.sh")
        self.assertEqual(result.returncode, 1)
        self.assert_mismatches_reported(result.stderr, "uv.lock")

    def test_all_mismatched_inputs_are_reported(self):
        for name in ("setup.sh", ".python-version"):
            (self.source / name).write_bytes(b"changed\n")
        result = self.run_script("check-env-inputs.sh")
        self.assertEqual(result.returncode, 1)
        self.assert_mismatches_reported(result.stderr, "setup.sh", ".python-version")

    def test_unset_environment_fails(self):
        del self.env["UV_PROJECT_ENVIRONMENT"]
        result = self.run_script("check-env-inputs.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("UV_PROJECT_ENVIRONMENT", result.stderr)

    def test_same_directory_is_checked_for_missing_inputs(self):
        self.env["UV_PROJECT_ENVIRONMENT"] = str(self.source / ".venv")
        result = self.run_script("check-env-inputs.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((result.stdout, result.stderr), ("", ""))
        (self.source / ".python-version").unlink()
        result = self.run_script("check-env-inputs.sh")
        self.assertEqual(result.returncode, 1)
        self.assert_mismatches_reported(result.stderr, ".python-version")

    def test_sync_runs_once_with_the_worktree_project(self):
        result = self.run_script("sync-env.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(),
                         ["sync", "--frozen", "--project", str(self.source)])

    def test_sync_does_not_run_when_inputs_differ(self):
        (self.source / "uv.lock").write_bytes(b"changed\n")
        result = self.run_script("sync-env.sh")
        self.assertNotEqual(result.returncode, 0)
        self.assert_mismatches_reported(result.stderr, "uv.lock")
        self.assertFalse(self.calls.exists())

    def test_sync_rejects_arguments_without_running_uv(self):
        result = self.run_script("sync-env.sh", "--offline")
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(result.stderr)
        self.assertFalse(self.calls.exists())


if __name__ == "__main__":
    unittest.main()
