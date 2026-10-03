"""基準PSTと候補PSTの評価、駒除去、および成りの比較を検証する。"""

from __future__ import annotations

import hashlib
import json
import math
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import numpy as np

from helpers import PIECE_VALUES, python_probe, write_mnsd, write_provenance
from minase_train.data.features import BOARD_FEATURE_COUNT, FEATURE_COUNT
from minase_train.data.mnpt import initial_piece_values, write_mnpt
from minase_train.data.mnsd import Dataset, read_header
from minase_train.data.taper import BAND_COUNT
from minase_train.diagnostics.comparison import (
    Weights,
    derived_piece_values,
    diagnose,
    outcome_metrics,
)


class OutcomeMetricsTest(unittest.TestCase):

    def test_hand_computed_means_signs_and_cross_file_game_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            ids = [str(i) for i in range(200)
                   if int.from_bytes(hashlib.sha256(str(i).encode()).digest()[:8], 'little') % 20 == 0]
            train_id = next(str(i) for i in range(200)
                            if int.from_bytes(hashlib.sha256(str(i).encode()).digest()[:8], 'little') % 20 != 0)
            paths = []
            for file, (squares, results, games) in enumerate((
                    ([0, 1, 4, 0], [2, 0, 2, 0], [1, 1, 2, 3]),
                    ([2, 3, 5], [0, 1, 1], [1, 1, 2]))):
                boards = np.zeros((len(squares), 144), dtype='u1')
                boards[np.arange(len(squares)), squares] = 1
                path = root / f'{file}.bin'
                write_mnsd(path, seed=file, checksum=b'a' * 32, games=games,
                           results=results, board=boards)
                write_provenance(path, result_origin='human', **{'lambda': 0, 'games': [
                    {'game': 1, 'id': ids[0]}, {'game': 2, 'id': ids[1]},
                    {'game': 3, 'id': train_id}]})
                paths.append(path)
            weights = np.zeros(FEATURE_COUNT, dtype='i2')
            # 未成の歩は駒状態29。1枚で±100cpまたは0cpにする。
            weights[29 * 144:29 * 144 + 6] = [800, -800, 800, 0, 0, -800]
            mnpt = root / 'pst.bin'
            write_mnpt(mnpt, weights, weights, initial_piece_values(), 100)
            dataset = Dataset(paths, lambda_override=1)
            # バッチ境界とファイル境界を越えて同じ対局にまとめる。
            with patch('minase_train.diagnostics.comparison.BATCH', 2):
                report = outcome_metrics(dataset, {'base': Weights(mnpt)})['base']
            a = math.log(1 + math.exp(-1))
            b = math.log(1 + math.exp(1))
            c = (a + b) / 2
            self.assertAlmostEqual(report['bce_position_mean'], (2*a+b+2*math.log(2)+c)/6)
            self.assertAlmostEqual(report['bce_game_mean'], ((2*a+b+math.log(2))/4 + (math.log(2)+c)/2)/2)
            self.assertEqual(report['records'], 6)
            self.assertEqual(report['games'], 2)
            self.assertEqual(report['sign_agreement'], 2/3)
            self.assertEqual(report['sign_matches'], 2)
            self.assertEqual(report['sign_records'], 3)
            self.assertEqual(report['sign_excluded'], 3)
            self.assertEqual(report['draw_records'], 2)
            self.assertEqual(report['zero_score_records'], 2)
            self.assertEqual(report['output_k'], 100)
            json.dumps(report, allow_nan=False)
            unchanged = outcome_metrics(Dataset(paths), {'base': Weights(mnpt)})['base']
            for key in report:
                if isinstance(report[key], float):
                    self.assertAlmostEqual(report[key], unchanged[key])
                else:
                    self.assertEqual(report[key], unchanged[key])
            weights[:] = 0
            write_mnpt(root / 'zero.bin', weights, weights, initial_piece_values(), 100)
            empty = outcome_metrics(dataset, {'zero': Weights(root / 'zero.bin')})['zero']
            self.assertIsNone(empty['sign_agreement'])
            self.assertEqual(empty['sign_excluded'], 6)
            self.assertAlmostEqual(empty['bce_position_mean'], math.log(2))


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
                        sample_size, 19, python_probe)

    def test_zero_lambda_omits_teacher_comparisons_but_keeps_other_diagnostics(self):
        original = self.dataset()
        for path in original.paths:
            write_provenance(path, **{"lambda": 0})
        dataset = Dataset(original.paths)
        with patch('minase_train.pst.teacher.estimate_k', side_effect=AssertionError("unexpected K estimate")):
            report = self.run_diagnose(dataset, self.root / "results-only")
        json.dumps(report, allow_nan=False)
        self.assertEqual(report["teacher_ks"], [None, None])
        self.assertEqual(report["teacher_comparison_excluded"], len(dataset.validation_indices))
        self.assertGreater(report["quantization"]["samples"], 0)
        for entry in report["bands"]:
            self.assertIsNone(entry["candidate"])
            self.assertIsNone(entry["base"])
            if entry["samples"]:
                self.assertEqual(entry["teacher_comparison_excluded"], entry["samples"])
                self.assertIsNotNone(entry["validation_loss"]["candidate"])

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
        self.assertEqual(report["rust_promotion_agreement"], {"base": 2, "candidate": 2})

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
        self.assertEqual(initial["promotions"]["candidate"][0]["delta"], 200)
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
        # docs/research/tapered-pst.mdの補間式より、除去後の1枚の価値は
        # (89×200 + 1×100)/90 = 198.88… cp。整数評価では0方向へ切り捨てる。
        for color, expected in ((0, -198), (1, 198)):
            removal = next(item for item in initial["removals"] if item["relative_color"] == color)
            self.assertEqual(removal["delta_cp"]["candidate"], expected)
        self.assertEqual(report["derived_piece_values"]["candidate_middlegame"][0], 200)
        self.assertEqual(report["derived_piece_values"]["candidate_endgame"][0], 100)
        # 成り後の固定局面は駒が1枚なのでq=0。着手前のq=90を流用してはならない。
        self.assertEqual(initial["promotions"]["candidate"][0]["delta"], 100)
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
            diagnose(self.dataset(), self.base, self.candidate, self.float_path, output, 5, 19, wrong_probe)

    def test_rejects_changed_piece_values_and_quantization_drift(self) -> None:
        dataset = self.dataset()
        values = PIECE_VALUES.copy()
        values[29] = 101
        values[11] = values[21] = 2601
        write_mnpt(self.candidate, self.weights, self.weights, values, 1000)
        output = self.root / "values"
        output.mkdir()
        with self.assertRaises(ValueError):
            diagnose(dataset, self.base, self.candidate, self.float_path, output, 5, 19, python_probe)
        write_mnpt(self.candidate, self.weights, self.weights, PIECE_VALUES, 1000)
        with self.float_path.open("wb") as stream:
            np.savez(stream, middlegame=self.weights.astype(np.float32) / 8 + 3,
                     endgame=self.weights.astype(np.float32) / 8 + 3)
        drift = self.root / "drift"
        drift.mkdir()
        with self.assertRaises(ValueError):
            diagnose(dataset, self.base, self.candidate, self.float_path, drift, 5, 19, python_probe)

    def test_promotion_delta_mismatch_is_rejected_when_before_evaluations_agree(self) -> None:
        """strength-stage7.md: 成りの評価差もRustとPython整数評価で一致しなければ停止する。"""
        dataset = self.dataset()
        for incorrect_model in (self.base, self.candidate):
            def wrong_promotion(mnpt: Path, mnsd: Path, promotions: bool) -> list[dict]:
                rows = python_probe(mnpt, mnsd, promotions)
                if promotions and mnpt == incorrect_model:
                    rows[0]["promotions"][0]["delta"] += 1
                return rows

            with self.subTest(model=incorrect_model.name):
                output = self.root / f"wrong-promotion-{incorrect_model.stem}"
                output.mkdir()
                with self.assertRaises(ValueError):
                    diagnose(dataset, self.base, self.candidate, self.float_path, output,
                             5, 19, wrong_promotion)

    def test_pst_removal_sign_reversal_is_rejected(self) -> None:
        self.write_candidate(-self.weights, -self.weights)
        with self.assertRaisesRegex(ValueError, "PST removal delta"):
            self.run_diagnose(self.dataset(), self.root / "reversed")

    def test_derived_values_round_half_away_from_zero(self) -> None:
        weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
        weights[:144] = 8  # own state 0: 1152/2304 = 0.5 → 1
        weights[47 * 144:48 * 144] = 0
        self.assertEqual(derived_piece_values(weights, weights, 0.5)[0], 1)
        weights[:144] = -8
        self.assertEqual(derived_piece_values(weights, weights, 0.5)[0], -1)
        weights[:144] = 7
        self.assertEqual(derived_piece_values(weights, weights, 0.5)[0], 0)

    def test_derived_values_blend_endpoints_before_rounding_and_set_royal_values(self) -> None:
        # strength-stage7.md: 平均φで合成後に駒価値を導き、王駒は最大非王駒+未成歩。
        for phi, expected in ((0.0, 200), (0.25, 175), (1.0, 100)):
            with self.subTest(phi=phi):
                values = derived_piece_values(self.weights, self.weights * 2, phi)
                self.assertEqual(len(values), 47)
                self.assertEqual(values[29], expected)
                self.assertEqual(values[10], expected)
                self.assertEqual(values[11], expected * 2)
                self.assertEqual(values[21], expected * 2)
        # 端点を先に丸めると平均0.5cpから1になるが、合成した0.4375cpは0へ丸める。
        middlegame = np.zeros(FEATURE_COUNT, dtype=np.int16)
        endgame = middlegame.copy()
        middlegame[:144] = 9
        endgame[:144] = 5
        self.assertEqual(derived_piece_values(middlegame, endgame, 0.5)[0], 0)

    def test_royal_values_exclude_unreachable_states_and_use_unpromoted_pawn(self) -> None:
        middlegame = self.weights.copy()
        endgame = self.weights.copy()
        # 到達不能な歩の成り状態と王の静的重みが、到達可能な最大非王駒を上回る。
        for state, mg_cp, eg_cp in ((0, 3000, 3000), (11, 4000, 4000),
                                    (20, 400, 800), (29, 50, 100)):
            for weights, value in ((middlegame, mg_cp), (endgame, eg_cp)):
                weights[state * 144:(state + 1) * 144] = value * 8
                weights[(47 + state) * 144:(48 + state) * 144] = -value * 8
        values = derived_piece_values(middlegame, endgame, 0.5)
        self.assertEqual(values[0], 3000)
        self.assertEqual(values[20], 600)
        self.assertEqual(values[29], 75)
        self.assertEqual(values[11], 675)
        self.assertEqual(values[21], 675)

    def test_derived_values_reject_invalid_mean_phase(self) -> None:
        for phi in (-0.01, 1.01, float("nan"), float("inf")):
            with self.subTest(phi=phi), self.assertRaises(ValueError):
                derived_piece_values(self.weights, self.weights, phi)


if __name__ == "__main__":
    unittest.main()
