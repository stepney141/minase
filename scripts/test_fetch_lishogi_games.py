"""Offline acceptance tests derived from task-phase5-fetch.md.

Behavior matrix: connected users -> breadth-first requests; shared games -> one
unchanged record; HTTP 429 -> same request after >=60s; other HTTP failures ->
record and continue; per-user checkpoint -> resume without repeats; interrupted
user -> refetch without duplicate output. CLI boundaries protect the 1.5s floor.
"""

import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

from scripts import fetch_lishogi_games as fetch


def game(game_id, sente, gote, **extra):
    return {
        "id": game_id,
        "players": {"sente": {"user": {"id": sente}}, "gote": {"user": {"id": gote}}},
        **extra,
    }


def games_path(user):
    return f"/api/games/user/{user}?perfType=chushogi&moves=true"


class MockHTTP:
    def __init__(self, routes):
        self.routes = routes
        self.calls = []
        self.events = []
        self.responses = []

    def __call__(self, request):
        # Even in the test, opening a second simultaneous response is forbidden.
        if any(not response.closed for response in self.responses):
            raise AssertionError("overlapping HTTP requests")
        path = request.full_url.removeprefix("https://lishogi.org")
        self.calls.append(path)
        self.events.append(("request", path))
        reply = self.routes[path].pop(0)
        if isinstance(reply, int):
            raise urllib.error.HTTPError(request.full_url, reply, "mock failure", {}, None)
        if isinstance(reply, BaseException):
            raise reply
        if isinstance(reply, dict):
            payload = json.dumps(reply)
            if request.get_header("Accept") != "application/json":
                raise AssertionError("leaderboard requires JSON")
        else:
            payload = "\n".join(json.dumps(record, ensure_ascii=False) for record in reply) + "\n"
            if request.get_header("Accept") != "application/x-ndjson":
                raise AssertionError("exports require NDJSON")
        if request.get_header("User-agent") != fetch.USER_AGENT:
            raise AssertionError("missing configured User-Agent")
        response = io.BytesIO(payload.encode("utf-8"))
        self.responses.append(response)
        return response

    def sleep(self, seconds):
        self.events.append(("sleep", seconds))


class FetchTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.output = Path(self.directory.name) / "games.ndjson"
        self.state = Path(self.directory.name) / "state.json"
        self.stdout = io.StringIO()
        self.stderr = io.StringIO()
        self.enterContext(contextlib.redirect_stdout(self.stdout))
        self.enterContext(contextlib.redirect_stderr(self.stderr))
        # Accidental real I/O or waiting must fail rather than pass unnoticed.
        self.enterContext(patch("urllib.request.urlopen", side_effect=AssertionError("network forbidden")))
        self.enterContext(patch("time.sleep", side_effect=AssertionError("real waiting forbidden")))

    def args(self, *extra):
        return fetch.parser().parse_args([
            "--output", str(self.output), "--state", str(self.state),
            "--no-leaderboard", "--seed-teams", "", "--seed-users", "alice", *extra,
        ])

    def run_fetch(self, mock, *extra):
        return fetch.collect(self.args(*extra), http=mock, sleep=mock.sleep)

    def output_games(self):
        return [json.loads(line) for line in self.output.read_text(encoding="utf-8").splitlines()]

    def test_breadth_first_deduplication_and_unmodified_sparse_records(self):
        ab = game("ab", "alice", "bob", initialSfen="custom position", moves="a1a2", note="棋譜")
        ac = game("ac", "alice", "carol")
        bd = game("bd", "bob", "dave")
        sparse = {"id": "old", "status": "draw"}
        anonymous = {"id": "anon", "players": {"sente": {"name": "Anonymous"}}}
        mock = MockHTTP({
            games_path("alice"): [[ab, ac, ab, sparse, anonymous]],
            games_path("bob"): [[ab, bd]],
            games_path("carol"): [[ac]],
            games_path("dave"): [[bd]],
        })
        state = self.run_fetch(mock)
        self.assertEqual(mock.calls, [games_path(user) for user in ["alice", "bob", "carol", "dave"]])
        self.assertEqual(self.output_games(), [ab, ac, sparse, anonymous, bd])
        self.assertEqual([event for event in mock.events if event[0] == "sleep"], [("sleep", 1.5)] * 3)
        self.assertEqual(state["stats"]["users"], 4)
        self.assertEqual(state["stats"]["games"], 5)
        self.assertEqual(state["stats"]["requests"], 4)
        self.assertEqual(state["queue"], [])
        self.assertIn("Visited users: 4; games: 5; requests: 4; errors: 0; elapsed:", self.stdout.getvalue())

    def test_leaderboard_and_default_teams_seed_users_once(self):
        mock = MockHTTP({
            "/api/player/top/200/chushogi": [{"users": [{"id": "Alice"}, {"id": "bob"}]}],
            "/api/team/chu-shogi-club/users": [[{"id": "bob"}, {"id": "carol"}]],
            "/api/team/chushogi-wc/users": [[{"id": "dave"}]],
            **{games_path(user): [[]] for user in ["alice", "bob", "carol", "dave"]},
        })
        args = fetch.parser().parse_args(["--output", str(self.output), "--state", str(self.state)])
        fetch.collect(args, http=mock, sleep=mock.sleep)
        self.assertEqual(mock.calls[3:], [games_path(user) for user in ["alice", "bob", "carol", "dave"]])
        self.assertEqual([event for event in mock.events if event[0] == "sleep"], [("sleep", 1.5)] * 6)

    def test_429_retries_same_request_after_60_seconds(self):
        path = games_path("alice")
        mock = MockHTTP({path: [429, 429, []]})
        state = self.run_fetch(mock)
        self.assertEqual(mock.events, [
            ("request", path), ("sleep", 60), ("request", path),
            ("sleep", 60), ("request", path),
        ])
        self.assertEqual(state["stats"]["requests"], 3)
        self.assertEqual(state["stats"]["errors"], 2)
        self.assertIn("HTTP 429", self.stderr.getvalue())

    def test_interval_above_60_is_respected_on_retry(self):
        mock = MockHTTP({games_path("alice"): [429, []]})
        self.run_fetch(mock, "--interval", "75")
        self.assertEqual(mock.events[1], ("sleep", 75))

    def test_http_errors_are_recorded_and_other_users_continue(self):
        mock = MockHTTP({
            "/api/player/top/200/chushogi": [404],
            "/api/team/missing/users": [404],
            games_path("alice"): [503],
            games_path("bob"): [[{"id": "old"}]],
        })
        state = self.run_fetch(mock, "--leaderboard", "--seed-teams", "missing", "--seed-users", "alice,bob")
        self.assertEqual(state["visited"], ["alice", "bob"])
        self.assertEqual(state["stats"]["errors"], 3)
        self.assertEqual(state["failures"], json.loads(self.state.read_text())["failures"])
        self.assertEqual([item["source"] for item in state["failures"]], ["leaderboard", "team:missing", "user:alice"])
        self.assertEqual(self.output_games(), [{"id": "old"}])
        self.assertIn("leaderboard:", self.stderr.getvalue())

    def test_resume_uses_saved_queue_and_ids_without_reseeding(self):
        ab = game("ab", "alice", "bob")
        bc = game("bc", "bob", "carol")
        first = MockHTTP({games_path("alice"): [[ab]]})
        self.run_fetch(first, "--max-users", "1")
        before = self.output.read_bytes()
        second = MockHTTP({games_path("bob"): [[ab, bc]]})
        state = self.run_fetch(second, "--leaderboard", "--seed-teams", "ignored", "--max-users", "1")
        self.assertEqual(second.calls, [games_path("bob")])
        self.assertTrue(self.output.read_bytes().startswith(before))
        self.assertEqual(self.output_games(), [ab, bc])
        self.assertEqual(state["queue"], ["carol"])
        self.assertEqual(state["stats"]["requests"], 2)
        self.assertEqual(state["stats"]["users"], 2)
        third = MockHTTP({games_path("carol"): [[bc]]})
        self.run_fetch(third)
        self.assertEqual(self.output_games(), [ab, bc])

    def test_leaderboard_connection_failure_keeps_explicit_seed(self):
        mock = MockHTTP({
            "/api/player/top/200/chushogi": [urllib.error.URLError("connection refused")],
            games_path("alice"): [[]],
        })
        state = self.run_fetch(mock, "--leaderboard")
        self.assertEqual(state["visited"], ["alice"])
        self.assertEqual(state["stats"]["errors"], 1)
        self.assertEqual(state["failures"][0]["source"], "leaderboard")
        self.assertIn("connection refused", self.stderr.getvalue())

    def test_state_is_replaced_only_after_complete_json_is_written(self):
        old = {"visited": ["alice"]}
        new = {"visited": ["alice", "bob"]}
        self.state.write_text(json.dumps(old))
        replace = Path.replace
        observations = []

        def inspect_replace(temporary, target):
            observations.append((json.loads(target.read_text()), json.loads(temporary.read_text())))
            return replace(temporary, target)

        with patch.object(Path, "replace", inspect_replace):
            fetch.save_state(self.state, new)
        self.assertEqual(observations, [(old, new)])
        self.assertEqual(json.loads(self.state.read_text()), new)

    def test_interrupted_user_is_refetched_without_duplicates(self):
        ab = game("ab", "alice", "bob")
        mock = MockHTTP({games_path("alice"): [[ab]]})
        original_save = fetch.save_state

        def interrupt_checkpoint(path, state):
            if state["stats"]["users"] == 1:
                raise KeyboardInterrupt
            original_save(path, state)

        with patch.object(fetch, "save_state", side_effect=interrupt_checkpoint):
            with self.assertRaises(KeyboardInterrupt):
                self.run_fetch(mock, "--max-users", "1")
        self.assertEqual(self.output_games(), [ab])
        # Also simulate termination in the middle of the following JSON write.
        with self.output.open("ab") as output:
            output.write(b'{"id": "partial')
        retry = MockHTTP({games_path("alice"): [[ab]], games_path("bob"): [[ab]]})
        self.run_fetch(retry)
        self.assertEqual(self.output_games(), [ab])
        self.assertEqual(retry.calls, [games_path("alice"), games_path("bob")])

    def test_resume_rejects_missing_output_and_leaves_state_unchanged(self):
        self.run_fetch(MockHTTP({games_path("alice"): [[{"id": "one"}]]}))
        saved = self.state.read_bytes()
        self.output.unlink()
        with self.assertRaises(ValueError):
            self.run_fetch(MockHTTP({}))
        self.assertEqual(self.state.read_bytes(), saved)

    def test_nonempty_output_without_state_is_not_overwritten(self):
        self.output.write_text('{"id":"existing"}\n')
        before = self.output.read_bytes()
        with self.assertRaises(ValueError):
            self.run_fetch(MockHTTP({}))
        self.assertEqual(self.output.read_bytes(), before)

    def test_argument_boundaries(self):
        self.assertEqual(self.args("--interval", "1.5").interval, 1.5)
        for interval in ["1.49", "0", "-1", "nan", "inf"]:
            with self.subTest(interval=interval), self.assertRaises(SystemExit):
                self.args("--interval", interval)
        for limit in ["0", "-1"]:
            with self.subTest(limit=limit), self.assertRaises(SystemExit):
                self.args("--max-users", limit)


if __name__ == "__main__":
    unittest.main()
