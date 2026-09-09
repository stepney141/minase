"""固定視点の恒等式と局面の等重みを診断集計の契約として検証する。"""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import numpy as np
from features import INITIAL_BOARD
from mnsd import RECORD_DTYPE, write_mnsd
import fm_strength_diagnostics as diagnostics
from fm_strength_diagnostics import grouped_moves, move_summary, static_summary


class DiagnosticTests(unittest.TestCase):
    def test_antisymmetric_evaluator_has_no_turn_component(self):
        # E_black=100, E_white=-100。手番依存がない評価関数の契約。
        rows = [{"eval_pst": 100, "eval_opposite_pst": -100,
                 "eval": 110, "eval_opposite": -110}]
        report = static_summary(rows)
        self.assertEqual(report["pst"]["turn_sum"]["max_abs"], 0)
        self.assertEqual(report["fm"]["turn_sum"]["max_abs"], 0)
        self.assertNotIn("teacher_error", report["fm"])

    def test_tempo_bonus_is_counted_twice(self):
        # 配置値100に手番ボーナス30を双方から足すと130/-70になる。
        rows = [{"eval_pst": 100, "eval_opposite_pst": -100,
                 "eval": 130, "eval_opposite": -70}]
        report = static_summary(rows)
        self.assertEqual(report["correction_turn_sum"]["mean"], 60)

    def test_move_decomposition_contract(self):
        # 配置改善20、手番ボーナス30の交代で合計-40になる。
        move = {"delta": -40, "placement": 20, "turn": -60,
                "delta_pst": 20, "placement_pst": 20, "turn_pst": 0}
        report = move_summary([{"moves": [move]}])
        self.assertEqual(report["correction"]["placement"]["max_abs"], 0)
        self.assertEqual(report["correction"]["turn"]["mean"], -60)
        move["turn"] = 60
        with self.assertRaises(ValueError):
            move_summary([{"moves": [move]}])

    def test_positions_have_equal_weight_despite_move_count(self):
        def move(amount):
            return {"delta": amount, "placement": amount, "turn": 0,
                    "delta_pst": 0, "placement_pst": 0, "turn_pst": 0}
        report = move_summary([{"moves": [move(10)]},
                               {"moves": [move(30), move(30), move(30)]}])
        self.assertEqual(report["correction"]["delta"]["mean"], 25)
        self.assertEqual(report["position_weighted_correction"]["delta"]["mean_abs"]["mean"], 20)

    def test_quiet_excludes_captures_and_promotions(self):
        def move(capture, promote, terminal=False):
            return {"delta": 0, "placement": 0, "turn": 0,
                    "delta_pst": 0, "placement_pst": 0, "turn_pst": 0,
                    "capture": capture, "promote": promote, "terminal": terminal}
        report = grouped_moves([{"moves": [move(False, False), move(True, False),
                                            move(False, True), move(True, True),
                                            move(True, True, True), move(False, False, True)]}])
        self.assertEqual(report["by_kind"]["quiet"]["count"], 1)
        self.assertEqual(report["by_kind"]["tactical"]["count"], 3)
        self.assertEqual(report["by_kind"]["terminal"]["count"], 2)

    def test_empty_band_and_terminal_position_are_explicit(self):
        self.assertEqual(static_summary([]), {"count": 0})
        self.assertEqual(move_summary([{"moves": []}])["count"], 0)


class CommandTests(unittest.TestCase):
    def test_reproducible_sample_and_probe_order_validation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "input.bin"
            records = np.zeros(8, dtype=RECORD_DTYPE)
            records["board"] = INITIAL_BOARD
            records["lion"] = 255
            records["result"] = 1
            records["ply"] = np.arange(8)
            write_mnsd(source, records, seed=1, network_checksum=bytes(32))
            binary = root / "probe"
            weights = root / "pst"
            binary.write_bytes(b"stub")
            weights.write_bytes(b"stub")
            rows = [{"index": i, "eval": 0, "eval_pst": 0,
                     "eval_opposite": 0, "eval_opposite_pst": 0} for i in range(3)]
            reports = []
            for name in ("first", "repeat", "bad-index"):
                output = root / name
                arguments = ["diagnostic", "--positions", str(source), "--pst", str(weights),
                             "--probe", str(binary), "--output-dir", str(output),
                             "--sample-size", "3", "--seed", "1"]
                returned = rows if name != "bad-index" else list(reversed(rows))
                with patch("sys.argv", arguments), patch.object(diagnostics, "probe", return_value=returned), contextlib.redirect_stdout(io.StringIO()):
                    if name == "bad-index":
                        with self.assertRaises(ValueError):
                            diagnostics.main()
                    else:
                        diagnostics.main()
                        reports.append(json.loads((output / "report.json").read_text()))
            self.assertEqual(reports[0]["sample_indices"], reports[1]["sample_indices"])
            self.assertEqual(len(set(reports[0]["sample_indices"])), 3)
            self.assertTrue(all(0 <= i < 8 for i in reports[0]["sample_indices"]))


if __name__ == "__main__":
    unittest.main()
