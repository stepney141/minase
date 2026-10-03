"""駒数分布と端点の識別性の報告を検証する。"""

from __future__ import annotations

import csv
import io
import json
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch

import numpy as np

from helpers import write_mnsd
from minase_train.data.mnpt import initial_piece_values, write_mnpt
from minase_train.diagnostics.taper_report import main as report_main


class MirroredIdentifiabilityTest(unittest.TestCase):

    def test_report_means_phase_over_training_positions_and_uses_model_feature_units(self) -> None:
        # 既知の分割ではseed=11のgame=0,1が訓練、game=4が検証。
        # 訓練のφは0と1なので局面平均は0.5。駒数重み付きや検証込みの平均とは異なる。
        boards = np.zeros((3, 144), dtype=np.uint8)
        boards[0, :2] = 1
        boards[1, :92] = 1
        boards[2, :2] = 1
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = root / "observations.bin"
            write_mnsd(data, seed=11, checksum=b"a" * 32, games=[0, 1, 4], board=boards,
                       scores=[-100, 100, 0], results=[0, 2, 1])
            pst = root / "base.bin"
            weights = np.zeros(13680, dtype=np.int16)
            write_mnpt(pst, weights, weights, initial_piece_values(), 1000)
            for model, feature_count in (("tapered", 13680), ("mirrored", 6840)):
                with self.subTest(model=model):
                    output = root / model
                    argv = ["taper_report", "--data", str(data), "--pst", str(pst),
                            "--model", model, "--output-dir", str(output)]
                    with patch("sys.argv", argv), redirect_stdout(io.StringIO()):
                        report_main()
                    report = json.loads((output / "report.json").read_text())
                    self.assertEqual(report["records"], {"training": 2, "validation": 1})
                    self.assertEqual(report["training_mean_phi"], 0.5)
                    self.assertEqual(report["model"], model)
                    with (output / "features.csv").open() as stream:
                        rows = list(csv.DictReader(stream))
                    self.assertEqual(len(rows), feature_count)
                    self.assertEqual(sum(int(row["count"]) for row in rows), 94)

    def test_report_requires_explicit_model(self) -> None:
        argv = ["taper_report", "--data", "observations.bin", "--pst", "base.bin",
                "--output-dir", "report"]
        with patch("sys.argv", argv), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            report_main()
        self.assertEqual(error.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
