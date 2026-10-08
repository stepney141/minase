"""整数式で決めた合成期待値で、Rust照合の呼出しと不一致検出を検査する。"""

import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import numpy as np

from minase_train.data.features import FEATURE_COUNT
from minase_train.data.mnpt import initial_piece_values, write_mnpt, write_mnpt_v3
from minase_train.data.mnsd import RECORD_DTYPE, write_mnsd
from minase_train.diagnostics.fm import compare, rust_probe, main


class FMProbeTest(unittest.TestCase):
    def test_probe_protocol_preserves_fm_eval_flags(self):
        for prefix in (["pst_probe"], ["minase", "dev", "pst-probe"]):
            for promotions, moves in ((False, False), (True, False), (False, True), (True, True)):
                with patch("minase_train.diagnostics.fm.subprocess.run", return_value=Mock(stdout="[]")) as run:
                    self.assertEqual(rust_probe(prefix)(Path("fm.bin"), Path("samples.bin"), promotions, moves), [])
                expected = prefix + ["--pst", "fm.bin", "--positions", "samples.bin"]
                if promotions:
                    expected += ["--promotions"]
                if moves:
                    expected += ["--moves"]
                run.assert_called_once_with(expected, check=True, capture_output=True, text=True)

    def test_synthetic_scores_and_pst_component_must_both_agree(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            base, candidate, positions = [root / name for name in ("base.bin", "fm.bin", "positions.bin")]
            weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
            weights[29 * 144:29 * 144 + 3] = 8  # Each pawn adds 1 cp.
            u = np.zeros((FEATURE_COUNT, 1), dtype=np.int16)
            u[29 * 144:29 * 144 + 3, 0] = [7, -5, 11]
            values = initial_piece_values()
            write_mnpt(base, weights, weights, values, 1000)
            write_mnpt_v3(candidate, weights, weights, values, 1000, u, np.array([1]), 1)
            records = np.zeros(3, dtype=RECORD_DTYPE)
            records["lion"] = 255
            records["board"][0, :1] = 1
            records["board"][1, :2] = 1
            records["board"][2, :3] = 1
            write_mnsd(positions, records, seed=0, network_checksum=b"a" * 32)
            # 1; 2+trunc(-35/4)=-6; 3+trunc((-35+77-55)/4)=0.
            expected = [{"index": 0, "eval": 1, "eval_pst": 1},
                        {"index": 1, "eval": -6, "eval_pst": 2},
                        {"index": 2, "eval": 0, "eval_pst": 3}]
            probe = Mock(return_value=expected)
            self.assertEqual(compare(base, candidate, positions, probe)["samples"], 3)
            probe.assert_called_once_with(candidate, positions, False, False)
            for altered in (expected[:-1], expected[::-1],
                            [dict(expected[0], skipped="invalid"), *expected[1:]],
                            [dict(expected[0], eval=2), *expected[1:]],
                            [dict(expected[0], eval_pst=2), *expected[1:]]):
                with self.subTest(rows=altered), self.assertRaises(ValueError):
                    compare(base, candidate, positions, Mock(return_value=altered))
            weights[0] = 1
            write_mnpt_v3(candidate, weights, weights, values, 1000, u, np.array([1]), 1)
            with self.assertRaisesRegex(ValueError, "fixed middlegame"):
                compare(base, candidate, positions, probe)


    def test_pst_changed_checks_candidate_endpoints_and_rejects_wrong_pst_score(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            base, candidate, positions = [root / name for name in ("base.bin", "joint.bin", "positions.bin")]
            zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
            weights = zeros.copy()
            weights[29 * 144:29 * 144 + 2] = 16
            u = np.zeros((FEATURE_COUNT, 1), dtype=np.int16)
            u[29 * 144:29 * 144 + 2, 0] = [7, -5]
            values = initial_piece_values()
            write_mnpt_v3(base, zeros, zeros, values, 1000, u, np.array([1]), 1)
            write_mnpt_v3(candidate, weights, weights, values, 1000, u, np.array([1]), 1)
            records = np.zeros(1, dtype=RECORD_DTYPE)
            records["lion"] = 255
            records["board"][:, :2] = 1
            write_mnsd(positions, records, seed=0, network_checksum=b"a" * 32)
            # 候補PST=4cp、FM=trunc(-35/4)=-8cp、合計=-4cp。
            expected = [{"index": 0, "eval": -4, "eval_pst": 4}]
            probe = Mock(return_value=expected)
            self.assertEqual(compare(base, candidate, positions, probe, pst_changed=True)["samples"], 1)
            with self.assertRaises(ValueError):
                compare(base, candidate, positions,
                        Mock(return_value=[dict(expected[0], eval_pst=5)]), pst_changed=True)
            output = root / "diagnostics.json"
            with patch("minase_train.diagnostics.fm.rust_probe", return_value=probe):
                main(["--base", str(base), "--candidate", str(candidate), "--positions", str(positions),
                      "--output", str(output), "--pst-changed", "--probe-command", "/bin/true"])
            self.assertEqual(json.loads(output.read_text())["candidate_pst"], 1)


if __name__ == "__main__":
    unittest.main()
