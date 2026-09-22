#!/usr/bin/env python3
"""Collect unchanged lishogi chushogi game records by breadth-first traversal.

Use the same --output and --state paths to resume. --max-users limits each run.
The state commits one user's output at a time; an interrupted user's uncommitted
output is removed on restart and that user is fetched again. Run only one process
per output/state pair. Seed options apply only when creating a new state.
"""

import argparse
import json
import math
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


BASE_URL = "https://lishogi.org"
USER_AGENT = "minase-research/0.1 (chushogi teacher data; contact via GitHub stepney141)"


def open_http(request):
    """HTTP boundary, replaceable by an in-memory response in tests."""
    return urllib.request.urlopen(request, timeout=60)


class Client:
    def __init__(self, interval, user_agent, stats, *, http=open_http, sleep=time.sleep):
        self.interval = interval
        self.user_agent = user_agent
        self.stats = stats
        self.http = http
        self.sleep = sleep
        self.requested = False

    def records(self, path, *, ndjson=True):
        request = urllib.request.Request(
            BASE_URL + path,
            headers={
                "Accept": "application/x-ndjson" if ndjson else "application/json",
                "User-Agent": self.user_agent,
            },
        )
        # Waiting after the preceding response closes also spaces long streams.
        delay = self.interval if self.requested else 0
        while True:
            if delay:
                self.sleep(delay)
            self.requested = True
            self.stats["requests"] += 1
            try:
                with self.http(request) as response:
                    if ndjson:
                        for line in response:
                            if line.strip():
                                yield json.loads(line)
                    else:
                        yield from json.load(response)["users"]
                self._transient_attempts = 0
                return
            except urllib.error.HTTPError as error:
                self.stats["errors"] += 1
                error.close()
                if error.code != 429:
                    raise
                delay = max(60, self.interval)
                print(f"HTTP 429: {path}; retrying in {delay:g}s", file=sys.stderr)
            except (urllib.error.URLError, TimeoutError) as error:
                # Read timeouts and connection resets are transient; retry a few
                # times after the same pause as a 429 before giving up on the path.
                # Other connection failures are reported to the caller at once.
                self.stats["errors"] += 1
                reason = getattr(error, "reason", error)
                if not isinstance(reason, (TimeoutError, ConnectionResetError)):
                    raise
                attempts = getattr(self, "_transient_attempts", 0) + 1
                self._transient_attempts = attempts
                if attempts > 5:
                    self._transient_attempts = 0
                    raise
                delay = max(60, self.interval)
                print(f"{error}: {path}; retrying in {delay:g}s", file=sys.stderr)


def csv_ids(value):
    return [item.strip().lower() for item in value.split(",") if item.strip()]


def interval_seconds(value):
    result = float(value)
    if not math.isfinite(result) or result < 1.5:
        raise argparse.ArgumentTypeError("interval must be finite and at least 1.5 seconds")
    return result


def positive_int(value):
    result = int(value)
    if result < 1:
        raise argparse.ArgumentTypeError("max-users must be positive")
    return result


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--output", required=True, type=Path)
    result.add_argument("--state", required=True, type=Path)
    result.add_argument("--seed-users", type=csv_ids, default=[])
    result.add_argument("--seed-teams", type=csv_ids, default="chu-shogi-club,chushogi-wc")
    result.add_argument("--leaderboard", action=argparse.BooleanOptionalAction, default=True)
    result.add_argument("--interval", type=interval_seconds, default=1.5)
    result.add_argument("--max-users", type=positive_int)
    result.add_argument("--user-agent", default=USER_AGENT)
    return result


def save_state(path, state):
    temporary = path.with_name(path.name + ".tmp")
    with temporary.open("w", encoding="utf-8") as output:
        json.dump(state, output, ensure_ascii=False, indent=2)
        output.write("\n")
    temporary.replace(path)


def collect(args, *, http=open_http, sleep=time.sleep):
    started = time.monotonic()
    output_path = args.output.resolve()
    state_path = args.state.resolve()
    temporary_path = state_path.with_name(state_path.name + ".tmp")
    if output_path in (state_path, temporary_path):
        raise ValueError("output must differ from state and its temporary file")
    output_path.parent.mkdir(parents=True, exist_ok=True)
    state_path.parent.mkdir(parents=True, exist_ok=True)
    resumed = state_path.exists()
    if resumed:
        state = json.loads(state_path.read_text(encoding="utf-8"))
        if state["output"] != str(output_path):
            raise ValueError("state belongs to a different output path")
        if not output_path.exists() or output_path.stat().st_size < state["output_bytes"]:
            raise ValueError("output is missing or shorter than its saved state")
    else:
        if output_path.exists() and output_path.stat().st_size:
            raise ValueError("nonempty output requires its matching state file")
        state = {
            "output": str(output_path),
            "output_bytes": 0,
            "visited": [],
            "queue": [],
            "game_ids": [],
            "stats": {"users": 0, "games": 0, "requests": 0, "errors": 0, "elapsed_seconds": 0.0},
            "failures": [],
        }

    visited = set(state["visited"])
    queued = set(state["queue"])
    game_ids = set(state["game_ids"])
    stats = state["stats"]
    previous_elapsed = stats["elapsed_seconds"]
    client = Client(args.interval, args.user_agent, stats, http=http, sleep=sleep)
    # A restarted process also observes the interval before its first request.
    client.requested = resumed

    def enqueue(user_id):
        user_id = user_id.lower()
        if user_id not in visited and user_id not in queued:
            state["queue"].append(user_id)
            queued.add(user_id)

    def record_failure(source, error):
        status = error.code if isinstance(error, urllib.error.HTTPError) else None
        state["failures"].append({"source": source, "status": status, "message": str(error)})
        print(f"{source}: {error}; continuing", file=sys.stderr)

    if not resumed:
        for user_id in args.seed_users:
            enqueue(user_id)
        if args.leaderboard:
            try:
                for user in client.records("/api/player/top/200/chushogi", ndjson=False):
                    enqueue(user["id"])
            except urllib.error.URLError as error:
                record_failure("leaderboard", error)
        for team_id in args.seed_teams:
            path = f"/api/team/{urllib.parse.quote(team_id, safe='')}/users"
            try:
                for user in client.records(path):
                    enqueue(user["id"])
            except urllib.error.URLError as error:
                record_failure(f"team:{team_id}", error)

    with output_path.open("r+b" if output_path.exists() else "w+b") as output:
        output.truncate(state["output_bytes"])
        output.seek(state["output_bytes"])

        def checkpoint():
            output.flush()
            state["output_bytes"] = output.tell()
            state["visited"] = sorted(visited)
            state["game_ids"] = sorted(game_ids)
            stats["users"] = len(visited)
            stats["games"] = len(game_ids)
            stats["elapsed_seconds"] = previous_elapsed + time.monotonic() - started
            save_state(state_path, state)

        checkpoint()
        processed = 0
        while state["queue"] and (args.max_users is None or processed < args.max_users):
            user_id = state["queue"][0]
            path = f"/api/games/user/{urllib.parse.quote(user_id, safe='')}?perfType=chushogi&moves=true"
            try:
                for game in client.records(path):
                    game_id = game["id"]
                    if game_id not in game_ids:
                        output.write((json.dumps(game, ensure_ascii=False) + "\n").encode("utf-8"))
                        game_ids.add(game_id)
                    # Anonymous players and older exports may omit these fields.
                    if "players" in game:
                        for side in ("sente", "gote"):
                            if side not in game["players"]:
                                continue
                            player = game["players"][side]
                            if "user" in player and "id" in player["user"]:
                                enqueue(player["user"]["id"])
            except (urllib.error.URLError, TimeoutError) as error:
                record_failure(f"user:{user_id}", error)
            visited.add(user_id)
            state["queue"].pop(0)
            queued.remove(user_id)
            processed += 1
            checkpoint()

    print(
        f"Visited users: {stats['users']}; games: {stats['games']}; "
        f"requests: {stats['requests']}; errors: {stats['errors']}; "
        f"elapsed: {stats['elapsed_seconds']:.1f}s"
    )
    return state


def main():
    args = parser().parse_args()
    collect(args)


if __name__ == "__main__":
    main()
