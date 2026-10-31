"""シードの閉区間と投了の較正を、指示書と二項分布の定義から検証する。

較正の小数4桁の期待値はdocs/plans/alphazero-rl.mdの「投了の有効化の条件」に従う。
精度は、誤投了0件の解析解と、二項係数を使った独立の累積確率で確かめる。
"""

from datetime import UTC, datetime
from decimal import Decimal, localcontext
import json
import math
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest.mock import patch

from minase_zero.rl import register_seed_range

from .cli import require_success, run_cli


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

    def write_summary(self, name: str, n: int, x: int) -> Path:
        path = self.root / name
        path.write_text(json.dumps({
            "resign_triggered_playthrough_games": n, "false_resignations": x,
            "games": n + 1000,
        }), encoding="utf-8")
        return path

    def calibrate(self, paths: list[Path]) -> dict:
        result = run_cli(["resign-calibration", "--summary", *map(str, paths)])
        require_success(result)
        self.assertEqual(result.stderr, "")
        self.assertEqual(len(result.stdout.splitlines()), 1)
        report = json.loads(result.stdout)
        self.assertEqual(set(report), {"n", "false_resignations", "upper_bound", "enable_resignation"})
        return report

    def test_design_examples_fix_upper_bounds_and_enablement(self) -> None:
        for x, expected, enabled in ((0, 0.0295, True), (1, 0.0466, True), (2, 0.0616, False)):
            with self.subTest(x=x):
                report = self.calibrate([self.write_summary("summary.json", 100, x)])
                self.assertEqual(report["n"], 100)
                self.assertEqual(report["false_resignations"], x)
                self.assertAlmostEqual(report["upper_bound"], expected, places=4)
                self.assertIs(report["enable_resignation"], enabled)
                # 定義の有限和を通常の確率空間で求め、丸めた設計値とは別に精度を確かめる。
                p = report["upper_bound"]
                cdf = sum(math.comb(100, k) * p**k * (1 - p)**(100 - k) for k in range(x + 1))
                self.assertAlmostEqual(cdf, 0.05, delta=0.05e-9)

    def test_zero_errors_matches_analytic_bound_at_small_probabilities(self) -> None:
        n = 10**12
        report = self.calibrate([self.write_summary("summary.json", n, 0)])
        # P(X = 0) = (1 - p)^n = 0.05を解いた解析解で、相対誤差を照合する。
        expected = -math.expm1(math.log(0.05) / n)
        self.assertAlmostEqual(report["upper_bound"] / expected, 1.0, delta=1e-9)
        self.assertIs(report["enable_resignation"], True)

    def test_large_sample_matches_binomial_definition_without_underflow(self) -> None:
        n, x = 2048, 1024
        report = self.calibrate([self.write_summary("summary.json", n, x)])
        # 浮動小数点の最初の項がアンダーフローする場合も、十進多倍長の有限和で照合する。
        with localcontext() as context:
            context.prec = 50
            p = Decimal(report["upper_bound"])
            q = 1 - p
            cdf = sum(Decimal(math.comb(n, k)) * p**k * q**(n - k) for k in range(x + 1))
            self.assertLess(abs(cdf - Decimal("0.05")), Decimal("5e-11"))
        self.assertIs(report["enable_resignation"], False)

    def test_fewer_than_100_trials_never_enable_resignation(self) -> None:
        report = self.calibrate([self.write_summary("summary.json", 99, 0)])
        self.assertLess(report["upper_bound"], 0.05)
        self.assertIs(report["enable_resignation"], False)

    def test_no_trials_has_null_bound_and_disables_resignation(self) -> None:
        report = self.calibrate([self.write_summary("summary.json", 0, 0)])
        self.assertEqual(report, {
            "n": 0, "false_resignations": 0, "upper_bound": None, "enable_resignation": False,
        })

    def test_all_trials_false_has_bound_one_and_disables_resignation(self) -> None:
        report = self.calibrate([self.write_summary("summary.json", 100, 100)])
        self.assertEqual(report, {
            "n": 100, "false_resignations": 100, "upper_bound": 1.0, "enable_resignation": False,
        })

    def test_multiple_summaries_pool_counts_before_calibration(self) -> None:
        paths = [self.write_summary("original.json", 40, 1), self.write_summary("rerun.json", 60, 0)]
        report = self.calibrate(paths)
        self.assertEqual(report["n"], 100)
        self.assertEqual(report["false_resignations"], 1)
        self.assertAlmostEqual(report["upper_bound"], 0.0466, places=4)
        self.assertIs(report["enable_resignation"], True)

    def test_each_summary_rejects_invalid_counts_even_if_totals_would_be_valid(self) -> None:
        valid = self.write_summary("valid.json", 100, 0)
        invalid = self.root / "invalid.json"
        for counts in (
            {}, {"resign_triggered_playthrough_games": 100}, {"false_resignations": 0},
            {"resign_triggered_playthrough_games": 1, "false_resignations": 2},
            {"resign_triggered_playthrough_games": -1, "false_resignations": 0},
            {"resign_triggered_playthrough_games": 1, "false_resignations": -1},
            {"resign_triggered_playthrough_games": 100.0, "false_resignations": 0},
            {"resign_triggered_playthrough_games": 100, "false_resignations": 0.0},
            {"resign_triggered_playthrough_games": True, "false_resignations": 0},
            {"resign_triggered_playthrough_games": 100, "false_resignations": False},
            {"resign_triggered_playthrough_games": "100", "false_resignations": 0},
            [],
        ):
            with self.subTest(counts=counts):
                invalid.write_text(json.dumps(counts), encoding="utf-8")
                result = run_cli(["resign-calibration", "--summary", str(valid), str(invalid)])
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(result.stdout, "")
                self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)
                self.assertIn(str(invalid), result.stderr)

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
