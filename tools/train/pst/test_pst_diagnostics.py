"""帯別標本、量子化誤差、駒の除去、成りの診断を独立した駒得で検証する。"""

from pathlib import Path
import json
import tempfile
import unittest

import numpy as np

from features import BOARD_FEATURE_COUNT, FEATURE_COUNT, INITIAL_BOARD, feature_indices
from mnsd import Dataset, read_header, map_records
from pst_diagnostics import Weights, derived_piece_values, diagnose
from taper import BAND_COUNT, phase_numerators
from test_train_pst import write_mnsd
from train_pst import initial_piece_values, integer_evaluate, write_mnpt

PIECE_VALUES = initial_piece_values()


def python_probe(mnpt: Path, mnsd: Path, promotions: bool) -> list[dict]:
    """Rustの代わりにPythonの参照評価で応答し、成り手は1件の固定値を返す。"""
    weights = Weights(mnpt)
    records = map_records(mnsd)
    scores = weights.evaluate(records)
    return [
        {"index": index, "eval": int(score),
         **({"promotions": [{"move": "1a1b+", "delta": 7}]} if promotions else {})}
        for index, score in enumerate(scores)
    ]


class DiagnosticsTest(unittest.TestCase):
    """自駒1枚800/8=100cpの基準に対し、倍額の候補と端点の異なる候補を検査する。"""

    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.base = self.root / "base.bin"
        self.candidate = self.root / "candidate.bin"
        self.float_path = self.root / "candidate-float.npz"
        weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
        weights[:BOARD_FEATURE_COUNT // 2] = 800
        weights[BOARD_FEATURE_COUNT // 2:BOARD_FEATURE_COUNT] = -800
        self.weights = weights
        write_mnpt(self.base, weights, weights, PIECE_VALUES, 1000)
        self.write_candidate(weights * 2, weights * 2)

    def write_candidate(self, middlegame: np.ndarray, endgame: np.ndarray) -> None:
        write_mnpt(self.candidate, middlegame, endgame, PIECE_VALUES, 1000)
        with self.float_path.open("wb") as stream:
            np.savez(stream, middlegame=middlegame.astype(np.float32) / 8,
                     endgame=endgame.astype(np.float32) / 8)

    def dataset(self, *, constant_teacher: bool = False) -> Dataset:
        paths = []
        for generation, seed in enumerate((0, 7)):
            path = self.root / f"gen{generation}.bin"
            boards = np.zeros((200, 144), dtype=np.uint8)
            # 自駒の歩を1〜3枚だけ並べる。帯[0.0, 0.2)だけに入る。
            counts = np.arange(200) % 3 + 1
            for index, count in enumerate(counts):
                boards[index, :count] = 1
            scores = [100] * 200 if constant_teacher else (counts * 100).tolist()
            write_mnsd(path, seed=seed, checksum=bytes([generation]) * 32,
                       games=list(range(200)), scores=scores, board=boards)
            paths.append(path)
        return Dataset(paths)

    def run_diagnose(self, dataset: Dataset, output: Path, sample_size: int = 5) -> dict:
        output.mkdir()
        return diagnose(dataset, self.base, self.candidate, self.float_path, output,
                        sample_size, 19, 0.75, python_probe, model_kind="tapered")

    def test_same_saved_samples_compare_both_weights_and_report_empty_bands(self) -> None:
        dataset = self.dataset()
        output = self.root / "first"
        report = self.run_diagnose(dataset, output)
        json.dumps(report, allow_nan=False)
        self.assertEqual(len(report["bands"]), 2 * BAND_COUNT)
        self.assertEqual(report["piece_values"], PIECE_VALUES.tolist())
        for entry in report["bands"]:
            indices = np.load(output / entry["indices_file"], allow_pickle=False)
            if entry["band"] == 0:
                self.assertEqual(len(indices), 5)
                self.assertEqual(len(set(indices.tolist())), 5)
                self.assertTrue(set(indices.tolist()).issubset(set(dataset.validation_indices.tolist())))
                np.testing.assert_array_equal(dataset.generations(indices), entry["generation"])
                self.assertGreater(entry["training_records"], 0)
                self.assertEqual(entry["base"]["mae_raw_cp"], 0)
                # 候補の価値は教師の2倍なので、生の誤差は教師の駒得に等しい。
                expected = float(dataset.gather(indices)["score"].mean())
                self.assertEqual(entry["candidate"]["mae_raw_cp"], expected)
                # 換算誤差は教師Kと出力Kの比で変わるので、有限であることだけを確かめる。
                self.assertTrue(np.isfinite(entry["candidate"]["mae_scaled_cp"]))
                for name in ("base", "candidate"):
                    self.assertAlmostEqual(entry[name]["correlation"], 1)
                    self.assertIsNone(entry[name]["correlation_reason"])
                    self.assertIsNotNone(entry["validation_loss"][name])
            else:
                self.assertEqual(len(indices), 0)
                self.assertEqual(entry["reason"], "empty band")
                self.assertEqual(entry["samples"], 0)
                self.assertIsNone(entry["candidate"])
                self.assertIsNone(entry["validation_loss"]["candidate"])
        self.assertEqual(report["quantization"]["samples"], 10)
        self.assertEqual(report["quantization"]["mean_absolute_error_cp"], 0)
        self.assertEqual(report["rust_agreement"], {"base": 10, "candidate": 10})

        repeated = self.root / "repeated"
        self.assertEqual(report, self.run_diagnose(dataset, repeated))
        for entry in report["bands"]:
            np.testing.assert_array_equal(np.load(output / entry["indices_file"]),
                                          np.load(repeated / entry["indices_file"]))

        # 代表局面は初期配置と、標本のある帯ごとに最小の通算番号を持つ局面である。
        labels = [entry["label"] for entry in report["representatives"]]
        self.assertEqual(labels, ["initial", "band0"])
        smallest = min(int(np.load(output / entry["indices_file"]).min())
                       for entry in report["bands"] if entry["samples"])
        self.assertEqual(report["representatives"][1]["index"], smallest)
        header = read_header(output / "representatives.bin")
        self.assertEqual(header.record_count, 2)
        initial = report["representatives"][0]
        self.assertEqual(initial["piece_count"], 92)
        self.assertEqual(initial["phase_numerator"], 90)
        self.assertEqual(initial["evaluations"], {"base": 0, "candidate": 0})
        self.assertEqual(len(initial["removals"]), 90)
        for item in initial["removals"]:
            self.assertNotIn(item["state"], (11, 21))
            sign = -1 if item["relative_color"] == 0 else 1
            # 先手の駒を除くと駒数も減るが、両端点が同一なので評価差は駒価値だけである。
            self.assertEqual(item["delta_cp"], {"base": sign * 100, "candidate": sign * 200})
        self.assertEqual(initial["promotions"]["candidate"], [{"move": "1a1b+", "delta": 7}])
        self.assertIsNone(initial["promotion_reason"])
        derived = report["derived_piece_values"]
        self.assertEqual(derived["fixed"], [int(PIECE_VALUES[s]) for s in derived["states"]])
        self.assertEqual(derived["base_middlegame"], [100] * len(derived["states"]))
        self.assertEqual(derived["candidate_endgame"], [200] * len(derived["states"]))

    def test_differing_endpoints_change_removal_deltas_with_the_phase(self) -> None:
        middlegame = self.weights * 2
        endgame = self.weights.copy()
        self.write_candidate(middlegame, endgame)
        dataset = self.dataset()
        report = self.run_diagnose(dataset, self.root / "tapered")
        initial = report["representatives"][0]
        # 初期配置(q=90)は序中盤側だけで評価され、1枚除くと q=89 になる。
        first_removal = initial["removals"][0]
        variant = INITIAL_BOARD.copy()
        variant[first_removal["square"]] = 0
        features = feature_indices(np.stack([INITIAL_BOARD, variant]), np.zeros(2, dtype=np.uint8),
                                   np.full(2, 255, dtype=np.uint8))
        scores = integer_evaluate(middlegame, endgame, features, phase_numerators(np.stack([INITIAL_BOARD, variant])))
        self.assertEqual(first_removal["delta_cp"]["candidate"], int(scores[1] - scores[0]))
        self.assertEqual(report["derived_piece_values"]["candidate_middlegame"][0], 200)
        self.assertEqual(report["derived_piece_values"]["candidate_endgame"][0], 100)
        # 端点が異なると補間後の浮動小数点評価は整数にならないが、切り捨て誤差は上限内に収まる。
        self.assertGreater(report["quantization"]["mean_absolute_error_cp"], 0)
        self.assertLessEqual(report["quantization"]["mean_absolute_error_cp"], 2)

    def test_undefined_correlations_and_mismatched_probe_are_reported(self) -> None:
        for constant_teacher, sample_size in ((True, 5), (False, 1), (False, 5)):
            with self.subTest(constant_teacher=constant_teacher, sample_size=sample_size):
                dataset = self.dataset(constant_teacher=constant_teacher)
                if not constant_teacher and sample_size == 5:
                    zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
                    self.write_candidate(zeros, zeros)
                report = self.run_diagnose(dataset, self.root / f"output-{constant_teacher}-{sample_size}", sample_size)
                json.dumps(report, allow_nan=False)
                for entry in report["bands"]:
                    if entry["samples"]:
                        self.assertIsNone(entry["candidate"]["correlation"])
                        self.assertTrue(entry["candidate"]["correlation_reason"])
                        self.assertGreaterEqual(entry["candidate"]["mae_scaled_cp"], 0)

        def wrong_probe(mnpt: Path, mnsd: Path, promotions: bool) -> list[dict]:
            return [{**item, "eval": item["eval"] + 1} for item in python_probe(mnpt, mnsd, promotions)]

        output = self.root / "mismatch"
        output.mkdir()
        with self.assertRaises(ValueError):
            diagnose(self.dataset(), self.base, self.candidate, self.float_path, output, 5, 19, 0.75, wrong_probe, model_kind="tapered")

    def test_rejects_changed_piece_values_and_quantization_drift(self) -> None:
        dataset = self.dataset()
        values = PIECE_VALUES.copy()
        values[29] = 101
        values[11] = values[21] = 2601
        write_mnpt(self.candidate, self.weights, self.weights, values, 1000)
        output = self.root / "values"
        output.mkdir()
        with self.assertRaises(ValueError):
            diagnose(dataset, self.base, self.candidate, self.float_path, output, 5, 19, 0.75, python_probe, model_kind="tapered")
        write_mnpt(self.candidate, self.weights, self.weights, PIECE_VALUES, 1000)
        with self.float_path.open("wb") as stream:
            np.savez(stream, middlegame=self.weights.astype(np.float32) / 8 + 3,
                     endgame=self.weights.astype(np.float32) / 8 + 3)
        drift = self.root / "drift"
        drift.mkdir()
        with self.assertRaises(ValueError):
            diagnose(dataset, self.base, self.candidate, self.float_path, drift, 5, 19, 0.75, python_probe, model_kind="tapered")

    def test_derived_values_round_half_away_from_zero(self) -> None:
        weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
        weights[:144] = 8  # own state 0: 1152/2304 = 0.5 → 1
        weights[47 * 144:48 * 144] = 0
        self.assertEqual(derived_piece_values(weights)[0], 1)
        weights[:144] = -8
        self.assertEqual(derived_piece_values(weights)[0], -1)
        weights[:144] = 7
        self.assertEqual(derived_piece_values(weights)[0], 0)


class FMDiagnosticsTest(unittest.TestCase):
    """候補の参照評価と駒除去を、1組1cpの既知のFMで確認する。"""

    setUp = DiagnosticsTest.setUp
    write_candidate = DiagnosticsTest.write_candidate
    dataset = DiagnosticsTest.dataset

    def test_python_fm_diagnostics_never_send_v3_to_rust(self) -> None:
        from train_fm import write_mnpt_v3
        write_mnpt_v3(self.candidate, self.weights, self.weights, PIECE_VALUES, 1000,
                      np.full((FEATURE_COUNT, 1), 64, dtype=np.int16), np.array([1], dtype=np.int8), 6)
        np.savez(self.float_path, V=np.ones((FEATURE_COUNT, 1)), a=np.array([0.001]),
                 mask=np.ones(FEATURE_COUNT, dtype=bool))
        calls = []
        def baseline_probe(mnpt, mnsd, promotions):
            self.assertEqual(mnpt, self.base)
            calls.append(promotions)
            return python_probe(mnpt, mnsd, promotions)
        output = self.root / "fm"
        output.mkdir()
        dataset = self.dataset()
        report = diagnose(dataset, self.base, self.candidate, self.float_path, output,
                          5, 19, 0.75, baseline_probe, model_kind="fm")
        json.dumps(report, allow_nan=False)
        self.assertIn(False, calls)
        self.assertIn(True, calls)
        self.assertEqual(report["rust_agreement"]["candidate"]["status"], "未実施（第2フェーズ）")
        self.assertEqual(report["move_deltas"]["status"], "未実施（第2フェーズ）")
        self.assertLess(report["quantization"]["max_absolute_error_cp"], 1e-8)
        for entry in report["bands"]:
            if not entry["samples"]:
                continue
            indices = np.load(output / entry["indices_file"])
            count = np.count_nonzero(dataset.gather(indices)["board"], axis=1)
            expected_mae = float((count * (count - 1) / 2).mean())
            self.assertEqual(entry["candidate"]["mae_raw_cp"], expected_mae)
            self.assertIsNotNone(entry["validation_loss"]["candidate"])
        initial = report["representatives"][0]
        self.assertEqual(initial["fm_correction_cp"], 92 * 91 // 2)
        for item in initial["removals"]:
            self.assertEqual(item["fm_delta_cp"], -91)
        for representative in report["representatives"]:
            self.assertEqual(representative["promotions"]["candidate"]["status"], "未実施（第2フェーズ）")


if __name__ == "__main__":
    unittest.main()
