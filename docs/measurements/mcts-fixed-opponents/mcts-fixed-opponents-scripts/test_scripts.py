"""設計書、`stats.rs`、および記録の型から導いた期待値でスクリプトを試験する。

対局条件はdocs/plans/archived/mcts-fixed-opponents.mdの「HaChuとの対局の条件」と
「固定したαβ探索との対局の条件」に従う。

使い方: python3 -I -m unittest discover
  -s docs/measurements/mcts-fixed-opponents/mcts-fixed-opponents-scripts -p 'test_*.py'
起動引数、Eloと集計、配置と表記変換、分類の境界、通信とログを検査する。
試験用の記録はtempfileの既定の一時領域へ書き、終了時に消す。
通信の試験はPythonの小さな応答器で行い、実エンジンもGPUも通信網も使わない。
"""

from contextlib import contextmanager, redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import call, patch

SCRIPT_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SCRIPT_DIR))

import check_hachu_forfeits as check
import run_match
import summarize


def write_json(path, value):
    path.write_text(json.dumps(value), encoding="utf-8")


def manifest():
    return {
        "candidate": {
            "identity": {"kind": "commit", "args": ["--search", "mcts"]},
            "options": [["NnModel", "ネット.onnx"]],
            "network_sha256": "a" * 64,
        },
        "baseline": {
            "identity": {"kind": "command", "protocol": "cecp"},
            "limit": {"kind": "fixed", "depth": 2, "nodes": None},
        },
        "mode": {"kind": "elo"}, "rules_source": "L1,L3,P0,P5,P6,R2,E1,E2",
        "seed": 20261113, "max_ply": 1900, "response_timeout_secs": 120,
    }


def game(color, count, termination):
    moves = ["11i11h", "11d11e", "10i10h", "10d10e"]
    turns = [
        {"side": "black" if index % 2 == 0 else "white",
         "response": {"kind": "move", "usi": move}}
        for index, move in enumerate(moves[:count])
    ]
    if termination["kind"] == "forfeit" and termination["reason"] != "rejected_move":
        turns.append({"side": termination["loser"],
                      "response": {"kind": "failure", "reason": termination["reason"]}})
    return {"candidate_color": color, "termination": termination, "turns": turns}


def pair(number, category, games):
    return {
        "pair_number": number, "pair_seed": number,
        "opening": {"seed": number, "moves": ["12i12h", "12d12e"]},
        "category": category, "games": games,
    }


def synthetic_pairs():
    # harness/records/mod.rsのcategoryは2局の候補得点に対応する。破棄ペア内の反則も残す。
    return [
        pair(1, 4, [
            game("black", 1, {"kind": "resigned", "loser": "white"}),
            game("white", 2, {"kind": "forfeit", "loser": "black", "reason": "illegal_move"}),
        ]),
        pair(2, None, [
            game("black", 3, {"kind": "cutoff"}),
            game("white", 1, {"kind": "forfeit", "loser": "white", "reason": "rejected_move"}),
        ]),
        pair(3, 0, [
            game("black", 0, {"kind": "forfeit", "loser": "black", "reason": "timeout"}),
            game("white", 1, {"kind": "adjudicated_win", "winner": "black", "reason": "royal_capture"}),
        ]),
        pair(4, 2, [
            game("black", 2, {"kind": "adjudicated_draw", "reason": "repetition"}),
            game("white", 4, {"kind": "adjudicated_draw", "reason": "repetition"}),
        ]),
    ]


class EloTests(unittest.TestCase):
    def test_stats_rs_reference_interval_and_point(self):
        # stats.rsの独立参照値。平均1/2、母分散13/184、115ペア。
        result = summarize.estimate_elo([10, 25, 45, 25, 10])
        self.assertAlmostEqual(result["elo"], 0, places=12)
        self.assertAlmostEqual(result["lower"], -33.864642936575, places=9)
        self.assertAlmostEqual(result["upper"], 33.864642936575, places=9)
        result = summarize.estimate_elo([0, 0, 1, 0, 1])
        self.assertAlmostEqual(result["elo"], 190.84850188786498, places=9)

    def test_smoke_reference(self):
        # mcts-rule-codes-hachu-smoke.mdに記録された丸め後の値。
        result = summarize.estimate_elo([1, 0, 2, 0, 6])
        self.assertAlmostEqual(result["elo"], 217.6, delta=0.05)
        self.assertAlmostEqual(result["lower"], 37.7, delta=0.05)
        self.assertEqual(result["upper"], "+inf")

    def test_infinities_and_empty_sample(self):
        for counts, value in (([0, 0, 0, 0, 50], "+inf"), ([50, 0, 0, 0, 0], "-inf")):
            result = summarize.estimate_elo(counts)
            self.assertEqual(result, {"elo": value, "lower": value, "upper": value})
            json.dumps(result, allow_nan=False)
        self.assertIsNone(summarize.estimate_elo([0, 0, 0, 0, 0]))

    def test_reflection_and_sample_size(self):
        small = summarize.estimate_elo([1, 2, 4, 3, 1])
        mirrored = summarize.estimate_elo([1, 3, 4, 2, 1])
        large = summarize.estimate_elo([10, 20, 40, 30, 10])
        self.assertAlmostEqual(small["elo"], -mirrored["elo"])
        self.assertAlmostEqual(small["elo"], large["elo"])
        self.assertLess(large["upper"] - large["lower"], small["upper"] - small["lower"])


class SummaryTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "pairs").mkdir()
        write_json(self.root / "manifest.json", manifest())
        write_json(self.root / "summary.json", {"invocation_active": False, "active_wall_time_ns": 2500000000})

    def save_pairs(self, pairs):
        for record in pairs:
            write_json(self.root / "pairs" / f'{record["pair_number"]:020d}.json', record)

    def test_discard_exclusion_union_and_all_game_counts(self):
        self.save_pairs(synthetic_pairs())
        result = summarize.summarize(self.root)
        self.assertEqual(result["attempted_pairs"], 4)
        self.assertEqual(result["valid_pairs"], 3)
        self.assertEqual(result["discarded_pairs"], 1)
        self.assertEqual(result["discarded_pair_numbers"], [2])
        self.assertEqual(result["valid"]["pentanomial"], [1, 0, 1, 0, 1])
        self.assertEqual(result["valid"]["score_rate"], 0.5)
        self.assertAlmostEqual(result["valid"]["elo"]["elo"], 0)
        self.assertEqual(result["move_forfeit_pair_numbers"], [1, 2])
        self.assertEqual(result["without_move_forfeits"]["pentanomial"], [1, 0, 1, 0, 0])
        self.assertAlmostEqual(result["without_move_forfeits"]["elo"]["elo"], -190.84850188786498)
        self.assertEqual(result["discarded_as_losses"]["pentanomial"], [2, 0, 1, 0, 1])
        self.assertEqual(result["discarded_as_wins"]["pentanomial"], [1, 0, 1, 0, 2])
        # 得点率3/8と5/8から、Eloは±400 log10(5/3)。
        self.assertAlmostEqual(result["discarded_as_losses"]["elo"]["elo"], -88.73949984654254)
        self.assertAlmostEqual(result["discarded_as_wins"]["elo"]["elo"], 88.73949984654254)
        self.assertEqual(result["affected_pairs"], 2)
        self.assertEqual(result["affected_pair_fraction"], 0.5)
        self.assertIs(result["affected_pair_fraction_exceeds_5_percent"], True)
        self.assertEqual(result["forfeits"]["illegal_move"], {"candidate": 0, "baseline": 1})
        self.assertEqual(result["forfeits"]["rejected_move"], {"candidate": 1, "baseline": 0})
        self.assertEqual(result["forfeits"]["timeout"], {"candidate": 1, "baseline": 0})
        self.assertEqual(result["forfeits"]["crash"], {"candidate": 0, "baseline": 0})
        self.assertEqual(result["forfeits"]["time_forfeit"], {"candidate": 0, "baseline": 0})
        self.assertEqual(result["terminations"], [
            {"kind": "adjudicated_draw", "reason": "repetition", "winner": None, "count": 2},
            {"kind": "adjudicated_win", "reason": "royal_capture", "winner": "baseline", "count": 1},
            {"kind": "cutoff", "reason": None, "winner": None, "count": 1},
            {"kind": "forfeit", "reason": None, "winner": "baseline", "count": 2},
            {"kind": "forfeit", "reason": None, "winner": "candidate", "count": 1},
            {"kind": "resigned", "reason": None, "winner": "candidate", "count": 1},
        ])
        self.assertEqual(result["plies"], {"min": 2, "median": 3.5, "max": 6})
        self.assertEqual(result["active_wall_time_seconds"], 2.5)
        self.assertEqual(result["baseline"], {"kind": "hachu", "limit": {"depth": 2}})
        self.assertEqual(result["network_sha256"], "a" * 64)

    def test_five_percent_is_not_exceeded_and_no_discard_matches_valid(self):
        records = []
        for number in range(1, 21):
            termination = {"kind": "adjudicated_draw", "reason": "repetition"}
            records.append(pair(number, 2, [game("black", 0, termination), game("white", 0, termination)]))
        records[0]["category"] = None
        records[0]["games"][0]["termination"] = {"kind": "cutoff"}
        self.save_pairs(records)
        result = summarize.summarize(self.root)
        self.assertEqual(result["affected_pair_fraction"], 0.05)
        self.assertIs(result["affected_pair_fraction_exceeds_5_percent"], False)
        records[0]["category"] = 4
        records[0]["games"] = [game("black", 0, {"kind": "resigned", "loser": "white"}),
                               game("white", 0, {"kind": "resigned", "loser": "black"})]
        self.save_pairs(records)
        result = summarize.summarize(self.root)
        self.assertEqual(result["discarded_as_losses"], result["valid"])
        self.assertEqual(result["discarded_as_wins"], result["valid"])

    def test_no_valid_pairs_and_alphabeta_identity(self):
        value = manifest()
        value["baseline"] = {"identity": {"kind": "commit"}, "limit": {"nodes": 500}}
        write_json(self.root / "manifest.json", value)
        self.save_pairs([synthetic_pairs()[1]])
        result = summarize.summarize(self.root)
        self.assertEqual(result["baseline"], {"kind": "alphabeta", "limit": {"nodes": 500}})
        self.assertIsNone(result["valid"]["elo"])
        self.assertIsNone(result["valid"]["score_rate"])
        self.assertEqual(result["affected_pair_fraction"], 1)
        self.assertEqual(result["discarded_as_losses"]["elo"]["elo"], "-inf")
        self.assertEqual(result["discarded_as_wins"]["elo"]["elo"], "+inf")

    def test_no_attempted_pairs_is_an_error(self):
        with self.assertRaisesRegex(ValueError, "試行ペアが0件"):
            summarize.summarize(self.root)

    def test_rejects_non_mcts_active_and_non_elo(self):
        cases = [
            ("candidate", {**manifest()["candidate"], "identity": {"args": ["--search", "alphabeta", "mcts"]}}),
            ("mode", {"kind": "gsprt"}),
        ]
        for key, value in cases:
            modified = manifest()
            modified[key] = value
            write_json(self.root / "manifest.json", modified)
            with self.assertRaises(ValueError):
                summarize.summarize(self.root)
        write_json(self.root / "manifest.json", manifest())
        write_json(self.root / "summary.json", {"invocation_active": True, "active_wall_time_ns": 1})
        with self.assertRaises(ValueError):
            summarize.summarize(self.root)

    def test_cli_isolated_unicode_json_array(self):
        self.save_pairs(synthetic_pairs())
        result = subprocess.run([sys.executable, "-I", str(SCRIPT_DIR / "summarize.py"),
                                 str(self.root), str(self.root)], capture_output=True, text=True, check=True)
        self.assertIn("ネット.onnx", result.stdout)
        values = json.loads(result.stdout)
        self.assertEqual(len(values), 2)
        self.assertEqual(values[0]["attempted_pairs"], 4)

    def test_classifier_legacy_candidate_unknown_and_cli_exit_codes(self):
        # 拒否の診断は旧形式の候補・時間制限を検査しない。候補側は再生せずunknown。
        value = manifest()
        value["candidate"] = {"identity": {"kind": "commit"}}
        value["baseline"]["limit"] = {"kind": "time", "base_ms": 60000}
        write_json(self.root / "manifest.json", value)
        argv = [sys.executable, "-I", str(SCRIPT_DIR / "check_hachu_forfeits.py"), str(self.root),
                "--hachu", "実在しないHaChu", "--minase", "実在しないminase"]
        empty = subprocess.run(argv, capture_output=True, text=True, check=True)
        self.assertEqual(json.loads(empty.stdout), [])
        record = synthetic_pairs()[1]
        self.save_pairs([record])
        result = subprocess.run(argv, capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(json.loads(result.stdout)[0]["classification"], "unknown")
        self.assertEqual(json.loads(result.stdout)[0]["loser"], "candidate")
        value["baseline"]["identity"] = {"kind": "commit"}
        write_json(self.root / "manifest.json", value)
        with self.assertRaisesRegex(ValueError, "CECP"):
            check.check_run(self.root, "absent", "absent")

    def test_classifier_requires_recorded_response_timeout_without_forfeits(self):
        # 設計書「HaChuとの対局の条件」の再生には、記録された応答期限を使う。
        value = manifest()
        del value["response_timeout_secs"]
        write_json(self.root / "manifest.json", value)
        with self.assertRaisesRegex(ValueError, "response_timeout_secs"):
            check.check_run(self.root, "absent", "absent")


class RunMatchTests(unittest.TestCase):
    def arguments(self, command, extra):
        return run_match.parser().parse_args([
            command, "--harness", "bin/harness", "--mcts-commit", "abc123",
            "--net", "nets/a b.onnx", "--name", "example", "--seed", "20261113",
            "--pairs", "30", "--concurrency", "2", *extra,
        ])

    def prefix(self):
        # 設計書「HaChuとの対局の条件」のMCTS条件を、実装側の定数を参照せずに列記する。
        return [
            "bin/harness", "match", "run", "--run-dir", "data/matches/example",
            "--seed", "20261113", "--candidate", "commit:abc123", "--candidate-args", "--search mcts",
            "--candidate-option", "NnModel=nets/a b.onnx", "--candidate-option", "NnBatchSize=16",
            "--candidate-limit", "nodes=800",
        ]

    def test_alphabeta_argv_order(self):
        for nodes in (250, 500, 1000):
            args = self.arguments("alphabeta", ["--nodes", str(nodes)])
            self.assertEqual(run_match.build_argv(args), self.prefix() + [
                "--baseline", "commit:45d5db5498e1739b2307b3719891a1eec24f7fbd",
                "--baseline-args", "--rules lishogi", "--baseline-limit", f"nodes={nodes}",
                "--rules", "lishogi", "--max-ply", "4096", "--concurrency", "2", "elo", "--pairs", "30",
            ])

    def test_hachu_argv_order(self):
        args = self.arguments("hachu", ["--hachu", "bin/hachu", "--depth", "2"])
        self.assertEqual(run_match.build_argv(args), self.prefix() + [
            "--baseline", "cecp:bin/hachu", "--baseline-limit", "depth=2", "--baseline-hash", "256",
            "--rules", "L1,L3,P0,P5,P6,R2,E1,E2", "--max-ply", "1900",
            "--concurrency", "2", "elo", "--pairs", "30",
        ])

    def test_resume_argv(self):
        for extra in ([], ["--target-pairs", "100"]):
            args = run_match.parser().parse_args(["resume", "--harness", "minase", "--name", "saved", *extra])
            self.assertEqual(run_match.build_argv(args), ["minase", "match", "run", "--resume", "data/matches/saved", *extra])

    def test_hachu_whitespace_and_zero_depth_are_errors(self):
        for path in ("a b", "a\tb", "a\nb"):
            args = self.arguments("hachu", ["--hachu", path, "--depth", "1"])
            with self.assertRaises(ValueError):
                run_match.build_argv(args)
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            self.arguments("hachu", ["--hachu", "hachu", "--depth", "0"])

    def test_logs_both_streams_by_line_and_appends(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "example.log"
            log.write_bytes(b"previous\n")
            stdout, stderr = io.StringIO(), io.StringIO()
            argv = [sys.executable, "-I", "-c",
                    "import sys; print('out'); sys.stderr.write('err\\nlast'); sys.exit(7)"]
            command = shlex.join(argv)
            with redirect_stdout(stdout), redirect_stderr(stderr):
                code = run_match.run_logged(argv, log)
            self.assertEqual(code, 7)
            self.assertEqual(stdout.getvalue().splitlines()[0], command)
            self.assertEqual(stdout.getvalue().splitlines()[-1], "out")
            self.assertEqual(stderr.getvalue(), "err\nlast")
            data = log.read_bytes()
            prefix = b"previous\n" + (command + "\n").encode("utf-8")
            self.assertTrue(data.startswith(prefix))
            self.assertIn(b"out\n", data)
            self.assertEqual(data.removeprefix(prefix).replace(b"out\n", b""), b"err\nlast")

    def test_each_launch_starts_with_command_after_unterminated_output(self):
        with tempfile.TemporaryDirectory() as directory:
            log = Path(directory) / "example.log"
            argv = [sys.executable, "-I", "-c", "import sys; sys.stdout.write('last')"]
            command = shlex.join(argv)
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                for _ in range(2):
                    self.assertEqual(run_match.run_logged(argv, log), 0)
            self.assertEqual(log.read_text(encoding="utf-8"), f"{command}\nlast\n{command}\nlast")


class PositionTests(unittest.TestCase):
    def test_article_5_initial_position_and_two_square_move(self):
        position = check.Position.initial()
        self.assertEqual(sum(piece is not None for piece in position.cells), 92)
        self.assertEqual(position.cells[36], "P")  # 12i、CECPのa4。
        position.play("12i12h")
        self.assertIsNone(position.cells[36])
        self.assertEqual(position.cells[48], "P")
        self.assertEqual(position.side, "white")

    def test_article_18_promotion(self):
        cells = [None] * 144
        cells[48] = "P"  # 12h。
        position = check.Position(cells, "black")
        position.play("12h12g+")
        self.assertIsNone(position.cells[48])
        self.assertEqual(position.cells[60], "+P")

    def test_article_12_two_captures_and_return_capture(self):
        for move, destination in (("6f5e4d", 104), ("6f5e6f", 78)):
            cells = [None] * 144
            cells[78], cells[91], cells[104] = "N", "p", "r"
            position = check.Position(cells, "black")
            position.play(move)
            self.assertIsNone(position.cells[91])
            self.assertEqual(position.cells[destination], "N")
            if destination != 78:
                self.assertIsNone(position.cells[78])
            else:
                self.assertEqual(position.cells[104], "r")

    def test_article_12_jitto_preserves_placement_and_changes_side(self):
        cells = [None] * 144
        cells[78] = "N"
        position = check.Position(cells, "black")
        before = position.key()
        self.assertEqual(check.usi_to_cecp("6f5e6f", position), "@@@@")
        self.assertEqual(check.cecp_to_usi("@@@@", position), "6f7g6f")
        position.play("6f5e6f")
        self.assertEqual(position.key()[0], before[0])
        self.assertNotEqual(position.key(), before)

    def test_cecp_conversion_captures_promotion_and_normalization(self):
        cells = [None] * 144
        cells[78], cells[91] = "N", "p"
        position = check.Position(cells, "black")
        self.assertEqual(check.usi_to_cecp("6f5e4d", position), "g7h8,h8i9")
        self.assertEqual(check.cecp_to_usi("g7h8,h8i9", position), "6f5e4d")
        self.assertEqual(check.cecp_to_usi("a11a12+", position), "12b12a+")
        self.assertEqual(check.explicit_cecp("12b12a+"), "a11a12+")
        cells[91] = None
        self.assertEqual(check.cecp_to_usi("g7h8,h8i9", position), "6f4d")
        self.assertEqual(check.usi_to_cecp("6f5e4d", position), "g7i9")
        with self.assertRaises(ValueError):
            check.cecp_to_usi("g7h8,a1a2", position)

    def test_nearest_repetition_uses_side_and_latest_occurrence(self):
        cells = tuple([None] * 144)
        black, white = (cells, "black"), (cells, "white")
        self.assertIsNone(check.repetition_distance([black, white]))
        self.assertEqual(check.repetition_distance([black, white, black, white, black]), 2)

    def test_articles_11_12_jitto_directions_and_representative(self):
        # 角鷹は前、飛鷲は前斜めだけ往復できる。麒麟成りは獅子と同じ。
        for piece, side, expected in [
            ("+O", "black", "6f7g6f"),
            ("+H", "black", "6f6e6f"), ("+h", "white", "6f6g6f"),
            ("+D", "black", "6f7e6f"), ("+d", "white", "6f7g6f"),
        ]:
            cells = [None] * 144
            cells[78] = piece
            position = check.Position(cells, side)
            self.assertEqual(check.cecp_to_usi("@@@@", position), expected)
        cells = [None] * 144
        cells[78], cells[52] = "N", "N"
        self.assertEqual(check.cecp_to_usi("@@@@", check.Position(cells, "black")), "8h9i8h")


class ClassificationTests(unittest.TestCase):
    def short_illegal_game(self):
        return pair(1, 4, [{
            "candidate_color": "white",
            "termination": {"kind": "forfeit", "loser": "black", "reason": "illegal_move"},
            "turns": [
                {"side": "black", "response": {"kind": "move", "usi": "11i11h"}},
                {"side": "white", "response": {"kind": "move", "usi": "11d11e"}},
                {"side": "black", "response": {"kind": "failure", "reason": "illegal_move"}},
            ],
        }])

    def test_known_repetition_boundaries_require_matching_response(self):
        rejection = ["Illegal move {Repeats earlier position}"]
        illegal = ["Illegal move (repetition): c8b7,b7c8"]
        move = "c8b7,b7c8"
        self.assertEqual(check.classify_repetition("rejected_move", rejection, 200, move), "placement_only_repetition")
        self.assertEqual(check.classify_repetition("rejected_move", rejection, 202, move), "unknown")
        self.assertEqual(check.classify_repetition("illegal_move", illegal, 200, move), "unknown")
        self.assertEqual(check.classify_repetition("illegal_move", illegal, 202, move), "history_window")
        for reason in ("rejected_move", "illegal_move"):
            self.assertEqual(check.classify_repetition(reason, ["Illegal move: a1b1"], 202, move), "unknown")
            self.assertEqual(check.classify_repetition(reason, rejection + illegal, None, move), "unknown")

    def test_bare_rejection_requires_jitto_and_repetition_within_200_plies(self):
        # hachu.c 2714、2731〜2736行。@@@@の反復拒否には理由が付かない。
        for move, distance, expected in [
            ("@@@@", 2, "placement_only_repetition"),
            ("@@@@", 200, "placement_only_repetition"),
            ("@@@@", 202, "unknown"),
            ("@@@@", None, "unknown"),
            ("a1b1", 2, "unknown"),
            ("c11c12,c12c11", 200, "unknown"),
        ]:
            with self.subTest(move=move, distance=distance):
                self.assertEqual(check.classify_repetition("rejected_move", ["Illegal move"], distance, move),
                                 expected)
        for line in ("Illegal move: @@@@", "Illegal move {Counterstrike against Lion}"):
            with self.subTest(line=line):
                self.assertEqual(check.classify_repetition("rejected_move", [line], 2, "@@@@"), "unknown")
        self.assertEqual(check.classify_repetition("illegal_move", ["Illegal move"], 202, "@@@@"), "unknown")

    def test_candidate_forfeits_are_unknown_without_processes(self):
        for reason in ("rejected_move", "illegal_move"):
            record = pair(1, 0, [game("black", 1, {"kind": "forfeit", "loser": "black", "reason": reason})])
            with patch.object(check, "CecpEngine", side_effect=AssertionError("起動は禁止")):
                result = check.classify_game(manifest(), record, 0, "absent", "absent")
            self.assertEqual(result["classification"], "unknown")
            self.assertEqual(result["loser"], "candidate")
            self.assertEqual(result["ply"], 3 if reason == "rejected_move" else 4)

    def test_missing_depth_for_baseline_illegal_move_is_an_error(self):
        value = manifest()
        value["baseline"]["limit"] = {"kind": "time", "base_ms": 60000}
        record = synthetic_pairs()[0]
        with self.assertRaisesRegex(ValueError, "深さ"):
            check.classify_game(value, record, 1, "absent", "absent")

    def test_split_move_and_rejection_after_move(self):
        self.assertEqual(check.received_move(["move e7d8,", "move d8d7"]), "e7d8,d8d7")
        self.assertEqual(check.received_move(["move a11a12+", "1-0 {royal capture}"]), "a11a12+")
        for lines in (["move a1b1", "Illegal move {Repeats earlier position}"], [], ["0-1 {resign}"]):
            with self.assertRaises(RuntimeError):
                check.received_move(lines)

    def test_search_replay_sends_only_new_moves_in_harness_order(self):
        record = self.short_illegal_game()
        with patch.object(check, "CecpEngine") as factory:
            engine = factory.return_value.__enter__.return_value
            engine.sync.side_effect = [["move b4b5"], ["move a5a6"]]
            self.assertEqual(check.reproduce_illegal_move("hachu", record, record["games"][0], 2, 120),
                             ("a5a6", "12h12g", ["move a5a6"]))
            factory.assert_called_once_with(["hachu"], 120)
            self.assertEqual(engine.method_calls, [
                call.initialize(None), call.send("sd 2"),
                call.send("usermove a4a5"), call.send("usermove a9a8"),
                call.send("time 3000000"), call.send("otim 3000000"), call.send("go"),
                call.sync(2), call.send("force"),
                call.send("usermove b9b8"),
                call.send("time 3000000"), call.send("otim 3000000"), call.send("go"),
                call.sync(4), call.send("force"),
            ])

    def test_replay_disagreement_and_early_rejection_raise(self):
        record = self.short_illegal_game()
        with patch.object(check, "CecpEngine") as factory:
            engine = factory.return_value.__enter__.return_value
            engine.sync.return_value = ["move c4c5"]
            with self.assertRaisesRegex(RuntimeError, "一致しません"):
                check.reproduce_illegal_move("hachu", record, record["games"][0], 2, 120)
            engine.sync.return_value = ["Illegal move {Repeats earlier position}"]
            with self.assertRaisesRegex(RuntimeError, "1手目"):
                check.replay_forced(["hachu"], ["12i12h"], "a9a8", None, 120)

    def test_rejected_replay_uses_manifest_response_timeout(self):
        # 設計書「HaChuとの対局の条件」の再生で、実行ごとの応答期限を引き継ぐ。
        value = manifest()
        value["response_timeout_secs"] = 37
        record = pair(1, 0, [game("black", 1, {"kind": "forfeit", "loser": "white", "reason": "rejected_move"})])
        with patch.object(check, "CecpEngine") as factory:
            engine = factory.return_value.__enter__.return_value
            engine.sync.side_effect = [[], [], ["Illegal move"]]
            result = check.classify_game(value, record, 0, "hachu", "minase")
            factory.assert_called_once_with(["hachu"], 37)
        self.assertEqual(result["classification"], "unknown")

    def test_untrackable_illegal_move_remains_unknown(self):
        record = self.short_illegal_game()
        value = manifest()
        value["response_timeout_secs"] = 37
        with patch.object(check, "reproduce_illegal_move", return_value=("a6a7", "12g12f", ["move a6a7"])) as replay, \
                patch.object(check, "replay_forced", return_value=(["Illegal move: a6a7"], check.Position.initial(), [])) as forced:
            result = check.classify_game(value, record, 0, "hachu", "minase")
            replay.assert_called_once_with("hachu", record, record["games"][0], 2, 37)
            forced.assert_called_once_with(["minase", "--protocol", "cecp"],
                                           ["12i12h", "12d12e", "11i11h", "11d11e"],
                                           "a6a7", value["rules_source"], 37)
        self.assertEqual(result["classification"], "unknown")
        self.assertEqual(result["usi"], "12g12f")
        self.assertIsNone(result["repetition_distance"])

    def test_fake_cecp_handshake_sync_and_noise(self):
        # harness/engine/cecp.rsの初期化順、固定時計、ping番号を独立に検査する応答器。
        program = r'''
import sys
expected = iter(["xboard", "protover 2", "memory 256", "new", "variant chu", "easy", "nopost", "force",
                 "sd 2", "time 3000000", "otim 3000000", "go", "ping 8", "force", "quit"])
for raw in sys.stdin:
    line = raw.strip()
    assert line == next(expected), line
    if line == "protover 2":
        print('feature option="done=1 is not a feature" done=0', flush=True)
        print("feature done=1", flush=True)
    if line == "go":
        print("unrelated board line\nmove e7d8,\nmove d8d7", flush=True)
    if line == "ping 8":
        print("pong 8", flush=True)
    if line == "quit":
        break
'''
        with check.CecpEngine([sys.executable, "-I", "-u", "-c", program], 120) as engine:
            engine.initialize(None)
            for command in ("sd 2", "time 3000000", "otim 3000000", "go"):
                engine.send(command)
            self.assertEqual(engine.sync(8), ["move e7d8,", "move d8d7"])
            engine.send("force")

    def test_response_deadline_and_unexpected_eof_fail(self):
        with self.assertRaisesRegex(RuntimeError, "出力が閉じられ"):
            with check.CecpEngine([sys.executable, "-I", "-c", "pass"], 120) as engine:
                with self.assertRaises(TimeoutError):
                    engine.read_line(0)
                engine.read_line(check.time.monotonic() + 120)


class CecpTimeoutTests(unittest.TestCase):
    """設計書「HaChuとの対局の条件」の再生で、ハーネスと同じ期限を守る。"""

    @contextmanager
    def timed_engine(self, events, timeout):
        events = list(events)
        now, ready = 0, b""

        def select(remaining):
            nonlocal now, ready
            if events and events[0][0] <= now + remaining:
                now, line = events.pop(0)
                ready = line.encode("utf-8") + b"\n"
                return [("stdout", check.selectors.EVENT_READ)]
            now += remaining
            return []

        with patch.object(check.subprocess, "Popen") as process, \
                patch.object(check.selectors, "DefaultSelector") as selector, \
                patch.object(check.time, "monotonic", side_effect=lambda: now), \
                patch.object(check.os, "read", side_effect=lambda fd, size: ready):
            process.return_value.poll.return_value = None
            process.return_value.wait.return_value = 0
            selector.return_value.select.side_effect = select
            with check.CecpEngine(["test-engine"], timeout) as engine:
                yield engine

    def test_handshake_uses_one_recorded_deadline_through_feature_done(self):
        for timeout, first, done, succeeds in [(120, 70, 119, True), (120, 70, 121, False), (5, 4, 6, False)]:
            with self.subTest(timeout=timeout, done=done):
                with self.timed_engine([(first, "feature done=0"), (done, "feature done=1")], timeout) as engine:
                    if succeeds:
                        engine.initialize(None)
                    else:
                        with self.assertRaises(TimeoutError):
                            engine.initialize(None)

    def test_sync_uses_separate_deadlines_before_response_and_pong(self):
        # 設計書「HaChuとの対局の条件」の再生をreceive_cecp_responseと揃える。
        for response in ("move a1b1", "Illegal move", "1-0 {royal capture}"):
            with self.subTest(response=response):
                events = [(70, "board line"), (119, response), (220, "another board line"), (238, "pong 8")]
                with self.timed_engine(events, 120) as engine:
                    self.assertEqual(engine.sync(8), [response])

    def test_sync_does_not_reset_deadline_for_noise_or_later_responses(self):
        cases = [[(70, "board line"), (121, "pong 8")]]
        for line in ("board line", "move d8d7", "1-0 {royal capture}"):
            cases.append([(110, "move e7d8,"), (220, line), (231, "pong 8")])
        for events in cases:
            with self.subTest(events=events):
                with self.timed_engine(events, 120) as engine:
                    with self.assertRaises(TimeoutError):
                        engine.sync(8)

    def test_forced_replay_can_receive_only_pong_before_deadline(self):
        with self.timed_engine([(70, "board line"), (119, "pong 8")], 120) as engine:
            self.assertEqual(engine.sync(8), [])


if __name__ == "__main__":
    unittest.main()
