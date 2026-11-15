#!/usr/bin/env python3
"""選別条件、入力検査、決定性、原文保存と失敗時の契約を検証する。"""

from collections import Counter
import hashlib
import json
from pathlib import Path
import random
import subprocess
import sys
import tempfile
import unittest

from select_validation_games import select_games


SCRIPT = Path(__file__).with_name("select_validation_games.py")
REASONS = (
    "variant", "initial_sfen", "ai", "anonymous", "bot", "unrated",
    "rating", "status", "winner", "short", "player_cap",
)


def game(game_id, sente="alice", gote="bob"):
    """境界の1600点・40手で全条件を満たす合成の対局を作る。"""
    return {
        "id": game_id, "variant": "chushogi", "rated": True,
        "players": {
            "sente": {"user": {"id": sente}, "rating": 1600},
            "gote": {"user": {"id": gote}, "rating": 1600},
        },
        "status": "resign", "winner": "sente", "moves": " ".join(["1i1h"] * 40),
    }


def encode(games):
    """合成対局をUTF-8のNDJSONへ符号化する。"""
    return b"".join((json.dumps(value, ensure_ascii=False) + "\n").encode() for value in games)


class SelectionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "input.ndjson"
        self.output = self.root / "selected.ndjson"
        self.report = self.root / "report.json"

    def command(self, *extra, report=None):
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--input", str(self.source),
             "--output", str(self.output), "--report", str(self.report if report is None else report), *extra],
            capture_output=True, check=False,
        )

    def assert_error(self, result):
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, b"")
        self.assertEqual(len(result.stderr.splitlines()), 1, result.stderr)
        self.assertFalse(self.output.exists())
        self.assertFalse(self.report.exists())

    def assert_input_error(self, raw, line_number):
        self.source.write_bytes(raw)
        result = self.command()
        try:
            self.assert_error(result)
            self.assertIn(f"NDJSON line {line_number}:".encode(), result.stderr)
        finally:
            self.output.unlink(missing_ok=True)
            self.report.unlink(missing_ok=True)

    def test_all_ten_filters_and_first_reason_priority(self):
        rows = []
        for reason in REASONS[:-1]:
            row = game(reason)
            if reason == "variant":
                row["variant"] = "standard"
            elif reason == "initial_sfen":
                row["initialSfen"] = None
            elif reason == "ai":
                row["players"]["sente"]["aiLevel"] = 1
            elif reason == "anonymous":
                del row["players"]["gote"]["user"]
            elif reason == "bot":
                row["players"]["gote"]["user"] = {"id": "bot-z", "title": "BOT"}
            elif reason == "unrated":
                row["rated"] = False
            elif reason == "rating":
                row["players"]["gote"]["rating"] = 1599
            elif reason == "status":
                row["status"] = "draw"
            elif reason == "winner":
                del row["winner"]
            elif reason == "short":
                row["moves"] = " ".join(["1i1h"] * 39)
            rows.append(row)
        multiple = game("first-only")
        multiple["variant"] = "standard"
        multiple["players"]["sente"]["user"] = {"id": "bot-a", "title": "BOT"}
        rows.append(multiple)
        self.source.write_bytes(encode(rows))
        result = self.command()
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads(self.report.read_bytes())
        expected = dict.fromkeys(REASONS, 1)
        expected["variant"] = 2
        expected["player_cap"] = 0
        self.assertEqual(report["excluded"], expected)
        self.assertEqual(report["bot_accounts"], ["bot-a", "bot-z"])
        self.assertEqual(report["candidates"], {"games": 0, "players": 0, "max_player_games": 0, "max_player_share": 0.0})
        self.assertEqual(report["selected"], {**report["candidates"], "plies": 0, "ids": []})
        self.assertEqual(self.output.read_bytes(), b"")
        self.assertEqual(result.stdout.decode().splitlines(), ["selected: 0", *(f"{key}: {value}" for key, value in expected.items())])

    def test_bot_accounts_apply_before_and_after_titles_without_changing_priority(self):
        human = game("human")
        before_sente = game("before-sente", "bot-sente", "alice")
        before_gote = game("before-gote", "alice", "bot-gote")
        initial = game("initial", "bot-sente")
        initial["initialSfen"] = None
        ai = game("ai", "bot-sente")
        ai["players"]["gote"]["aiLevel"] = 1
        anonymous = game("anonymous", "bot-sente")
        anonymous["players"]["gote"]["user"] = None
        evidence_sente = game("evidence-sente", "alice", "bot-sente")
        evidence_sente["players"]["gote"]["user"]["title"] = "BOT"
        evidence_sente["variant"] = "standard"
        evidence_gote = game("evidence-gote", "bot-gote", "alice")
        evidence_gote["players"]["sente"]["user"]["title"] = "BOT"
        after = game("after", "bot-sente", "bot-gote")
        after["rated"] = False
        rows = [human, before_sente, before_gote, initial, ai, anonymous,
                evidence_sente, evidence_gote, after]
        self.source.write_bytes(encode(rows))
        report = select_games(self.source, self.output, self.report)
        self.assertEqual(report["excluded"], {
            **dict.fromkeys(REASONS, 0), "variant": 1, "initial_sfen": 1,
            "ai": 1, "anonymous": 1, "bot": 4,
        })
        self.assertEqual(report["bot_accounts"], ["bot-gote", "bot-sente"])
        self.assertEqual(report["selected"]["ids"], ["human"])
        self.assertEqual(self.output.read_bytes(), encode([human]))
        self.source.write_bytes(encode(reversed(rows)))
        reversed_report = select_games(self.source, self.root / "reversed.ndjson", self.root / "reversed.json")
        for field in ("bot_accounts", "excluded", "candidates", "selected"):
            self.assertEqual(reversed_report[field], report[field])

    def test_missing_and_null_players_ratings_and_ai_precedence(self):
        rows = []
        for field in ("players", "sente", "gote", "user"):
            for null in (False, True):
                row = game(f"{field}-{null}")
                parent = row if field == "players" else row["players"] if field in ("sente", "gote") else row["players"]["sente"]
                if null:
                    parent[field] = None
                else:
                    del parent[field]
                rows.append(row)
        for value in (None, "missing", "provisional"):
            row = game(str(value))
            if value is None:
                row["players"]["sente"]["rating"] = None
            elif value == "missing":
                del row["players"]["gote"]["rating"]
            else:
                row["players"]["sente"]["provisional"] = True
            rows.append(row)
        ai = game("ai-with-anonymous")
        ai["players"]["sente"]["aiLevel"] = 0
        ai["players"]["gote"] = None
        rows.append(ai)
        self.source.write_bytes(encode(rows))
        report = select_games(self.source, self.output, self.report)
        expected = dict.fromkeys(REASONS, 0)
        expected.update(anonymous=8, rating=3, ai=1)
        self.assertEqual(report["excluded"], expected)

    def test_cap_follows_seeded_id_order_for_both_players(self):
        rows = [game(f"a-{i:03}", "alice", f"a-peer-{i}") for i in range(130)]
        rows += [game(f"b-{i:03}", f"b-peer-{i}", "bob") for i in range(130)]
        rows += [game(f"shared-{i:03}", "alice", "bob") for i in range(60)]
        # 指示書が定める手順から期待値を独立に計算する。
        ordered = sorted(rows, key=lambda row: row["id"])
        random.Random(20261113).shuffle(ordered)
        counts = Counter()
        chosen = []
        for row in ordered:
            ids = [row["players"][side]["user"]["id"] for side in ("sente", "gote")]
            if all(counts[user] < 100 for user in ids):
                chosen.append(row["id"])
                counts.update(ids)
        chosen.sort()
        self.source.write_bytes(encode(reversed(rows)))
        report = select_games(self.source, self.output, self.report)
        self.assertEqual(report["selected"]["ids"], chosen)
        self.assertEqual(report["selected"]["games"], len(chosen))
        self.assertEqual(report["selected"]["players"], len(counts))
        self.assertEqual(report["selected"]["max_player_games"], 100)
        self.assertEqual(report["selected"]["max_player_share"], 100 / len(chosen))
        self.assertEqual(report["selected"]["plies"], 40 * len(chosen))
        self.assertEqual(report["candidates"], {"games": 320, "players": 262, "max_player_games": 190, "max_player_share": 190 / 320})
        self.assertEqual(report["excluded"], {**dict.fromkeys(REASONS, 0), "player_cap": 320 - len(chosen)})
        self.assertEqual([json.loads(line)["id"] for line in self.output.read_bytes().splitlines()], chosen)
        # 入力順を変えても採否は変わらない。
        self.source.write_bytes(encode(rows))
        second = select_games(self.source, self.root / "second.ndjson", self.root / "second.json")
        self.assertEqual(second["selected"], report["selected"])

    def test_original_bytes_sorted_output_and_every_report_field(self):
        rows = [game("z", "alice", "bob"), game("a", "alice", "carol"), game("m", "dave", "carol")]
        rows[0]["note"] = "日本語"
        rows[0]["players"]["sente"]["aiLevel"] = None
        rows[1]["status"] = "royalsLost"
        rows[2]["status"] = "bareKing"
        raw = [("  " + json.dumps(row, ensure_ascii=False, separators=(", ", " : ")) + " \n").encode() for row in rows]
        raw[1] = raw[1][:-1] + b"\r\n"
        source = b"".join(raw)
        self.source.write_bytes(source)
        report = select_games(self.source, self.output, self.report)
        self.assertEqual(self.output.read_bytes(), raw[1] + raw[2] + raw[0])
        summary = {"games": 3, "players": 4, "max_player_games": 2, "max_player_share": 2 / 3}
        self.assertEqual(report, {
            "input": {"path": str(self.source), "size": len(source), "sha256": hashlib.sha256(source).hexdigest(), "games": 3},
            "parameters": {"min_rating": 1600, "min_plies": 40, "player_cap": 100, "seed": 20261113, "statuses": ["bareKing", "resign", "royalsLost"]},
            "bot_accounts": [], "excluded": dict.fromkeys(REASONS, 0),
            "candidates": summary, "selected": {**summary, "plies": 120, "ids": ["a", "m", "z"]},
        })
        self.assertEqual(json.loads(self.report.read_bytes()), report)

    def test_existing_output_or_report_is_unchanged(self):
        self.source.write_bytes(encode([game("valid")]))
        for existing in ((self.output,), (self.report,), (self.output, self.report)):
            with self.subTest(existing=existing):
                for path in existing:
                    path.write_bytes(b"keep these bytes")
                result = self.command()
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, b"")
                self.assertEqual(len(result.stderr.splitlines()), 1)
                for path in (self.output, self.report):
                    if path in existing:
                        self.assertEqual(path.read_bytes(), b"keep these bytes")
                        path.unlink()
                    else:
                        self.assertFalse(path.exists())

    def test_duplicate_game_id_and_invalid_user_id_are_errors_even_on_excluded_rows(self):
        duplicate = [game("same"), game("same")]
        duplicate[1]["variant"] = "standard"
        cases = [duplicate]
        for value in (None, 7, "missing"):
            row = game("bad-user")
            row["variant"] = "standard"
            if value == "missing":
                del row["players"]["gote"]["user"]["id"]
            else:
                row["players"]["gote"]["user"]["id"] = value
            cases.append([row])
        for rows in cases:
            with self.subTest(rows=rows):
                self.source.write_bytes(encode(rows))
                self.assert_error(self.command())

    def test_empty_game_id_or_game_id_with_whitespace_is_an_input_error(self):
        for game_id in ("", " ", "bad id", " id", "id ", "id\tpart", "id\npart", "id\rpart",
                        "id\u0085part", "id\u00a0part", "id\u2028part", "id\u3000part"):
            with self.subTest(game_id=game_id):
                invalid = game(game_id)
                invalid["variant"] = "standard"
                self.assert_input_error(encode([game("valid"), invalid]), 2)

    def test_game_ids_allow_characters_outside_unicode_white_space(self):
        # input.rsのchar::is_whitespaceはU+001C〜U+001Fやゼロ幅スペースを含まない。
        rows = [game(game_id) for game_id in (
            "日本語", "id\u200bpart", "id\x1cpart", "id\x1dpart", "id\x1epart", "id\x1fpart",
        )]
        self.source.write_bytes(encode(rows))
        report = select_games(self.source, self.output, self.report)
        self.assertEqual(report["excluded"], dict.fromkeys(REASONS, 0))
        expected = sorted(rows, key=lambda row: row["id"])
        self.assertEqual(report["selected"]["ids"], [row["id"] for row in expected])
        self.assertEqual(self.output.read_bytes(), encode(expected))

    def test_non_integer_and_out_of_i32_range_ratings_are_input_errors(self):
        for side in ("sente", "gote"):
            for rating in (1600.0, 1600.5, -(2**31) - 1, 2**31, True, False, "1600", [], {}):
                with self.subTest(side=side, rating=rating):
                    invalid = game("bad-rating")
                    invalid["players"][side]["rating"] = rating
                    self.assert_input_error(encode([game("valid"), invalid]), 2)

    def test_invalid_rating_is_checked_before_exclusion(self):
        variant = game("variant")
        variant["variant"] = "standard"
        provisional = game("provisional")
        provisional["players"]["gote"]["provisional"] = True
        missing = game("missing")
        del missing["players"]["sente"]["rating"]
        for invalid in (variant, provisional, missing):
            with self.subTest(game_id=invalid["id"]):
                invalid["players"]["gote"]["rating"] = 1600.0
                self.assert_input_error(encode([game("valid"), invalid]), 2)

    def test_i32_rating_boundaries_are_valid_input(self):
        minimum = game("minimum")
        minimum["players"]["sente"]["rating"] = -(2**31)
        maximum = game("maximum")
        for player in maximum["players"].values():
            player["rating"] = 2**31 - 1
        self.source.write_bytes(encode([minimum, maximum]))
        report = select_games(self.source, self.output, self.report)
        self.assertEqual(report["excluded"], {**dict.fromkeys(REASONS, 0), "rating": 1})
        self.assertEqual(report["selected"]["ids"], ["maximum"])
        self.assertEqual(self.output.read_bytes(), encode([maximum]))

    def test_final_line_without_lf_is_an_input_error_even_if_excluded(self):
        for variant in ("chushogi", "standard"):
            for ending in (b"", b"\r"):
                with self.subTest(variant=variant, ending=ending):
                    last = game("last")
                    last["variant"] = variant
                    raw = encode([game("valid"), last])[:-1] + ending
                    self.assert_input_error(raw, 2)

    def test_failed_report_creation_removes_created_output(self):
        self.source.write_bytes(encode([game("valid")]))
        result = self.command(report=self.root / "missing" / "report.json")
        self.assert_error(result)

    def test_invalid_json_and_missing_arguments_report_one_line(self):
        for raw in (b"{\n", b"null\n", b"\xff\n", b"\n"):
            with self.subTest(raw=raw):
                self.source.write_bytes(raw)
                self.assert_error(self.command())
        result = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, check=False)
        self.assert_error(result)

    def test_empty_input_has_zero_counts_and_a_hash(self):
        self.source.write_bytes(b"")
        report = select_games(self.source, self.output, self.report)
        self.assertEqual(report["input"], {"path": str(self.source), "size": 0, "sha256": hashlib.sha256(b"").hexdigest(), "games": 0})
        self.assertEqual(report["selected"], {"games": 0, "players": 0, "max_player_games": 0, "max_player_share": 0.0, "plies": 0, "ids": []})
        self.assertEqual(report["excluded"], dict.fromkeys(REASONS, 0))


if __name__ == "__main__":
    unittest.main()
