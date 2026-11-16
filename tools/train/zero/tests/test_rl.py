"""シードの閉区間と、設計書docs/plans/archived/zero-recipe-precedents.mdの投了の較正の式を検証する。"""

from datetime import UTC, datetime
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

from minase_zero.rl import register_seed_range, resign_calibration, _resign_threshold

from tests.cli import require_success, run_cli


ENTRY = {
    "name": "cycle-0", "seed": 10, "games": 10, "low": 11, "high": 20,
    "registered_at": "2026-10-28T01:02:03Z",
}


class SeedLedgerTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-ledger-test-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.ledger = self.root / "seeds.jsonl"
        self.ledger.touch()

    def register(self, name: str, seed: int, games: int):
        return run_cli([
            "seed-ledger", "--ledger", str(self.ledger), "--name", name,
            "--seed", str(seed), "--games", str(games),
        ])

    def assert_rejected(self, name: str, seed: int, games: int):
        before = self.ledger.read_bytes()
        result = self.register(name, seed, games)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)
        self.assertEqual(self.ledger.read_bytes(), before)
        self.assertEqual(list(self.root.iterdir()), [self.ledger])
        return result

    def test_empty_ledger_records_interval_and_utc_seconds(self) -> None:
        before = datetime.now(UTC).replace(microsecond=0)
        result = self.register("試走", 0, 3)
        after = datetime.now(UTC)
        require_success(result)
        self.assertEqual(result.stderr, "")
        self.assertEqual(len(result.stdout.splitlines()), 1)
        self.assertEqual(self.ledger.read_text(encoding="utf-8"), result.stdout)
        entry = json.loads(result.stdout)
        timestamp = entry.pop("registered_at")
        self.assertEqual(entry, {"name": "試走", "seed": 0, "games": 3, "low": 1, "high": 3})
        self.assertRegex(timestamp, r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$")
        self.assertLessEqual(before, datetime.fromisoformat(timestamp))
        self.assertLessEqual(datetime.fromisoformat(timestamp), after)
        self.assertEqual(list(self.root.iterdir()), [self.ledger])

    def test_adjacent_intervals_preserve_existing_bytes(self) -> None:
        original = ("  " + json.dumps(ENTRY) + "\r\n").encode("utf-8")
        for seed, interval in ((20, (21, 23)), (7, (8, 10))):
            with self.subTest(seed=seed):
                self.ledger.write_bytes(original)
                result = self.register("adjacent", seed, 3)
                require_success(result)
                entry = json.loads(result.stdout)
                self.assertEqual((entry["low"], entry["high"]), interval)
                self.assertEqual(self.ledger.read_bytes(), original + result.stdout.encode("utf-8"))

    def test_final_line_without_newline_is_separated(self) -> None:
        original = json.dumps(ENTRY).encode("utf-8")
        self.ledger.write_bytes(original)
        result = self.register("next", 20, 1)
        require_success(result)
        self.assertEqual(self.ledger.read_bytes(), original + b"\n" + result.stdout.encode("utf-8"))

    def test_overlaps_report_existing_name_and_interval_without_writing(self) -> None:
        self.ledger.write_text(json.dumps(ENTRY) + "\n", encoding="utf-8")
        # 両端の1点だけの重なり、包含、被包含のいずれも登録できない。
        for seed, games in ((19, 4), (7, 4), (12, 2), (9, 12)):
            with self.subTest(seed=seed, games=games):
                result = self.assert_rejected("overlap", seed, games)
                self.assertIn("cycle-0", result.stderr)
                self.assertRegex(result.stderr, r"11\D+20")

    def test_duplicate_name_is_rejected_even_for_disjoint_interval(self) -> None:
        self.ledger.write_text(json.dumps(ENTRY) + "\n", encoding="utf-8")
        self.assert_rejected("cycle-0", 100, 1)

    def test_uint64_last_value_is_valid_but_next_value_wraps(self) -> None:
        # 対局番号は1始まりなので、最後に通る基本シードはuint64の最大値から対局数を引く。
        result = self.register("last", 2**64 - 1 - 2, 2)
        require_success(result)
        entry = json.loads(result.stdout)
        self.assertEqual((entry["low"], entry["high"]), (2**64 - 2, 2**64 - 1))
        self.assert_rejected("wraps", 2**64 - 2, 2)

    def test_invalid_seed_games_and_names_leave_ledger_unchanged(self) -> None:
        for name, seed, games in (
            ("bad", -1, 1), ("bad", 2**64, 1), ("bad", 0, 0), ("bad", 0, -1),
            ("", 0, 1), ("two words", 0, 1), ("tab\tname", 0, 1), ("全角　空白", 0, 1),
        ):
            with self.subTest(name=name, seed=seed, games=games):
                self.assert_rejected(name, seed, games)

    def test_missing_ledger_is_not_created(self) -> None:
        self.ledger.unlink()
        result = self.register("missing", 0, 1)
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)
        self.assertEqual(list(self.root.iterdir()), [])

    def test_malformed_lines_are_rejected_without_writing(self) -> None:
        for contents in (b"{broken\n", b"[]\n", b"\n", b"\xff\n"):
            with self.subTest(contents=contents):
                self.ledger.write_bytes(contents)
                self.assert_rejected("new", 100, 1)

    def test_existing_fields_are_validated_before_registration(self) -> None:
        # 欄ごとの境界は公開関数でも検証し、ファイルは実物を読み書きする。
        invalid = [{key: value for key, value in ENTRY.items() if key != field} for field in ENTRY]
        invalid += [{**ENTRY, field: value} for field, value in (
            ("name", 1), ("name", ""), ("name", "a b"),
            ("seed", "10"), ("seed", True), ("seed", -1),
            ("games", 10.0), ("games", False), ("games", 0),
            ("low", True), ("low", 11.0), ("low", 10),
            ("high", "20"), ("high", False), ("high", 21),
            ("registered_at", 123), ("registered_at", "2026-02-30T01:02:03Z"),
            ("registered_at", "2026-10-28T01:02:03+00:00"),
            ("registered_at", "2026-10-28t01:02:03z"),
            ("registered_at", "2026-10-28T01:02:03.000Z"),
        )]
        invalid += [{**ENTRY, "extra": 1}, {
            **ENTRY, "seed": 2**64 - 1, "games": 1, "low": 2**64, "high": 2**64,
        }]
        for entry in invalid:
            with self.subTest(entry=entry):
                contents = (json.dumps(entry) + "\n").encode("utf-8")
                self.ledger.write_bytes(contents)
                with self.assertRaises(ValueError):
                    register_seed_range(self.ledger, "new", 100, 1)
                self.assertEqual(self.ledger.read_bytes(), contents)
                self.assertEqual(list(self.root.iterdir()), [self.ledger])

    def test_duplicate_existing_names_and_broken_later_lines_are_rejected(self) -> None:
        first = json.dumps(ENTRY) + "\n"
        duplicate = {**ENTRY, "seed": 20, "low": 21, "high": 30}
        for tail in (json.dumps(duplicate) + "\n", "{broken\n"):
            with self.subTest(tail=tail):
                self.ledger.write_text(first + tail, encoding="utf-8")
                self.assert_rejected("new", 100, 1)

    def test_file_sync_replace_and_directory_sync_commit_in_order(self) -> None:
        # fsyncと置換の順序は耐久性の契約なので、実際の入出力を保ったまま記録する。
        sync, replace = os.fsync, os.replace
        events = []

        def record_sync(descriptor: int) -> None:
            kind = "directory" if stat.S_ISDIR(os.fstat(descriptor).st_mode) else "file"
            events.append(kind)
            sync(descriptor)

        def record_replace(source: Path, destination: Path) -> None:
            self.assertEqual(source.parent, self.ledger.parent)
            self.assertEqual(destination, self.ledger)
            self.assertEqual(self.ledger.read_bytes(), b"")
            self.assertEqual(json.loads(source.read_text())["name"], "durable")
            events.append("replace")
            replace(source, destination)

        with patch("minase_zero.rl.os.fsync", side_effect=record_sync), patch(
            "minase_zero.rl.os.replace", side_effect=record_replace,
        ):
            registration = register_seed_range(self.ledger, "durable", 0, 1)
        self.assertEqual(events, ["file", "replace", "directory"])
        self.assertEqual(json.loads(self.ledger.read_text()), registration)
        self.assertEqual(list(self.root.iterdir()), [self.ledger])

    def test_io_failure_before_replace_preserves_ledger_and_removes_temporary(self) -> None:
        original = (json.dumps(ENTRY) + "\n").encode("utf-8")
        self.ledger.write_bytes(original)
        for operation in ("fsync", "replace"):
            with self.subTest(operation=operation), patch(
                f"minase_zero.rl.os.{operation}", side_effect=OSError("injected I/O failure"),
            ):
                with self.assertRaises(OSError):
                    register_seed_range(self.ledger, "next", 20, 1)
                self.assertEqual(self.ledger.read_bytes(), original)
                self.assertEqual(list(self.root.iterdir()), [self.ledger])

    def test_all_ledger_arguments_are_required(self) -> None:
        arguments = ["--ledger", str(self.ledger), "--name", "new", "--seed", "0", "--games", "1"]
        for offset in range(0, len(arguments), 2):
            with self.subTest(argument=arguments[offset]):
                result = run_cli(["seed-ledger", *arguments[:offset], *arguments[offset + 2:]])
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)
                self.assertEqual(self.ledger.read_bytes(), b"")


class ResignCalibrationTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-calibration-test-")
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def write_summary(self, name, records):
        path = self.root / name
        path.write_text(json.dumps({"resign_calibration_games": records}), encoding="utf-8")
        return path

    @staticmethod
    def game(number=1, result="black", black=0.5, white=0.5):
        return {"game": number, "result": result, "min_value_black": black, "min_value_white": white}

    def test_initial_estimates_and_threshold(self):
        self.assertEqual(resign_calibration([]), {"threshold": 0.0, "estimates": [0.10] * 64, "games": 0})
        path = self.write_summary("empty.json", [])
        self.assertEqual(resign_calibration([path]), resign_calibration([]))

    def test_one_game_uses_winner_or_smaller_draw_value_and_includes_boundary(self):
        # 指示書の式から、最小値が候補以下のときだけ1/5000を加える。
        for result, black, white, first_hit in (
            ("black", 0, 1, 0), ("black", 1, 0, 64),
            ("white", 0, 1, 64), ("white", 1, 0.005, 0),
            ("draw", 1, 0.005, 0), ("draw", 0.005, 1, 0),
            ("black", 0.04, 0, 7), ("white", 1, 0.32, 63),
        ):
            with self.subTest(result=result, black=black, white=white):
                path = self.write_summary("one.json", [self.game(result=result, black=black, white=white)])
                report = resign_calibration([path])
                self.assertEqual(report["games"], 1)
                self.assertEqual(len(report["estimates"]), 64)
                for index, estimate in enumerate(report["estimates"]):
                    expected = 0.10 * 4999 / 5000 + (1 / 5000 if index >= first_hit else 0)
                    self.assertAlmostEqual(estimate, expected, delta=1e-15)

    def test_threshold_uses_first_estimate_strictly_above_five_percent(self):
        for estimates, expected in (([0.10] * 64, 0.0),
                                    ([0.05] * 7 + [0.10] * 57, 0.035),
                                    ([0.05] * 64, 0.32)):
            with self.subTest(expected=expected):
                self.assertEqual(_resign_threshold(estimates), expected)

    def test_cli_applies_summaries_and_games_in_given_order_without_writing_state(self):
        first = self.write_summary("first.json", [self.game(1, black=0), self.game(11, black=1)])
        second = self.write_summary("second.json", [self.game(1, "draw", black=1, white=0)])
        result = run_cli(["resign-calibration", "--summary", str(first), str(second)])
        require_success(result)
        self.assertEqual(result.stderr, "")
        self.assertEqual(len(result.stdout.splitlines()), 1)
        report = json.loads(result.stdout)
        self.assertEqual(set(report), {"threshold", "estimates", "games"})
        self.assertEqual(report["games"], 3)
        decay = 4999 / 5000
        expected = ((0.10 * decay + 1 / 5000) * decay) * decay + 1 / 5000
        for estimate in report["estimates"]:
            self.assertAlmostEqual(estimate, expected, delta=1e-15)
        self.assertEqual(report, resign_calibration([first, second]))
        self.assertNotEqual(report["estimates"], resign_calibration([second, first])["estimates"])
        self.assertEqual(set(self.root.iterdir()), {first, second})

    def test_many_games_cross_threshold_at_the_prescribed_boundary(self):
        for count, expected in ((3465, 0.0), (3466, 0.035)):
            path = self.write_summary("many.json", [self.game(10 * i + 1, black=0.04) for i in range(count)])
            report = resign_calibration([path])
            self.assertEqual(report["threshold"], expected)
            self.assertAlmostEqual(report["estimates"][0], 0.10 * (4999 / 5000)**count, delta=1e-14)

    def test_calibration_rejects_invalid_fields_types_ranges_and_order(self):
        game = self.game()
        invalid = [[], {}, {"resign_calibration_games": None}, {"resign_calibration_games": {}}]
        records = [None, [], {}, {**game, "extra": 1}]
        records += [{key: value for key, value in game.items() if key != field} for field in game]
        records += [{**game, "game": value} for value in (0, -1, True, 1.0, "1", None)]
        records += [{**game, "result": value} for value in ("", "win", 0, True, [], None)]
        records += [{**game, field: value} for field in ("min_value_black", "min_value_white")
                    for value in (-0.01, 1.01, float("nan"), float("inf"), -float("inf"), True, "0.5", None)]
        invalid += [{"resign_calibration_games": [record]} for record in records]
        invalid += [{"resign_calibration_games": [self.game(11), self.game(number)]} for number in (11, 1)]
        path = self.root / "invalid.json"
        for value in invalid:
            with self.subTest(value=value):
                path.write_text(json.dumps(value), encoding="utf-8")
                with self.assertRaisesRegex(ValueError, str(path)):
                    resign_calibration([path])
        result = run_cli(["resign-calibration", "--summary", str(path)])
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(len(result.stderr.splitlines()), 1)

    def test_summary_argument_is_required_and_files_must_be_readable_json(self) -> None:
        broken = self.root / "broken.json"
        broken.write_text("{broken", encoding="utf-8")
        for arguments in (
            [], ["--summary"], ["--summary", str(self.root / "missing.json")],
            ["--summary", str(broken)],
        ):
            with self.subTest(arguments=arguments):
                result = run_cli(["resign-calibration", *arguments])
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)


if __name__ == "__main__":
    unittest.main()
