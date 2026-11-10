"""同じ機械でのONNX再書き出しの一致と、既存の出力の保全を確かめる。"""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from minase_zero.train import export_checkpoint
from tests.cli import COMMIT, require_success, run_cli


class ExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-export-test-")
        cls.addClassCleanup(directory.cleanup)
        cls.root = Path(directory.name)
        cls.initial = cls.root / "initial"
        require_success(run_cli([
            "init", "--blocks", "1", "--channels", "8", "--seed", "23",
            "--out-dir", str(cls.initial), "--trainer-commit", COMMIT,
        ]))

    def test_export_matches_init_bytes_and_reports_written_checksum(self) -> None:
        output = self.root / "exported.onnx"
        result = run_cli([
            "export-onnx", "--checkpoint", str(self.initial / "checkpoint.pt"),
            "--out", str(output),
        ])
        require_success(result)
        self.assertEqual(output.read_bytes(), (self.initial / "model.onnx").read_bytes())
        self.assertEqual(len(result.stdout.splitlines()), 1)
        self.assertEqual(json.loads(result.stdout), {
            "network_sha256": hashlib.sha256(output.read_bytes()).hexdigest(),
        })

    def test_existing_output_raises_and_cli_preserves_its_contents(self) -> None:
        output = self.root / "existing.onnx"
        output.write_bytes(b"preserve this file")
        checkpoint = self.initial / "checkpoint.pt"
        with self.assertRaises(ValueError):
            export_checkpoint(checkpoint, output)
        result = run_cli([
            "export-onnx", "--checkpoint", str(checkpoint), "--out", str(output),
        ])
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertIn("output", result.stderr)
        self.assertEqual(output.read_bytes(), b"preserve this file")

    def test_checkpoint_and_output_are_required(self) -> None:
        for arguments in (
            ["--checkpoint", str(self.initial / "checkpoint.pt")],
            ["--out", str(self.root / "missing-checkpoint.onnx")],
        ):
            with self.subTest(arguments=arguments):
                result = run_cli(["export-onnx", *arguments])
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("required", result.stderr)


if __name__ == "__main__":
    unittest.main()
