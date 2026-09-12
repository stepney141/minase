"""段階7の識別性診断を、鏡映対の観測を合わせた独立標本で検証する。"""

from contextlib import redirect_stderr, redirect_stdout
import csv
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np

from mnsd import Dataset
from taper import feature_identifiability
from taper_report import main as report_main, state_summary
from test_train_pst import write_mnsd
from train_pst import initial_piece_values, write_mnpt


class MirroredIdentifiabilityTest(unittest.TestCase):
    def test_mirror_pairs_pool_phase_observations_before_computing_ssd(self) -> None:
        # strength-stage7.md: 正準特徴の単位で出現回数とφの偏差平方和を集計する。
        # 同じ段の両端の歩をφ=0とφ=1で1回ずつ観測する。
        boards = np.zeros((2, 144), dtype=np.uint8)
        boards[0, :2] = [1, 12]
        boards[1, :92] = 11
        boards[1, 11] = 1
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "observations.bin"
            write_mnsd(path, seed=0, checksum=b"a" * 32, games=[1, 2], board=boards)
            dataset = Dataset([path])
            indices = np.arange(2, dtype=np.int64)
            full = feature_identifiability(dataset, indices, mirrored=False, batch=1)
            pooled = feature_identifiability(dataset, indices, mirrored=True, batch=1)
        self.assertEqual(full["count"].size, 13680)
        self.assertEqual(pooled["count"].size, 6840)
        self.assertEqual(int(pooled["count"].sum()), 94)
        pawn_full = 29 * 144
        pawn_pooled = 29 * 72
        self.assertEqual(full["ssd"][pawn_full], 0)
        self.assertEqual(full["ssd"][pawn_full + 11], 0)
        self.assertEqual(pooled["count"][pawn_pooled], 2)
        self.assertEqual(pooled["mean_phi"][pawn_pooled], 0.5)
        self.assertEqual(pooled["ssd"][pawn_pooled], 0.5)
        pawn = state_summary(pooled, 72)[29]
        self.assertEqual(pawn["observed_squares"], 1)
        self.assertEqual(pawn["occurrences"], 2)
        self.assertEqual(pawn["mean_phi"], 0.5)

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
