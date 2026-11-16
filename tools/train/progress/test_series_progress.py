"""設計書 training-series-records とフェーズ2・4指示書の契約を検証する。

収集は列・値・並び順・欠測の保持と入力の拒否を、作図はartistの種類・座標・
色・透明度を検証する。得点率の境界は0.1と0.9、設定変更の位置は直前の累計を
正解とする。期待値は以下の小さな入力から手で定め、実データや実装の出力を使わない。
CLIだけはプロセスを起動し、リポジトリの根からのパス解決とPNGへの出力を確認する。
"""

import copy
import csv
import json
import math
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from matplotlib.collections import LineCollection
from matplotlib.lines import Line2D
from matplotlib.patches import FancyArrowPatch
from matplotlib.text import Annotation
from matplotlib.ticker import PercentFormatter

import series_progress as progress


# 列名と順序は指示書にあるファイル形式の契約を独立に記す。
STRENGTH_COLUMNS = (
    "cycle", "opponent", "opponent_limit", "condition", "rules", "max_ply",
    "run_name", "network_sha256", "attempted_pairs", "valid_pairs", "discarded_pairs",
    "pentanomial_0", "pentanomial_1", "pentanomial_2", "pentanomial_3", "pentanomial_4",
    "score_rate", "elo", "elo_lower", "elo_upper", "elo_without_move_forfeits",
    "affected_pairs", "affected_pair_fraction", "elo_discarded_as_losses",
    "elo_discarded_as_wins",
)
VALIDATION_COLUMNS = (
    "cycle", "data_sha256", "game_count", "position_count", "value_result",
    "policy_agreement", "value_result_1_40", "policy_agreement_1_40",
    "value_result_41_100", "policy_agreement_41_100", "value_result_101_200",
    "policy_agreement_101_200", "value_result_201_up", "policy_agreement_201_up",
)
CYCLE_COLUMNS = (
    "cycle", "training_positions", "cumulative_training_positions", "games", "plies",
    "draws", "max_ply_games", "resigned_games", "train_policy", "train_value_result",
    "train_legal_probability_mass", "validation_policy", "validation_value_result",
    "validation_legal_probability_mass",
)

CYCLE_SETTING_COLUMNS = (
    "setting_lr", "setting_batch_size", "setting_momentum", "setting_weight_decay",
    "setting_clip_grad_norm", "setting_value_lambda", "setting_precision",
    "setting_simulations", "setting_target_positions", "setting_start_position",
    "setting_replay_buffer_games",
)


def cycle_record(cycle=1):
    """フェーズ4の対応表に必要な欄を持つ、小さなサイクルの記録を作る。"""
    return {
        "cycle": cycle, "lr": 0.04, "replay_buffer_games": 12000,
        "generation": {"simulations": 800, "target_positions": 100,
                       "start_position": "initial"},
        "selfplay_summary": {"training_positions": 120, "games": 10, "plies": 200,
                             "draws": 5, "draw_reason_max_ply": 2,
                             "win_reason_resignation": 1},
        "training_summary": {
            "config": {"batch_size": 1024, "momentum": 0.9, "weight_decay": 0.0001,
                       "clip_grad_norm": 5.0, "value_lambda": 0.5, "precision": "bf16"},
            "diagnostics": {
                "latest_training_sample": {"policy": 2, "value_result": 0.6,
                                           "legal_probability_mass": 0.8},
                "validation": {"policy": 2.2, "value_result": 0.7,
                               "legal_probability_mass": 0.9},
                "training_sample": {"policy": 9, "value_result": 8,
                                    "legal_probability_mass": 0.1},
            },
        },
    }


def summary(run_name="run-a"):
    """1勝1敗のペア2件と破棄1件を持つ集計を作る。"""
    return {
        "run_name": run_name,
        "baseline": {"kind": "alphabeta", "limit": {"nodes": 500}},
        "rules_source": "lishogi", "max_ply": 100,
        "network_sha256": "a" * 64,
        "attempted_pairs": 3, "valid_pairs": 2, "discarded_pairs": 1,
        "valid": {"pentanomial": [0, 0, 2, 0, 0], "score_rate": 0.5,
                  "elo": {"elo": 0.0, "lower": -20.0, "upper": 30.0}},
        "without_move_forfeits": {"pentanomial": [0, 0, 1, 0, 0],
                                  "elo": {"elo": 0.0}},
        "affected_pairs": 2, "affected_pair_fraction": 2 / 3,
        "discarded_as_losses": {"pentanomial": [1, 0, 2, 0, 0],
                                "elo": {"elo": -120.0}},
        "discarded_as_wins": {"pentanomial": [0, 0, 2, 0, 1],
                              "elo": {"elo": 120.0}},
    }


def diagnostic():
    """201手目以降に局面がない診断を作る。"""
    return {
        "data": {"sha256": "b" * 64},
        "diagnostics": {"game_count": 2, "position_count": 6,
                        "value_result": 0.7, "policy_agreement": 0.25},
        "bands": [
            {"first_move": 1, "last_move": 40, "game_count": 2,
             "position_count": 2, "value_result": 0.8, "policy_agreement": 0.0},
            {"first_move": 41, "last_move": 100, "game_count": 2,
             "position_count": 2, "value_result": 0.7, "policy_agreement": 0.25},
            {"first_move": 101, "last_move": 200, "game_count": 2,
             "position_count": 2, "value_result": 0.6, "policy_agreement": 0.5},
            {"first_move": 201, "last_move": None, "game_count": 0,
             "position_count": 0, "value_result": None, "policy_agreement": None},
        ],
    }


def strength_row(cycle=1, **changes):
    row = dict(zip(STRENGTH_COLUMNS, (
        cycle, "alphabeta", "nodes=500", "standard", "lishogi", 100,
        "run-a", "a" * 64, 3, 2, 1, 0, 0, 2, 0, 0,
        0.5, 0.0, -20.0, 30.0, 0.0, 2, 2 / 3, -120.0, 120.0,
    ), strict=True))
    row.update(changes)
    return row


def validation_row(cycle=0, **changes):
    row = dict(zip(VALIDATION_COLUMNS, (
        cycle, "b" * 64, 2, 6, 0.7, 0.25, 0.8, 0.0, 0.7, 0.25,
        0.6, 0.5, "", "",
    ), strict=True))
    row.update(changes)
    return row


def cycle_rows():
    """累計は1、3、6百万局面とし、各比率を暗算できる値にする。"""
    return [dict(zip(CYCLE_COLUMNS, values, strict=True)) for values in (
        (1, 1_000_000, 1_000_000, 10, 100, 1, 2, 3, 2.0, 0.6, 0.8, 2.2, 0.7, 0.9),
        (2, 2_000_000, 3_000_000, 20, 400, 2, 4, 6, 1.9, 0.5, 0.9, 2.1, 0.6, 0.9),
        (3, 3_000_000, 6_000_000, 40, 1200, 4, 8, 12, 1.8, 0.4, 0.9, 2.0, 0.5, 0.9),
    )]


def write_csv(path, columns, rows):
    with path.open("w", encoding="utf-8", newline="") as output:
        writer = csv.DictWriter(output, fieldnames=columns)
        writer.writeheader()
        writer.writerows(rows)


def artists(figure, prefix, suffix=""):
    return [artist for artist in figure.findobj()
            if artist.get_gid() is not None
            and artist.get_gid().startswith(prefix)
            and artist.get_gid().endswith(suffix)]


class ProgressTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.series = self.root / "docs" / "training" / "main"
        self.series.mkdir(parents=True)
        self.summary_path = self.root / "summary.json"
        self.diagnostic_path = self.root / "diagnostic.json"
        self.record = summary()
        self.diagnosis = diagnostic()
        self.write_sources()
        self.write_definition([(1, "run-a", "standard")], [(0, "diagnostic.json")])

    def write_sources(self):
        self.summary_path.write_text(json.dumps([self.record]), encoding="utf-8")
        self.diagnostic_path.write_text(json.dumps(self.diagnosis), encoding="utf-8")

    def write_definition(self, strengths, validations):
        parts = []
        for cycle, name, condition in strengths:
            parts.append(f'[[strength]]\ncycle = {cycle}\nsummary = "summary.json"\n'
                         f'run_name = "{name}"\ncondition = "{condition}"\n')
        for cycle, path in validations:
            parts.append(f'[[validation]]\ncycle = {cycle}\npath = "{path}"\n')
        (self.series / "series.toml").write_text("\n".join(parts), encoding="utf-8")

    def read_csv(self, name):
        with (self.series / name).open(encoding="utf-8", newline="") as source:
            reader = csv.reader(source)
            return next(reader), list(reader)

    def prepare_csv(self, *, strength=None, validation=None, cycles=None,
                    directory=None, settings=()):
        if directory is None:
            directory = self.series
        directory.mkdir(parents=True, exist_ok=True)
        write_csv(directory / "cycles.csv", CYCLE_COLUMNS + tuple(settings),
                  cycle_rows() if cycles is None else cycles)
        write_csv(directory / "strength.csv", STRENGTH_COLUMNS,
                  [strength_row()] if strength is None else strength)
        write_csv(directory / "validation.csv", VALIDATION_COLUMNS,
                  [validation_row()] if validation is None else validation)
        return directory

    def assert_collect_error(self, filename, field):
        with self.assertRaises(ValueError) as caught:
            progress.collect(self.series, self.root)
        self.assertIn(filename, str(caught.exception))
        self.assertIn(field, str(caught.exception))

    def assert_read_error(self, filename, field):
        with self.assertRaises(ValueError) as caught:
            progress.read_series(self.series)
        self.assertIn(filename, str(caught.exception))
        self.assertIn(field, str(caught.exception))

    def test_collect_columns_values_and_overwrite_preserves_cycles(self):
        """CSVの全列を仕様の順と値で書き、cycles.csvを変更しない。"""
        cycles = self.series / "cycles.csv"
        cycles.write_bytes(b"untouched\n")
        (self.series / "strength.csv").write_text("old", encoding="utf-8")
        (self.series / "validation.csv").write_text("old", encoding="utf-8")
        progress.collect(self.series, self.root)
        for name in ("strength.csv", "validation.csv"):
            with self.subTest(filename=name):
                self.assertNotIn(b"\r", (self.series / name).read_bytes())
        columns, rows = self.read_csv("strength.csv")
        self.assertEqual(columns, list(STRENGTH_COLUMNS))
        self.assertEqual(rows, [[
            "1", "alphabeta", "nodes=500", "standard", "lishogi", "100", "run-a",
            "a" * 64, "3", "2", "1", "0", "0", "2", "0", "0", "0.5", "0.0",
            "-20.0", "30.0", "0.0", "2", str(2 / 3), "-120.0", "120.0",
        ]])
        columns, rows = self.read_csv("validation.csv")
        self.assertEqual(columns, list(VALIDATION_COLUMNS))
        self.assertEqual(rows, [["0", "b" * 64, "2", "6", "0.7", "0.25", "0.8",
                                 "0.0", "0.7", "0.25", "0.6", "0.5", "", ""]])
        self.assertEqual(cycles.read_bytes(), b"untouched\n")

    def write_cycles_record(self, records):
        path = self.root / "cycles.jsonl"
        path.write_text("".join(json.dumps(row) + "\n" for row in records), encoding="utf-8")
        return path

    def assert_cycle_record_error_preserves_csv(self, path, field, line):
        previous = {}
        for name in ("cycles.csv", "strength.csv", "validation.csv"):
            previous[name] = f"previous {name}\n".encode()
            (self.series / name).write_bytes(previous[name])
        with self.assertRaises(ValueError) as caught:
            progress.collect(self.series, self.root, cycles_record=path)
        message = str(caught.exception)
        self.assertIn(str(path), message)
        self.assertIn(f":{line}", message)
        self.assertIn(field, message)
        for name, data in previous.items():
            self.assertEqual((self.series / name).read_bytes(), data)

    def test_collect_cycles_record_columns_and_values(self):
        """対応表から手で定めた全列を比較し、累計・引き算・数値表記を守る。"""
        first, second = cycle_record(), cycle_record(2)
        second["selfplay_summary"].update(training_positions=80, games=12, plies=300,
                                           draws=4, draw_reason_max_ply=3,
                                           win_reason_resignation=2)
        second["lr"] = 0.02
        second["training_summary"]["diagnostics"]["latest_training_sample"].update(
            policy=1.8, value_result=0.4, legal_probability_mass=1.0)
        second["training_summary"]["diagnostics"]["validation"].update(
            policy=2.0, value_result=0.5, legal_probability_mass=0.95)
        path = self.write_cycles_record([first, second])
        progress.collect(self.series, self.root, cycles_record=path)
        columns, rows = self.read_csv("cycles.csv")
        self.assertEqual(columns, list(CYCLE_COLUMNS + CYCLE_SETTING_COLUMNS))
        settings = ["1024", "0.9", "0.0001", "5.0", "0.5", "bf16", "800", "100",
                    "initial", "12000"]
        self.assertEqual(rows, [
            ["1", "120", "120", "10", "200", "3", "2", "1", "2", "0.6", "0.8",
             "2.2", "0.7", "0.9", "0.04", *settings],
            ["2", "80", "200", "12", "300", "1", "3", "2", "1.8", "0.4", "1.0",
             "2.0", "0.5", "0.95", "0.02", *settings],
        ])
        self.assertNotIn(b"\r", (self.series / "cycles.csv").read_bytes())
        self.assertEqual(len(progress.read_series(self.series).cycles), 2)
        before = (self.series / "cycles.csv").read_bytes()
        progress.collect(self.series, self.root, cycles_record=path)
        self.assertEqual((self.series / "cycles.csv").read_bytes(), before)

    def test_collect_cycles_record_preserves_zero_and_numeric_representation(self):
        record = cycle_record()
        record["selfplay_summary"].update(training_positions=0, draws=2)
        record["training_summary"]["diagnostics"]["latest_training_sample"].update(
            policy=0.0, value_result=0, legal_probability_mass=0)
        record["training_summary"]["diagnostics"]["validation"]["legal_probability_mass"] = 1
        record["lr"] = 1e-9
        record["generation"]["target_positions"] = 9007199254740993
        path = self.write_cycles_record([record])
        progress.collect(self.series, self.root, cycles_record=path)
        columns, rows = self.read_csv("cycles.csv")
        row = dict(zip(columns, rows[0], strict=True))
        self.assertEqual({name: row[name] for name in (
            "training_positions", "cumulative_training_positions", "draws", "train_policy",
            "train_value_result", "train_legal_probability_mass", "validation_legal_probability_mass",
            "setting_lr", "setting_target_positions",
        )}, {
            "training_positions": "0", "cumulative_training_positions": "0", "draws": "0",
            "train_policy": "0.0", "train_value_result": "0", "train_legal_probability_mass": "0",
            "validation_legal_probability_mass": "1", "setting_lr": "1e-09",
            "setting_target_positions": "9007199254740993",
        })
        progress.read_series(self.series)

    def test_collect_cycles_record_requires_contiguous_cycles(self):
        for numbers in ((2,), (1, 3), (1, 1), (1, 2, 2)):
            with self.subTest(cycles=numbers):
                path = self.write_cycles_record([cycle_record(number) for number in numbers])
                self.assert_cycle_record_error_preserves_csv(path, "cycle", len(numbers))

    def test_collect_cycles_record_rejects_empty_and_malformed_lines(self):
        for content, line in (("", 1), ("\n", 1), ("{", 1), ("[]\n", 1),
                              (json.dumps(cycle_record()) + "\n\n", 2)):
            with self.subTest(content=content):
                path = self.root / "cycles.jsonl"
                path.write_text(content, encoding="utf-8")
                self.assert_cycle_record_error_preserves_csv(path, "cycle" if content == "" else "", line)
        path.unlink()
        self.assert_cycle_record_error_preserves_csv(path, "JSON", 1)

    def test_collect_cycles_record_missing_fields(self):
        for keys in (("replay_buffer_games",), ("lr",), ("generation", "simulations"),
                     ("selfplay_summary", "training_positions"),
                     ("training_summary", "config", "precision"),
                     ("training_summary", "diagnostics", "latest_training_sample"),
                     ("training_summary", "diagnostics", "validation", "policy")):
            with self.subTest(keys=keys):
                record = cycle_record(2)
                target = record
                for key in keys[:-1]:
                    target = target[key]
                del target[keys[-1]]
                path = self.write_cycles_record([cycle_record(), record])
                self.assert_cycle_record_error_preserves_csv(path, keys[-1], 2)

    def test_collect_cycles_record_rejects_invalid_types_and_ranges(self):
        cases = [
            (("cycle",), True), (("cycle",), 1.0),
            (("selfplay_summary", "training_positions"), 12.0),
            (("selfplay_summary", "plies"), True),
            (("selfplay_summary", "training_positions"), -1),
            (("selfplay_summary", "games"), 0),
            (("selfplay_summary", "draws"), 1),
            (("selfplay_summary", "win_reason_resignation"), 6),
            (("selfplay_summary", "draw_reason_max_ply"), 11),
            (("training_summary", "diagnostics", "latest_training_sample", "policy"), -0.1),
            (("training_summary", "diagnostics", "validation", "legal_probability_mass"), 1.01),
            (("training_summary", "diagnostics", "validation", "legal_probability_mass"), -0.01),
            (("training_summary", "config"), []),
            (("training_summary", "config", "batch_size"), None),
            (("generation", "start_position"), False),
        ]
        for value in (True, None, [], {}, "", math.nan, math.inf, -math.inf):
            cases.append((("lr",), value))
        for value in (True, None, "0.5", math.nan, math.inf, -math.inf):
            cases.append((("training_summary", "diagnostics", "validation", "policy"), value))
        for keys, value in cases:
            with self.subTest(keys=keys, value=value):
                record = cycle_record()
                target = record
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = value
                path = self.write_cycles_record([record])
                # 件数の和の違反は、CSVで対応する終わり方の列を示す。
                field = "resigned_games" if keys[-1] == "win_reason_resignation" else keys[-1]
                self.assert_cycle_record_error_preserves_csv(path, field, 1)

    def test_collect_cycles_record_checks_measurements_before_writing(self):
        path = self.write_cycles_record([cycle_record()])
        for name in ("cycles.csv", "strength.csv", "validation.csv"):
            (self.series / name).write_bytes(b"previous\n")
        del self.diagnosis["diagnostics"]["value_result"]
        self.write_sources()
        with self.assertRaisesRegex(ValueError, "diagnostic.json.*value_result"):
            progress.collect(self.series, self.root, cycles_record=path)
        for name in ("cycles.csv", "strength.csv", "validation.csv"):
            self.assertEqual((self.series / name).read_bytes(), b"previous\n")

    def test_collected_cycles_plot_setting_change_at_cycle_start(self):
        records = [cycle_record(number) for number in (1, 2, 3)]
        records[2]["lr"] = 0.02
        path = self.write_cycles_record(records)
        progress.collect(self.series, self.root, cycles_record=path)
        figure = progress.build_figure(self.series)
        for axis in figure.axes:
            lines = [item for item in axis.lines
                     if item.get_gid() is not None and item.get_gid().startswith("setting:")]
            self.assertEqual(len(lines), 1)
            self.assertEqual(lines[0].get_gid(), "setting:c3")
            self.assertEqual(list(lines[0].get_xdata()), [0.00024, 0.00024])
        self.assertEqual([item.get_text() for item in artists(figure, "setting-label:")], ["lr=0.02"])
        self.assertEqual(progress.plot(self.series).read_bytes()[:8], b"\x89PNG\r\n\x1a\n")

    def test_collect_sorts_rows_and_selects_named_run(self):
        records = [summary(name) for name in ("z", "a", "h", "n")]
        records[2]["baseline"] = {"kind": "hachu", "limit": {"depth": 3}}
        records[3]["baseline"]["limit"] = {"nodes": 800}
        self.summary_path.write_text(json.dumps(records), encoding="utf-8")
        self.write_definition([(2, "a", "a"), (1, "h", "a"), (1, "n", "a"),
                               (1, "z", "z"), (1, "a", "a")],
                              [(2, "diagnostic.json"), (0, "diagnostic.json")])
        progress.collect(self.series, self.root)
        _, rows = self.read_csv("strength.csv")
        self.assertEqual([row[:4] for row in rows], [
            ["1", "alphabeta", "nodes=500", "a"],
            ["1", "alphabeta", "nodes=500", "z"],
            ["1", "alphabeta", "nodes=800", "a"],
            ["1", "hachu", "depth=3", "a"],
            ["2", "alphabeta", "nodes=500", "a"],
        ])
        self.assertEqual([row[6] for row in rows], ["a", "z", "n", "h", "a"])
        self.assertEqual([row[0] for row in self.read_csv("validation.csv")[1]], ["0", "2"])

    def test_collect_infinity_round_trip(self):
        self.record["valid"]["elo"]["lower"] = "-inf"
        self.record["discarded_as_wins"]["elo"]["elo"] = "+inf"
        self.write_sources()
        progress.collect(self.series, self.root)
        write_csv(self.series / "cycles.csv", CYCLE_COLUMNS, cycle_rows())
        row = progress.read_series(self.series).strength[0]
        self.assertEqual(row["elo_lower"], -math.inf)
        self.assertEqual(row["elo_discarded_as_wins"], math.inf)
        _, rows = self.read_csv("strength.csv")
        self.assertEqual(rows[0][18], "-inf")
        self.assertEqual(rows[0][24], "inf")

    def test_collect_zero_pairs_and_empty_bands_remain_blank(self):
        self.record["valid_pairs"] = 0
        self.record["discarded_pairs"] = 3
        self.record["valid"] = {"pentanomial": [0] * 5, "elo": None, "score_rate": None}
        self.record["without_move_forfeits"] = {"pentanomial": [0] * 5, "elo": None}
        self.record["discarded_as_losses"] = {"pentanomial": [3, 0, 0, 0, 0],
                                               "elo": {"elo": "-inf"}}
        self.record["discarded_as_wins"] = {"pentanomial": [0, 0, 0, 0, 3],
                                             "elo": {"elo": "+inf"}}
        self.record["affected_pairs"] = 3
        self.record["affected_pair_fraction"] = 1.0
        self.write_sources()
        progress.collect(self.series, self.root)
        _, rows = self.read_csv("strength.csv")
        self.assertEqual(rows[0][16:21], [""] * 5)
        self.assertEqual(rows[0][23:], ["-inf", "inf"])
        _, rows = self.read_csv("validation.csv")
        self.assertEqual(rows[0][12:], ["", ""])
        self.assertEqual(rows[0][7], "0.0")
        write_csv(self.series / "cycles.csv", CYCLE_COLUMNS, cycle_rows())
        data = progress.read_series(self.series)
        self.assertIsNone(data.strength[0]["elo"])
        self.assertIsNone(data.validation[0]["policy_agreement_201_up"])

    def test_collect_absent_sections_write_empty_csv(self):
        self.write_definition([], [])
        progress.collect(self.series, self.root)
        self.assertEqual(self.read_csv("strength.csv"), (list(STRENGTH_COLUMNS), []))
        self.assertEqual(self.read_csv("validation.csv"), (list(VALIDATION_COLUMNS), []))
        self.assertFalse((self.series / "cycles.csv").exists())

    def test_collect_missing_file(self):
        for path, field in ((self.summary_path, "summary"),
                            (self.diagnostic_path, "path")):
            with self.subTest(field=field):
                self.write_sources()
                path.unlink()
                self.assert_collect_error("series.toml", field)

    def test_collect_missing_or_ambiguous_run(self):
        for records in ([], [self.record, self.record]):
            with self.subTest(count=len(records)):
                self.summary_path.write_text(json.dumps(records), encoding="utf-8")
                self.assert_collect_error("summary.json", "run_name")

    def test_collect_missing_fields(self):
        cases = [("summary", ("network_sha256",)),
                 ("summary", ("valid", "elo", "lower")),
                 ("summary", ("without_move_forfeits", "pentanomial")),
                 ("diagnostic", ("data", "sha256")),
                 ("diagnostic", ("diagnostics", "policy_agreement"))]
        for source, keys in cases:
            with self.subTest(source=source, keys=keys):
                self.record, self.diagnosis = summary(), diagnostic()
                target = self.record if source == "summary" else self.diagnosis
                for key in keys[:-1]:
                    target = target[key]
                del target[keys[-1]]
                self.write_sources()
                self.assert_collect_error(f"{source}.json", keys[-1])

    def test_collect_duplicate_strength_and_conflicting_conditions(self):
        self.write_definition([(1, "run-a", "standard"), (1, "run-a", "standard")], [])
        self.assert_collect_error("series.toml", "strength")
        for field, value in (("rules_source", "other"), ("max_ply", 200)):
            with self.subTest(field=field):
                other = summary("run-b")
                other[field] = value
                self.summary_path.write_text(json.dumps([summary(), other]), encoding="utf-8")
                self.write_definition([(1, "run-a", "standard"), (2, "run-b", "standard")], [])
                self.assert_collect_error("series.toml", "rules" if field == "rules_source" else field)

    def test_collect_duplicate_validation_and_changed_data(self):
        self.write_definition([], [(0, "diagnostic.json"), (0, "diagnostic.json")])
        self.assert_collect_error("series.toml", "validation")
        self.diagnosis["data"]["sha256"] = "c" * 64
        (self.root / "other.json").write_text(json.dumps(self.diagnosis), encoding="utf-8")
        self.write_definition([], [(0, "diagnostic.json"), (1, "other.json")])
        self.assert_collect_error("series.toml", "data_sha256")

    def test_collect_toml_shape_types_and_paths(self):
        valid = ('[[strength]]\ncycle = 1\nsummary = "summary.json"\n'
                 'run_name = "run-a"\ncondition = "standard"\n')
        cases = [
            ('extra = 1\n', "extra"), ('strength = 1\n', "strength"),
            ('[[validation]]\ncycle = 0\n', "path"),
            ('[[validation]]\ncycle = 0\npath = 10\n', "path"),
            (valid + 'extra = 1\n', "extra"),
            (valid.replace('condition = "standard"\n', ''), "condition"),
            (valid.replace('"standard"', '""'), "condition"),
            (valid.replace('"run-a"', '1'), "run_name"),
            (valid.replace('cycle = 1', 'cycle = true'), "cycle"),
            (valid.replace('cycle = 1', 'cycle = -1'), "cycle"),
            (valid.replace('cycle = 1', 'cycle = 1.0'), "cycle"),
            (valid.replace('"summary.json"', json.dumps(str(self.summary_path))), "summary"),
        ]
        for text, field in cases:
            with self.subTest(text=text):
                (self.series / "series.toml").write_text(text, encoding="utf-8")
                self.assert_collect_error("series.toml", field)

    def test_collect_invalid_json_values(self):
        cases = [(("baseline", "limit"), {}),
                 (("baseline", "limit"), {"nodes": 500, "depth": 3}),
                 (("baseline", "limit"), {"nodes": True}),
                 (("baseline", "kind"), "unknown"),
                 (("max_ply",), "100"), (("valid_pairs",), True),
                 (("valid", "pentanomial"), [0, 2]),
                 (("valid", "pentanomial"), [0, 0, -1, 0, 0]),
                 (("valid", "score_rate"), "0.5"),
                 (("valid", "score_rate"), 1.1),
                 (("valid", "elo", "elo"), "nan"),
                 (("valid", "elo", "lower"), 10.0)]
        for keys, value in cases:
            with self.subTest(keys=keys, value=value):
                self.record = summary()
                target = self.record
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = value
                self.write_sources()
                self.assert_collect_error("summary.json", keys[-1])

    def test_collect_rejects_nonstandard_json_numeric_constants(self):
        """JSONの無限大は文字列だけとし、数値のNaNとInfinityを拒否する。"""
        for value in (math.nan, math.inf, -math.inf):
            with self.subTest(value=value):
                self.record["valid"]["elo"]["elo"] = value
                self.write_sources()
                self.assert_collect_error("summary.json", "JSON")

    def test_collect_null_requires_exactly_zero_counts(self):
        for group in ("valid", "without_move_forfeits", "discarded_as_losses", "discarded_as_wins"):
            for empty in (False, True):
                with self.subTest(group=group, empty=empty):
                    self.record = summary()
                    if empty:
                        self.record[group]["pentanomial"] = [0] * 5
                    else:
                        self.record[group]["elo"] = None
                    self.write_sources()
                    self.assert_collect_error("summary.json", group)
        for count, value in ((2, None), (0, 0.5)):
            with self.subTest(count=count, value=value):
                self.record = summary()
                self.record["valid"]["pentanomial"] = [0, 0, count, 0, 0]
                self.record["valid"]["score_rate"] = value
                if count == 0:
                    self.record["valid"]["elo"] = None
                self.write_sources()
                self.assert_collect_error("summary.json", "valid")

    def test_collect_band_coverage_and_null_validation(self):
        for change in ("missing", "duplicate", "unexpected", "null_nonempty", "value_empty"):
            with self.subTest(change=change):
                self.diagnosis = diagnostic()
                if change == "missing":
                    self.diagnosis["bands"].pop()
                elif change == "duplicate":
                    self.diagnosis["bands"][3] = copy.deepcopy(self.diagnosis["bands"][0])
                elif change == "unexpected":
                    self.diagnosis["bands"][0]["last_move"] = 39
                elif change == "null_nonempty":
                    self.diagnosis["bands"][0]["value_result"] = None
                else:
                    self.diagnosis["bands"][3]["policy_agreement"] = 0.0
                self.write_sources()
                self.assert_collect_error("diagnostic.json", "bands")

    def test_collect_rejects_before_overwriting_outputs(self):
        for name in ("strength.csv", "validation.csv"):
            (self.series / name).write_text("previous\n", encoding="utf-8")
        del self.diagnosis["diagnostics"]["value_result"]
        self.write_sources()
        self.assert_collect_error("diagnostic.json", "value_result")
        for name in ("strength.csv", "validation.csv"):
            self.assertEqual((self.series / name).read_text(), "previous\n")

    def test_read_rejects_unknown_measurement_cycles(self):
        for name in ("strength", "validation"):
            with self.subTest(name=name):
                self.prepare_csv(**{name: [strength_row(4) if name == "strength"
                                          else validation_row(4)]})
                self.assert_read_error(f"{name}.csv", "cycle")

    def test_read_rejects_required_blank_and_invalid_types(self):
        cases = [("cycles", "training_positions", ""), ("cycles", "games", "0"),
                 ("cycles", "cycle", "1.5"), ("cycles", "train_policy", "nan"),
                 ("strength", "opponent", ""), ("strength", "elo", ""),
                 ("strength", "valid_pairs", "two"),
                 ("validation", "value_result", ""),
                 ("validation", "policy_agreement", "inf")]
        for name, field, value in cases:
            with self.subTest(name=name, field=field, value=value):
                rows = {"cycles": cycle_rows(), "strength": [strength_row()],
                        "validation": [validation_row()]}
                rows[name][0][field] = value
                self.prepare_csv(**rows)
                self.assert_read_error(f"{name}.csv", field)

    def test_read_checks_headers_and_row_width(self):
        schemas = {"cycles": CYCLE_COLUMNS, "strength": STRENGTH_COLUMNS,
                   "validation": VALIDATION_COLUMNS}
        for name, schema in schemas.items():
            for columns in (schema[:-1], schema + ("extra",), schema + (schema[-1],)):
                with self.subTest(name=name, columns=columns):
                    self.prepare_csv()
                    write_csv(self.series / f"{name}.csv", columns, [])
                    self.assert_read_error(f"{name}.csv", "header")
        for name in ("strength", "validation"):
            with self.subTest(name=name, kind="order"):
                self.prepare_csv()
                write_csv(self.series / f"{name}.csv", schemas[name][::-1], [])
                self.assert_read_error(f"{name}.csv", "header")
        self.prepare_csv()
        with (self.series / "strength.csv").open("a", encoding="utf-8") as output:
            output.write("1,alphabeta\n")
        self.assert_read_error("strength.csv", "row")

    def test_read_checks_cycle_sequence_and_cumulative_sum(self):
        for field, value in (("cycle", 3), ("cumulative_training_positions", 2_000_000)):
            with self.subTest(field=field):
                rows = cycle_rows()
                rows[1][field] = value
                self.prepare_csv(cycles=rows)
                self.assert_read_error("cycles.csv", field)

    def test_read_accepts_reordered_cycle_columns_and_empty_settings(self):
        self.prepare_csv()
        rows = cycle_rows()
        for row in rows:
            row["setting_window"] = ""
        write_csv(self.series / "cycles.csv", ("setting_window",) + CYCLE_COLUMNS[::-1], rows)
        self.assertEqual(progress.read_series(self.series).cycles[1]["training_positions"], 2_000_000)

    def test_strength_markers_respect_open_interval_boundaries(self):
        """0.1と0.9は塗り、0.099と0.901は白抜きにする。"""
        self.prepare_csv(strength=[strength_row(cycle, score_rate=score)
                                   for cycle, score in enumerate((0.099, 0.1, 0.9, 0.901))])
        figure = progress.build_figure(self.series)
        points = artists(figure, "strength:main:", ":point")
        points.sort(key=lambda point: point.get_xdata()[0])
        self.assertEqual(len(points), 4)
        self.assertTrue(all(isinstance(point, Line2D) for point in points))
        self.assertEqual([point.get_xdata()[0] for point in points], [0.0, 1.0, 3.0, 6.0])
        self.assertEqual([point.get_markerfacecolor() == "none" for point in points],
                         [True, False, False, True])

    def test_infinite_intervals_have_arrows_at_panel_edges(self):
        self.prepare_csv(strength=[
            strength_row(0, elo=-50, elo_lower=-math.inf, elo_upper=-10),
            strength_row(2, elo=50, elo_lower=10, elo_upper=math.inf),
        ])
        figure = progress.build_figure(self.series)
        bottom, top = figure.axes[0].get_ylim()
        self.assertLess(bottom, -50)
        self.assertGreater(top, 50)
        intervals = artists(figure, "strength:main:", ":interval")
        self.assertEqual(len(intervals), 2)
        self.assertTrue(all(isinstance(item, LineCollection) for item in intervals))
        self.assertEqual(intervals[0].get_segments()[0].tolist(), [[0.0, bottom], [0.0, -10.0]])
        self.assertEqual(intervals[1].get_segments()[0].tolist(), [[3.0, 10.0], [3.0, top]])
        arrows = artists(figure, "strength:main:", "-arrow")
        self.assertEqual(len(arrows), 2)
        self.assertTrue(all(isinstance(item, Annotation) for item in arrows))
        self.assertTrue(all(isinstance(item.arrow_patch, FancyArrowPatch) for item in arrows))
        self.assertEqual([item.xy for item in arrows], [(0.0, bottom), (3.0, top)])
        self.assertGreater(arrows[0].xyann[1], bottom)
        self.assertLess(arrows[1].xyann[1], top)

    def test_infinite_point_estimates_have_triangles_and_finite_other_end(self):
        self.prepare_csv(strength=[
            strength_row(1, elo=-math.inf, elo_lower=-math.inf, elo_upper=-80, score_rate=0.0),
            strength_row(3, elo=math.inf, elo_lower=90, elo_upper=math.inf, score_rate=1.0),
        ])
        figure = progress.build_figure(self.series)
        bottom, top = figure.axes[0].get_ylim()
        points = artists(figure, "strength:main:", ":point")
        self.assertEqual([item.get_marker() for item in points], ["v", "^"])
        self.assertEqual([item.get_ydata()[0] for item in points], [bottom, top])
        self.assertEqual([item.get_markerfacecolor() for item in points], ["none", "none"])
        intervals = artists(figure, "strength:main:", ":interval")
        self.assertEqual(intervals[0].get_segments()[0].tolist(), [[1.0, bottom], [1.0, -80.0]])
        self.assertEqual(intervals[1].get_segments()[0].tolist(), [[6.0, 90.0], [6.0, top]])

    def test_strength_without_finite_values_fails_but_no_rows_is_empty(self):
        self.prepare_csv(strength=[strength_row(elo=-math.inf, elo_lower=-math.inf, elo_upper=-math.inf)])
        with self.assertRaisesRegex(ValueError, r"strength\.csv.*elo"):
            progress.build_figure(self.series)
        self.prepare_csv(strength=[])
        figure = progress.build_figure(self.series)
        self.assertIn("no data", [text.get_text() for text in figure.axes[0].texts])

    def test_strength_groups_and_reference_use_exact_keys_and_own_positions(self):
        primary = [strength_row(3), strength_row(1),
                   strength_row(1, opponent="hachu", opponent_limit="depth=3"),
                   strength_row(1, opponent_limit="nodes=800"),
                   strength_row(1, condition="alternate")]
        self.prepare_csv(strength=primary)
        reference = self.root / "reference"
        ref_cycles = cycle_rows()
        for row in ref_cycles:
            row["training_positions"] *= 2
            row["cumulative_training_positions"] *= 2
        self.prepare_csv(directory=reference, cycles=ref_cycles, validation=[], strength=[
            strength_row(2, elo=1000, elo_lower=900, elo_upper=1100),
            strength_row(1, condition="unmatched", elo=10000, elo_lower=9000, elo_upper=11000),
            strength_row(1, opponent="hachu", opponent_limit="depth=4"),
        ])
        figure = progress.build_figure(self.series, reference)
        lines = artists(figure, "strength:main:", ":line")
        refs = artists(figure, "strength:reference:", ":line")
        self.assertEqual(len(lines), 4)
        self.assertEqual(len(refs), 1)
        matched = next(line for line in lines if len(line.get_xdata()) == 2)
        self.assertEqual(list(matched.get_xdata()), [1.0, 6.0])
        self.assertEqual(list(refs[0].get_xdata()), [6.0])
        self.assertEqual(matched.get_color(), refs[0].get_color())
        self.assertLess(refs[0].get_alpha(), matched.get_alpha())
        self.assertIn("reference", refs[0].get_label())
        self.assertGreater(figure.axes[0].get_ylim()[1], 1100)
        self.assertLess(figure.axes[0].get_ylim()[1], 9000)

    def test_validation_reference_requires_same_data_and_main_rows(self):
        reference = self.root / "reference"
        for same_data, main_rows, expected in ((True, True, 6), (False, True, 0), (True, False, 0)):
            with self.subTest(same_data=same_data, main_rows=main_rows):
                main = [validation_row()] if main_rows else []
                self.prepare_csv(validation=main)
                ref = validation_row(2, data_sha256=("b" if same_data else "c") * 64)
                ref_cycles = cycle_rows()
                for row in ref_cycles:
                    row["training_positions"] *= 2
                    row["cumulative_training_positions"] *= 2
                self.prepare_csv(directory=reference, cycles=ref_cycles, validation=[ref], strength=[])
                figure = progress.build_figure(self.series, reference)
                refs = artists(figure, "validation:reference:")
                self.assertEqual(len(refs), expected)
                for ref_line in refs:
                    main_gid = ref_line.get_gid().replace(":reference:", ":main:")
                    main_line = next(item for item in figure.findobj() if item.get_gid() == main_gid)
                    self.assertEqual(ref_line.get_color(), main_line.get_color())
                    self.assertLess(ref_line.get_alpha(), main_line.get_alpha())
                    self.assertIn("reference", ref_line.get_label())
                    self.assertEqual(list(ref_line.get_xdata()), [6.0])
                self.assertEqual(len(artists(figure, "cycles:")), 8)

    def test_six_panels_use_defined_metrics_and_secondary_cycle_axis(self):
        self.prepare_csv(strength=[strength_row(2)], validation=[validation_row(), validation_row(3)])
        figure = progress.build_figure(self.series)
        self.assertEqual(len(figure.axes), 6)
        for axis in figure.axes[1:]:
            self.assertTrue(figure.axes[0].get_shared_x_axes().joined(figure.axes[0], axis))
        metrics = {item.get_gid(): item for item in artists(figure, "cycles:")}
        expected = {
            "train_policy": [2.0, 1.9, 1.8], "validation_policy": [2.2, 2.1, 2.0],
            "train_value_result": [0.6, 0.5, 0.4], "validation_value_result": [0.7, 0.6, 0.5],
            "plies_per_game": [10.0, 20.0, 30.0], "draws": [0.1] * 3,
            "max_ply_games": [0.2] * 3, "resigned_games": [0.3] * 3,
        }
        self.assertEqual(len(metrics), len(expected))
        for name, values in expected.items():
            self.assertEqual(list(metrics[f"cycles:{name}"].get_ydata()), values)
            self.assertEqual(list(metrics[f"cycles:{name}"].get_xdata()), [1.0, 3.0, 6.0])
        self.assertIn("per position", metrics["cycles:train_value_result"].get_label())
        external = artists(figure, "validation:main:value_result")[0]
        self.assertIn("per game", external.get_label())
        self.assertEqual(list(external.get_xdata()), [0.0, 6.0])
        self.assertEqual(list(external.get_ydata()), [0.7, 0.7])
        baseline = artists(figure, "value:ln2")[0]
        self.assertEqual(list(baseline.get_ydata()), [math.log(2), math.log(2)])
        for index in (3, 5):
            self.assertIsInstance(figure.axes[index].yaxis.get_major_formatter(), PercentFormatter)
        top_axis = figure.axes[0].child_axes[0]
        self.assertEqual(list(top_axis.get_xticks()), [0.0, 3.0, 6.0])
        self.assertEqual([item.get_text() for item in top_axis.get_xticklabels()], ["c0", "c2", "c3"])
        empty_band = artists(figure, "validation:main:policy_agreement_201_up")[0]
        self.assertFalse(any(math.isfinite(value) for value in empty_band.get_xydata()[:, 1]))

    def test_setting_changes_are_at_previous_cumulative_and_do_not_cross_blanks(self):
        rows = cycle_rows()
        rows += [dict(rows[-1], cycle=4, training_positions=4_000_000,
                      cumulative_training_positions=10_000_000),
                 dict(rows[-1], cycle=5, training_positions=5_000_000,
                      cumulative_training_positions=15_000_000)]
        for row, window, rate in zip(rows, ("1", "3", "", "5", "5"),
                                      ("0.10", "0.10", "0.10", "0.10", "0.1"), strict=True):
            row["setting_window"] = window
            row["setting_rate"] = rate
        self.prepare_csv(cycles=rows, settings=("setting_window", "setting_rate"))
        figure = progress.build_figure(self.series)
        for axis in figure.axes:
            lines = [item for item in axis.lines
                     if item.get_gid() is not None and item.get_gid().startswith("setting:")]
            self.assertEqual(len(lines), 2)
            self.assertEqual([list(item.get_xdata()) for item in lines], [[1.0, 1.0], [10.0, 10.0]])
        annotations = [item.get_text() for item in figure.axes[0].texts]
        self.assertIn("window=3", annotations)
        self.assertIn("rate=0.1", annotations)
        self.assertNotIn("window=5", annotations)

    def test_plot_only_needs_csv_and_skips_missing_elo_points(self):
        missing = strength_row(2, valid_pairs=0, discarded_pairs=3,
                               pentanomial_2=0, score_rate="", elo="", elo_lower="",
                               elo_upper="", elo_without_move_forfeits="",
                               affected_pairs=3, affected_pair_fraction=1.0)
        self.prepare_csv(strength=[strength_row(1), missing, strength_row(3)])
        (self.series / "series.toml").unlink()
        self.summary_path.unlink()
        self.diagnostic_path.unlink()
        figure = progress.build_figure(self.series)
        self.assertEqual(len(artists(figure, "strength:main:", ":point")), 2)
        line = artists(figure, "strength:main:", ":line")[0]
        self.assertEqual(list(line.get_xdata()), [1.0, 6.0])

    def test_cli_collect_and_plot(self):
        """両サブコマンドを1回ずつ通し、パス解決と上書きを検証する。"""
        script = Path(progress.__file__).resolve()
        repository = script.parents[3]
        source_paths = (os.path.relpath(self.summary_path, repository),
                        os.path.relpath(self.diagnostic_path, repository))
        definition = (self.series / "series.toml").read_text()
        definition = definition.replace('"summary.json"', json.dumps(source_paths[0]))
        definition = definition.replace('"diagnostic.json"', json.dumps(source_paths[1]))
        (self.series / "series.toml").write_text(definition, encoding="utf-8")
        self.write_cycles_record([cycle_record()])
        result = subprocess.run([sys.executable, "-B", str(script), "collect", str(self.series),
                                 "--cycles-record", "cycles.jsonl"],
                                cwd=self.root, text=True, capture_output=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.read_csv("strength.csv")[1][0][1], "alphabeta")
        self.assertEqual(self.read_csv("cycles.csv")[1][0][:3], ["1", "120", "120"])
        (self.series / "series.toml").unlink()
        output = self.series / "progress.png"
        output.write_bytes(b"previous")
        result = subprocess.run([sys.executable, "-B", str(script), "plot", str(self.series),
                                 "--reference", str(self.series)],
                                cwd=self.root, text=True, capture_output=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(output.read_bytes()[:8], b"\x89PNG\r\n\x1a\n")
        self.assertGreater(output.stat().st_size, 1000)


if __name__ == "__main__":
    unittest.main()
