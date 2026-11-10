"""仕様の計算式と偽の生成器を使い、生成の測定と記録の保全を確かめる。"""

from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime, timezone
import importlib.util
import io
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("measure-generation.py")
spec = importlib.util.spec_from_file_location("measure_generation", SCRIPT)
measure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(measure)

COMMIT = "0123456789abcdef" * 2 + "01234567"
CHECKSUM = "a" * 64
FAKE_GENERATOR = '''
import json
import os
from pathlib import Path
import sys

scenario = json.loads(Path(__file__).with_name("generator.json").read_text())
arguments = sys.argv[1:]
output = Path(arguments[arguments.index("--output-dir") + 1])
(output / "arguments.json").write_text(json.dumps(arguments))
(output / "environment.json").write_text(json.dumps({
    name: os.environ[name] for name in ("ORT_DYLIB_PATH", "XDG_CACHE_HOME") if name in os.environ
}))
(output / "summary.json").write_text(json.dumps(scenario["summary"]))
(output / "chunk-00000.json").write_text("{}")
(output / "chunk-00000.mnsd").write_bytes(b"games")
(output / "chunk-00000.resigned.mnsd").write_bytes(b"resigned games")
for line in scenario["stdout"]:
    print(line)
for line in scenario["stderr"]:
    print(line, file=sys.stderr)
raise SystemExit(scenario["exit_code"])
'''


class MeasurementTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="measure-generation-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.out = self.root / "measurements"
        self.network = self.root / "network with spaces.onnx"
        self.network.write_bytes(b"test network")
        self.minase = self.root / "fake minase"
        self.minase.write_text(f"#!{sys.executable}\n{FAKE_GENERATOR}", encoding="utf-8")
        self.minase.chmod(0o755)
        self.variants_path = self.root / "variants.json"
        self.variants = [self.variant("S", None), self.variant("A", "S")]
        self.write_variants(self.variants)
        self.scenario = {
            "summary": {"positions": 30, "resigned_positions": 10, "plies": 70,
                        "full_searches": 44, "fast_searches": 26, "games": 8,
                        "discarded_max_ply": 1, "training_positions": 999},
            "stdout": [f"network_sha256: {CHECKSUM}", "elapsed_seconds: 2.500000",
                       "positions_per_second: 999.000000"],
            "stderr": ["generator diagnostic"],
            "exit_code": 0,
        }
        self.write_scenario()

    def variant(self, name: str, parent: str | None) -> dict:
        return {"name": name, "minase": str(self.minase), "network": str(self.network),
                "commit": COMMIT, "parent": parent}

    def write_variants(self, value: object) -> None:
        self.variants_path.write_text(json.dumps(value), encoding="utf-8")

    def write_scenario(self) -> None:
        (self.root / "generator.json").write_text(json.dumps(self.scenario), encoding="utf-8")

    def run_arguments(self) -> list[str]:
        return ["run", "--variants", str(self.variants_path), "--out", str(self.out),
                "--round-start", "0", "--rounds", "1", "--games", "8", "--chunk-games", "4",
                "--devices", "2", "--threads-per-gpu", "3", "--games-per-thread", "128",
                "--simulations", "32", "--seed", "17"]

    def warm_arguments(self) -> list[str]:
        return ["warm", "--variants", str(self.variants_path), "--out", str(self.out),
                "--devices", "2", "--threads-per-gpu", "3", "--games-per-thread", "128",
                "--simulations", "32", "--seed", "17"]

    def invoke(self, arguments: list[str]) -> tuple[int, str, str]:
        stdout, stderr = io.StringIO(), io.StringIO()
        with redirect_stdout(stdout), redirect_stderr(stderr):
            code = measure.main(arguments)
        return code, stdout.getvalue(), stderr.getvalue()

    def read_rows(self, path: Path) -> list[dict]:
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]

    def write_speeds(self, samples: dict[str, list[float]]) -> list[Path]:
        # 2ファイルに分け、全ファイルを合わせた中央値になることも検査する。
        files = [self.root / "first.jsonl", self.root / "second.jsonl"]
        lines = [[], []]
        for name, speeds in samples.items():
            for number, speed in enumerate(speeds):
                lines[number % 2].append(json.dumps({"name": name, "positions_per_second": speed}))
        for path, rows in zip(files, lines):
            path.write_text("".join(line + "\n" for line in rows), encoding="utf-8")
        return files

    def test_run_records_counts_arguments_environment_and_deletes_only_games(self) -> None:
        before = datetime.now(timezone.utc)
        environment = {"ORT_DYLIB_PATH": "/caller/runtime.so", "XDG_CACHE_HOME": "/caller/cache"}
        with patch.dict(os.environ, environment):
            code, stdout, stderr = self.invoke(self.run_arguments())
        after = datetime.now(timezone.utc)
        self.assertEqual((code, stdout, stderr), (0, "", ""))
        rows = self.read_rows(self.out / "runs.jsonl")
        self.assertEqual(len(rows), 2)
        for position, name in enumerate(("S", "A")):
            output = self.out / "round-00" / name
            row = rows[position].copy()
            started_at = datetime.fromisoformat(row.pop("started_at"))
            self.assertEqual(started_at.utcoffset().total_seconds(), 0)
            self.assertLessEqual(before, started_at)
            self.assertLessEqual(started_at, after)
            self.assertEqual(row, {
                "round": 0, "position": position, "name": name, "commit": COMMIT,
                "network_sha256": CHECKSUM, "training_positions": 40, "plies": 70,
                "full_searches": 44, "fast_searches": 26, "games": 8, "discarded_max_ply": 1,
                "elapsed_seconds": 2.5, "positions_per_second": 16.0,
            })
            arguments = json.loads((output / "arguments.json").read_text())
            self.assertEqual(arguments[:2], ["data", "zero-selfplay"])
            self.assertEqual(dict(zip(arguments[2::2], arguments[3::2])), {
                "--output-dir": str(output), "--nn-model": str(self.network),
                "--nn-backend": "tensorrt", "--generation-commit": COMMIT, "--seed": "17",
                "--start-position": "shuffled-setup", "--games": "8",
                "--target-positions": "1000000000", "--chunk-games": "4", "--simulations": "32",
                "--playthrough-percent": "100", "--threads": "6", "--devices": "2",
                "--games-per-thread": "128",
            })
            self.assertEqual(json.loads((output / "environment.json").read_text()), environment)
            self.assertEqual(list(output.glob("*.mnsd")), [])
            self.assertTrue((output / "summary.json").is_file())
            self.assertTrue((output / "chunk-00000.json").is_file())
            log = (output.parent / f"{name}.log").read_text()
            self.assertIn("generator diagnostic", log)
            self.assertIn(f"network_sha256: {CHECKSUM}", log)

    def test_round_order_uses_absolute_round_parity_and_keeps_seed(self) -> None:
        self.write_variants([*self.variants, self.variant("B", "A")])
        arguments = self.run_arguments()
        arguments[arguments.index("--round-start") + 1] = "4"
        arguments[arguments.index("--rounds") + 1] = "2"
        code, _, stderr = self.invoke(arguments)
        self.assertEqual(code, 0, stderr)
        rows = self.read_rows(self.out / "runs.jsonl")
        self.assertEqual([(row["round"], row["position"], row["name"]) for row in rows], [
            (4, 0, "S"), (4, 1, "A"), (4, 2, "B"),
            (5, 0, "B"), (5, 1, "A"), (5, 2, "S"),
        ])
        for row in rows:
            output = self.out / f"round-{row['round']:02d}" / row["name"]
            generated = json.loads((output / "arguments.json").read_text())
            self.assertEqual(generated[generated.index("--seed") + 1], "17")

    def test_duplicate_run_is_rejected_without_changing_records_or_logs(self) -> None:
        code, _, stderr = self.invoke(self.run_arguments())
        self.assertEqual(code, 0, stderr)
        record = (self.out / "runs.jsonl").read_bytes()
        log = (self.out / "round-00/S.log").read_bytes()
        code, _, stderr = self.invoke(self.run_arguments())
        self.assertEqual(code, 1)
        self.assertIn("duplicate measurement", stderr)
        self.assertEqual((self.out / "runs.jsonl").read_bytes(), record)
        self.assertEqual((self.out / "round-00/S.log").read_bytes(), log)

    def test_duplicate_in_later_round_is_rejected_before_any_generation(self) -> None:
        arguments = self.run_arguments()
        arguments[arguments.index("--round-start") + 1] = "1"
        code, _, stderr = self.invoke(arguments)
        self.assertEqual(code, 0, stderr)
        shutil.rmtree(self.out / "round-01")
        arguments = self.run_arguments()
        arguments[arguments.index("--rounds") + 1] = "2"
        code, _, stderr = self.invoke(arguments)
        self.assertEqual(code, 1)
        self.assertIn("duplicate measurement", stderr)
        self.assertFalse((self.out / "round-00").exists())

    def test_round_start_appends_new_round_without_regenerating_previous_round(self) -> None:
        code, _, stderr = self.invoke(self.run_arguments())
        self.assertEqual(code, 0, stderr)
        arguments = self.run_arguments()
        arguments[arguments.index("--round-start") + 1] = "1"
        code, _, stderr = self.invoke(arguments)
        self.assertEqual(code, 0, stderr)
        rows = self.read_rows(self.out / "runs.jsonl")
        self.assertEqual([(row["round"], row["name"]) for row in rows],
                         [(0, "S"), (0, "A"), (1, "A"), (1, "S")])

    def test_existing_generator_output_is_rejected_before_any_generation(self) -> None:
        output = self.out / "round-00/A"
        output.mkdir(parents=True)
        (output / "summary.json").write_text("{}")
        code, _, stderr = self.invoke(self.run_arguments())
        self.assertEqual(code, 1)
        self.assertIn("output already exists", stderr)
        self.assertFalse((self.out / "round-00/S").exists())
        self.assertFalse((self.out / "runs.jsonl").exists())

    def test_failed_generator_stops_without_recording_or_deleting_games(self) -> None:
        self.scenario["exit_code"] = 1
        self.write_scenario()
        code, _, stderr = self.invoke(self.run_arguments())
        self.assertEqual(code, 1)
        self.assertIn(str(self.out / "round-00/S.log"), stderr)
        self.assertFalse((self.out / "runs.jsonl").exists())
        self.assertFalse((self.out / "round-00/A").exists())
        self.assertEqual((self.out / "round-00/S/chunk-00000.mnsd").read_bytes(), b"games")

    def test_record_write_failure_preserves_games_and_stops(self) -> None:
        original_open = Path.open

        def fail_record(path, *args, **kwargs):
            if path == self.out / "runs.jsonl" and args == ("a",):
                raise OSError("cannot append measurement")
            return original_open(path, *args, **kwargs)

        with patch.object(Path, "open", new=fail_record):
            code, _, stderr = self.invoke(self.run_arguments())
        self.assertEqual(code, 1)
        self.assertIn("cannot append measurement", stderr)
        self.assertFalse((self.out / "runs.jsonl").exists())
        self.assertFalse((self.out / "round-00/A").exists())
        self.assertTrue((self.out / "round-00/S/chunk-00000.mnsd").is_file())
        self.assertTrue((self.out / "round-00/S/chunk-00000.resigned.mnsd").is_file())

    def test_invalid_summary_counts_fail_without_recording_or_deleting(self) -> None:
        original = self.scenario["summary"].copy()
        invalid = [{name: value for name, value in original.items() if name != "plies"},
                   {**original, "full_searches": "44"}, {**original, "fast_searches": True},
                   {**original, "games": -1}, []]
        for index, summary in enumerate(invalid):
            with self.subTest(summary=summary):
                self.out = self.root / f"invalid-summary-{index}"
                self.scenario["summary"] = summary
                self.write_scenario()
                code, _, stderr = self.invoke(self.run_arguments())
                self.assertEqual(code, 1)
                self.assertIn("summary.json", stderr)
                self.assertFalse((self.out / "runs.jsonl").exists())
                self.assertTrue((self.out / "round-00/S/chunk-00000.mnsd").exists())

    def test_stdout_requires_valid_unique_checksum_and_positive_elapsed(self) -> None:
        invalid = [
            ["elapsed_seconds: 2.5"], [f"network_sha256: {CHECKSUM}"],
            ["network_sha256: invalid", "elapsed_seconds: 2.5"],
            *[[f"network_sha256: {CHECKSUM}", f"elapsed_seconds: {value}"]
              for value in ("0", "-1", "NaN", "Infinity", "true", '"2.5"', "invalid")],
            [f"network_sha256: {CHECKSUM}", "elapsed_seconds: 2.5", "elapsed_seconds: 3"],
            [f"network_sha256: {CHECKSUM}", f"network_sha256: {CHECKSUM}", "elapsed_seconds: 2.5"],
        ]
        for index, lines in enumerate(invalid):
            with self.subTest(lines=lines):
                self.out = self.root / f"invalid-stdout-{index}"
                self.scenario["stdout"] = lines
                self.write_scenario()
                code, _, stderr = self.invoke(self.run_arguments())
                self.assertEqual(code, 1)
                self.assertIn("S.log", stderr)
                self.assertFalse((self.out / "runs.jsonl").exists())
                self.assertFalse((self.out / "round-00/A").exists())

    def test_stderr_cannot_supply_missing_stdout_fields(self) -> None:
        self.scenario["stderr"] = self.scenario["stdout"]
        self.scenario["stdout"] = []
        self.write_scenario()
        code, _, stderr = self.invoke(self.run_arguments())
        self.assertEqual(code, 1)
        self.assertIn("network_sha256", stderr)
        self.assertFalse((self.out / "runs.jsonl").exists())

    def test_unset_environment_is_not_filled_in(self) -> None:
        with patch.dict(os.environ, {}, clear=True):
            code, _, stderr = self.invoke(self.run_arguments())
        self.assertEqual(code, 0, stderr)
        self.assertEqual(json.loads((self.out / "round-00/S/environment.json").read_text()), {})

    def test_warm_uses_16_games_and_run_batch_size_and_measures_wall_time(self) -> None:
        with patch.object(measure.time, "perf_counter", side_effect=[10.0, 14.0, 20.0, 25.0]):
            code, _, stderr = self.invoke(self.warm_arguments())
        self.assertEqual(code, 0, stderr)
        self.assertEqual(self.read_rows(self.out / "warm.jsonl"), [
            {"name": "S", "elapsed_seconds": 4.0, "exit_code": 0},
            {"name": "A", "elapsed_seconds": 5.0, "exit_code": 0},
        ])
        for name in ("S", "A"):
            arguments = json.loads((self.out / "warm" / name / "arguments.json").read_text())
            self.assertEqual(arguments[arguments.index("--games") + 1], "16")
            self.assertEqual(arguments[arguments.index("--chunk-games") + 1], "16")
            self.assertEqual(arguments[arguments.index("--games-per-thread") + 1], "128")
            self.assertEqual(arguments[arguments.index("--threads") + 1], "6")

    def test_warm_records_failure_and_stops(self) -> None:
        self.scenario["exit_code"] = 1
        self.write_scenario()
        with patch.object(measure.time, "perf_counter", side_effect=[10.0, 13.0]):
            code, _, stderr = self.invoke(self.warm_arguments())
        self.assertEqual(code, 1)
        self.assertIn(str(self.out / "warm/S.log"), stderr)
        self.assertEqual(self.read_rows(self.out / "warm.jsonl"), [
            {"name": "S", "elapsed_seconds": 3.0, "exit_code": 1},
        ])
        self.assertFalse((self.out / "warm/A").exists())

    def test_summary_medians_widths_and_decision_boundaries(self) -> None:
        samples = {"S": [7, 8, 9], "A": [11, 12, 13], "U": [8, 9, 10],
                   "E": [9, 10, 11], "Z": [7, 8, 9], "L": [5, 6, 7],
                   "F": [4, 5, 6], "W": [3, 10, 17]}
        self.write_variants([self.variant(name, None if name == "S" else "S") for name in samples])
        paths = self.write_speeds(samples)
        code, stdout, stderr = self.invoke([
            "summarize", "--variants", str(self.variants_path), "--runs", *map(str, paths),
        ])
        self.assertEqual(code, 0, stderr)
        result = json.loads(stdout)
        self.assertEqual(result["variants"]["S"],
                         {"median": 8, "min": 7, "max": 9, "count": 3, "width": 0.25})
        self.assertEqual(result["variants"]["A"]["median"], 12)
        self.assertAlmostEqual(result["variants"]["A"]["width"], 1 / 6)
        # 増分が幅と等しい候補と、低下が親の幅と等しい候補は境界の内側である。
        expected = {
            "A": (0.5, 0.25, "adopt", False),
            "U": (0.125, 0.25, "undecided", False),
            "E": (0.25, 0.25, "undecided", False),
            "Z": (0, 0.25, "reject", False),
            "L": (-0.25, 1 / 3, "reject", False),
            "F": (-0.375, 0.4, "reject", True),
            "W": (0.25, 1.4, "undecided", False),
        }
        self.assertEqual(set(result["comparisons"]), set(expected))
        for name, (increment, threshold, decision, slower) in expected.items():
            with self.subTest(name=name):
                comparison = result["comparisons"][name]
                self.assertEqual(comparison["parent"], "S")
                self.assertAlmostEqual(comparison["increment"], increment)
                self.assertAlmostEqual(comparison["threshold"], threshold)
                self.assertEqual(comparison["decision"], decision)
                self.assertIs(comparison["slower_beyond_width"], slower)
        self.assertIn('\n  "variants":', stdout)

    def test_summary_even_count_and_count_mismatch(self) -> None:
        variants = measure.load_variants(self.variants_path)
        result = measure.summarize(variants, self.write_speeds({"S": [2, 6], "A": [4, 8]}))
        self.assertEqual(result["variants"]["S"]["median"], 4)
        self.assertEqual(result["variants"]["A"]["median"], 6)
        self.assertEqual(result["variants"]["S"]["count"], 2)
        with self.assertRaisesRegex(ValueError, "counts differ"):
            measure.summarize(variants, self.write_speeds({"S": [2, 6], "A": [4]}))

    def test_summary_missing_candidate_or_parent_fails_without_partial_output(self) -> None:
        for samples in ({"S": [8]}, {"A": [12]}, {}):
            with self.subTest(samples=samples):
                paths = self.write_speeds(samples)
                code, stdout, stderr = self.invoke([
                    "summarize", "--variants", str(self.variants_path), "--runs", *map(str, paths),
                ])
                self.assertEqual(code, 1)
                self.assertEqual(stdout, "")
                self.assertIn("no measurement rows", stderr)

    def test_summary_rejects_invalid_speed_and_zero_median(self) -> None:
        variants = measure.load_variants(self.variants_path)
        for value in (-1, True, "8", None, float("nan"), float("inf"), 0):
            with self.subTest(value=value), self.assertRaises(ValueError):
                measure.summarize(variants, self.write_speeds({"S": [value], "A": [12]}))

    def test_summary_rejects_missing_speed_unknown_name_and_malformed_rows(self) -> None:
        path = self.root / "invalid.jsonl"
        for row in ('{"name":"S"}', '{"positions_per_second":8}',
                    '{"name":"unknown","positions_per_second":8}', '[]', 'null', '{', '',
                    '{"name":"S","name":"A","positions_per_second":8}'):
            with self.subTest(row=row):
                path.write_text(row + "\n")
                with self.assertRaises(ValueError):
                    measure.summarize(measure.load_variants(self.variants_path), [path])

    def test_variants_reject_missing_extra_duplicate_and_unknown_parent(self) -> None:
        cases = [
            ([{key: value for key, value in self.variants[0].items() if key != "network"}], "network"),
            ([{**self.variants[0], "extra": 1}], "extra"),
            ([self.variants[0], self.variants[0]], "duplicate name"),
            ([self.variant("A", "missing")], "unknown parent"),
        ]
        for variants, message in cases:
            with self.subTest(variants=variants):
                self.write_variants(variants)
                with self.assertRaisesRegex(ValueError, message):
                    measure.load_variants(self.variants_path)

    def test_variants_reject_wrong_types_paths_and_commit(self) -> None:
        for field, value in (
            ("name", 1), ("name", "../S"), ("name", "."), ("name", ""),
            ("parent", []), ("commit", 123), ("commit", "a" * 39), ("commit", "g" * 40),
            ("network", None), ("network", "relative.onnx"),
            ("network", str(self.root / "missing.onnx")), ("network", str(self.root)),
            ("minase", False), ("minase", str(self.root / "missing-minase")),
        ):
            with self.subTest(field=field, value=value):
                self.write_variants([{**self.variants[0], field: value}])
                with self.assertRaises(ValueError):
                    measure.load_variants(self.variants_path)
        for value in ({}, [], [1]):
            with self.subTest(value=value):
                self.write_variants(value)
                with self.assertRaises(ValueError):
                    measure.load_variants(self.variants_path)

    def test_invalid_later_variant_fails_before_generation(self) -> None:
        self.variants[1]["network"] = str(self.root / "missing.onnx")
        self.write_variants(self.variants)
        code, _, stderr = self.invoke(self.run_arguments())
        self.assertEqual(code, 1)
        self.assertIn("network", stderr)
        self.assertFalse(self.out.exists())

    def test_all_arguments_are_required(self) -> None:
        for complete in (self.warm_arguments(), self.run_arguments(),
                         ["summarize", "--variants", str(self.variants_path), "--runs", "runs.jsonl"]):
            for index in range(1, len(complete), 2):
                with self.subTest(command=complete[0], missing=complete[index]):
                    code, _, stderr = self.invoke(complete[:index] + complete[index + 2:])
                    self.assertEqual(code, 1)
                    self.assertIn(complete[index], stderr)
        self.assertFalse(self.out.exists())

    def test_invalid_numeric_arguments_fail_before_generation(self) -> None:
        for option, value in (
            ("--devices", "0"), ("--threads-per-gpu", "-1"), ("--games-per-thread", "0"),
            ("--simulations", "0"), ("--round-start", "-1"), ("--rounds", "0"),
            ("--games", "0"), ("--chunk-games", "0"), ("--seed", "-1"), ("--seed", str(2 ** 64)),
        ):
            with self.subTest(option=option, value=value):
                arguments = self.run_arguments()
                arguments[arguments.index(option) + 1] = value
                code, _, stderr = self.invoke(arguments)
                self.assertEqual(code, 1)
                self.assertIn(option, stderr)
        self.assertFalse(self.out.exists())


if __name__ == "__main__":
    unittest.main()
