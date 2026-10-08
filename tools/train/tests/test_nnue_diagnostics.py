"""独立した既知評価で、Rust探査の引数、完全一致、および報告を検証する。"""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import numpy as np

from helpers import write_mnsd
from minase_train.data.mnpt import initial_piece_values
from minase_train.diagnostics.nnue import compare, main, rust_probe
from minase_train.nnue.format import write_mnue


class DiagnosticsTest(unittest.TestCase):
    def test_probe_passes_net_positions_and_preserves_command_prefix(self):
        for command in (["nnue_probe"], ["minase", "dev", "nnue-probe"]):
            with patch("minase_train.diagnostics.nnue.subprocess.run", return_value=Mock(stdout="[]")) as run:
                self.assertEqual(rust_probe(command)(Path("net.bin"), Path("samples.bin")), [])
            run.assert_called_once_with(command + ["--net", "net.bin", "--positions", "samples.bin"],
                                        check=True, capture_output=True, text=True)
        with self.assertRaises(ValueError):
            rust_probe([])

    def test_scores_counts_order_and_report_checksums(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            net, data, output = [root / name for name in ("net.bin", "data.bin", "report.json")]
            linear = np.zeros((13680, 2), dtype=np.int16)
            linear[29 * 144:30 * 144] = 8
            write_mnue(net, linear, initial_piece_values(), 1000, np.zeros((13680, 2), dtype=np.int16),
                       np.zeros(2, dtype=np.int32), np.zeros((2, 2), dtype=np.int16), 0)
            board = np.zeros((3, 144), dtype=np.uint8)
            for i, row in enumerate(board):
                row[:i + 1] = 1
            write_mnsd(data, seed=0, checksum=b"a" * 32, games=[0, 1, 2], board=board)
            expected = [{"index": i, "eval": i + 1} for i in range(3)]
            probe = Mock(return_value=expected)
            self.assertEqual(compare(net, data, probe)["samples"], 3)
            probe.assert_called_once_with(net, data)
            for wrong in (expected[:-1], expected + [{"index": 3, "eval": 4}], expected[::-1],
                          [{"index": 0, "eval": 2}, *expected[1:]],
                          [{"index": 0, "eval": 1.0}, *expected[1:]],
                          [{"index": False, "eval": 1}, *expected[1:]],
                          [{"index": 0}, *expected[1:]], [None, *expected[1:]]):
                with self.subTest(rows=wrong), self.assertRaisesRegex(ValueError, "index [0-3]: Python=.*Rust="):
                    compare(net, data, Mock(return_value=wrong))
            with patch("minase_train.diagnostics.nnue.rust_probe", return_value=probe):
                main(["--net", str(net), "--positions", str(data), "--output", str(output),
                      "--probe-command", "/bin/true"])
            report = json.loads(output.read_text())
            for key, path in (("net_sha256", net), ("positions_sha256", data), ("probe_sha256", Path("/bin/true"))):
                self.assertEqual(report[key], hashlib.sha256(path.read_bytes()).hexdigest())
            self.assertEqual(report["probe_command"], ["/bin/true", "--net", str(net), "--positions", str(data)])
            with self.assertRaisesRegex(ValueError, "already exists"):
                main(["--net", str(net), "--positions", str(data), "--output", str(output),
                      "--probe-command", "/bin/true"])


if __name__ == "__main__":
    unittest.main()
