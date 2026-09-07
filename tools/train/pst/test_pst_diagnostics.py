"""教師値との比較、標本の保存、駒除去の符号を独立した駒得で検証する。"""

from pathlib import Path
import json
import tempfile
import unittest

import numpy as np

from features import BOARD_FEATURE_COUNT, FEATURE_COUNT
from mnsd import Dataset
from pst_diagnostics import diagnose
from test_train_pst import write_mnsd
from train_pst import write_mnpt


class DiagnosticsTest(unittest.TestCase):
    """1枚100cpの駒得を基準に、倍額の候補と退化した標本を検査する。"""

    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.base = self.root / "base.bin"
        self.candidate = self.root / "candidate.bin"
        weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
        weights[:BOARD_FEATURE_COUNT // 2] = 800
        weights[BOARD_FEATURE_COUNT // 2:BOARD_FEATURE_COUNT] = -800
        write_mnpt(self.base, weights, 1000)
        write_mnpt(self.candidate, weights * 2, 1000)

    def dataset(self, *, constant_teacher: bool = False) -> Dataset:
        paths = []
        for generation, seed in enumerate((0, 7)):
            path = self.root / f"gen{generation}.bin"
            boards = np.zeros((200, 144), dtype=np.uint8)
            counts = np.arange(200) % 3 + 1
            for index, count in enumerate(counts):
                boards[index, :count] = 1
            scores = [100] * 200 if constant_teacher else (counts * 100).tolist()
            write_mnsd(path, seed=seed, checksum=bytes([generation]) * 32,
                       games=list(range(200)), scores=scores, board=boards)
            paths.append(path)
        return Dataset(paths)

    def test_same_saved_validation_sample_compares_both_weights(self) -> None:
        dataset = self.dataset()
        report = diagnose(dataset, self.base, self.candidate, self.root, 5, 19)
        json.dumps(report, allow_nan=False)
        self.assertEqual(len(report["generations"]), 2)
        for result in report["generations"]:
            indices = np.load(self.root / result["indices_file"], allow_pickle=False)
            self.assertEqual(len(indices), 5)
            self.assertEqual(len(set(indices)), 5)
            self.assertTrue(set(indices).issubset(set(dataset.validation_indices)))
            np.testing.assert_array_equal(dataset.generations(indices), result["generation"])
            self.assertEqual(result["base"]["mae_cp"], 0)
            # 候補の価値が教師の2倍なので、誤差は教師の駒得に等しい。
            expected_error = float(dataset.gather(indices)["score"].mean())
            self.assertEqual(result["candidate"]["mae_cp"], expected_error)
            for name in ("base", "candidate"):
                self.assertAlmostEqual(result[name]["correlation"], 1)
                self.assertIsNone(result[name]["correlation_reason"])
        repeated = self.root / "repeated"
        repeated.mkdir()
        self.assertEqual(report, diagnose(dataset, self.base, self.candidate, repeated, 5, 19))
        for result in report["generations"]:
            np.testing.assert_array_equal(
                np.load(self.root / result["indices_file"]),
                np.load(repeated / result["indices_file"]),
            )
        # 初期配置は先後46枚ずつ。先手の1枚除去で駒価値だけ減る。
        for name, value in (("base", 100), ("candidate", 200)):
            material = report["material"][name]
            self.assertEqual(material["initial_cp"], 0)
            self.assertEqual(len(material["removals"]), 46)
            self.assertEqual(len({item["square"] for item in material["removals"]}), 46)
            for item in material["removals"]:
                self.assertTrue(1 <= item["piece_byte"] < 65)
                self.assertEqual(item["delta_cp"], -value)

    def test_undefined_correlations_remain_valid_json_with_reasons(self) -> None:
        for constant_teacher, sample_size in ((True, 5), (False, 1), (False, 5)):
            with self.subTest(constant_teacher=constant_teacher, sample_size=sample_size):
                dataset = self.dataset(constant_teacher=constant_teacher)
                if not constant_teacher and sample_size == 5:
                    write_mnpt(self.candidate, np.zeros(FEATURE_COUNT, dtype=np.int16), 1000)
                output = self.root / f"output-{constant_teacher}-{sample_size}"
                output.mkdir()
                report = diagnose(dataset, self.base, self.candidate, output, sample_size, 19)
                json.dumps(report, allow_nan=False)
                for result in report["generations"]:
                    self.assertIsNone(result["candidate"]["correlation"])
                    self.assertTrue(result["candidate"]["correlation_reason"])
                    self.assertGreaterEqual(result["candidate"]["mae_cp"], 0)

    def test_rejects_nonpositive_sample_and_empty_validation_generation(self) -> None:
        dataset = self.dataset()
        for sample_size in (0, -1):
            with self.assertRaises(ValueError):
                diagnose(dataset, self.base, self.candidate, self.root, sample_size, 19)
        # seed=1, game=0は訓練用であり、この世代には検証局面がない。
        path = self.root / "no-validation.bin"
        write_mnsd(path, seed=1, checksum=b"z" * 32, games=[0])
        with self.assertRaises(ValueError):
            diagnose(Dataset([path]), self.base, self.candidate, self.root, 5, 19)
        self.assertEqual(list(self.root.glob("*.npy")), [])


if __name__ == "__main__":
    unittest.main()
