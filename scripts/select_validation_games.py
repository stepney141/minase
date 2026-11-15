#!/usr/bin/env python3
"""lishogiの人間同士の棋譜を固定条件で選び、原文と選別の集計を書き出す。

入力は1行1局のUTF-8 JSONとする。全行で棋譜idの重複とuser.idを検査し、
10種類の条件を順に適用した後、固定シードで並べ替えて1人100局まで採る。
出力の棋譜はid順に並べ、入力のバイト列を保つ。末尾にLFがなければ補う。
既存の出力は上書きせず、失敗時にはこの実行で作成したファイルだけを消す。
"""

import argparse
from collections import Counter
from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import random
import sys


MIN_RATING = 1600
MIN_PLIES = 40
PLAYER_CAP = 100
SEED = 20261113
STATUSES = ("bareKing", "resign", "royalsLost")
REASONS = (
    "variant", "initial_sfen", "ai", "anonymous", "bot", "unrated",
    "rating", "status", "winner", "short", "player_cap",
)


class SelectionError(ValueError):
    """入力または引数が選別の契約に違反したことを表す。"""


class ArgumentParser(argparse.ArgumentParser):
    """引数の誤りも標準エラー1行と終了コード1で報告する。"""

    def error(self, message):
        raise SelectionError(message)


@dataclass(frozen=True)
class Candidate:
    """選別条件を通った棋譜の識別子、対局者、手数と原文を保持する。"""

    id: str
    players: frozenset[str]
    plies: int
    raw: bytes


def object_or_none(value, field):
    """省略またはnullを許すオブジェクト欄を検査する。"""
    if value is not None and not isinstance(value, dict):
        raise SelectionError(f"{field} must be an object or null")
    return value


def read_players(game, bot_accounts):
    """全行の対局者idを検査し、選別にかかわらずBOTのidを集める。"""
    players = object_or_none(game.get("players"), "players")
    sides = []
    for side in ("sente", "gote"):
        player = None if players is None else object_or_none(players.get(side), f"players.{side}")
        if player is not None:
            user = object_or_none(player.get("user"), f"players.{side}.user")
            if user is not None:
                if not isinstance(user.get("id"), str):
                    raise SelectionError(f"players.{side}.user.id must be a string")
                if user.get("title") == "BOT":
                    bot_accounts.add(user["id"])
        sides.append(player)
    return sides


def exclusion(game, players):
    """最初に該当する除外理由を返す。採用候補ならNoneを返す。"""
    if game.get("variant") != "chushogi":
        return "variant"
    if "initialSfen" in game:
        return "initial_sfen"
    if any(p is not None and p.get("aiLevel") is not None for p in players):
        return "ai"
    if any(p is None or p.get("user") is None for p in players):
        return "anonymous"
    if any(p["user"].get("title") == "BOT" for p in players):
        return "bot"
    if game.get("rated") is not True:
        return "unrated"
    for player in players:
        rating = player.get("rating")
        if rating is None or player.get("provisional") is True:
            return "rating"
        if (
            isinstance(rating, bool)
            or not isinstance(rating, (int, float))
            or (isinstance(rating, float) and not math.isfinite(rating))
        ):
            raise SelectionError("rating must be a finite number or null")
        if rating < MIN_RATING:
            return "rating"
    if game.get("status") not in STATUSES:
        return "status"
    if game.get("winner") not in ("sente", "gote"):
        return "winner"
    if not isinstance(game.get("moves"), str):
        raise SelectionError("moves must be a string")
    if len(game["moves"].split()) < MIN_PLIES:
        return "short"
    return None


def player_summary(games):
    """各人が関わる対局数を数え、最多の対局数と全対局に占める割合を返す。"""
    counts = Counter(player for game in games for player in game.players)
    maximum = max(counts.values()) if counts else 0
    return {
        "games": len(games),
        "players": len(counts),
        "max_player_games": maximum,
        "max_player_share": maximum / len(games) if games else 0.0,
    }


def select_games(input_path, output_path, report_path):
    """入力を選別し、新規の棋譜ファイルとJSON報告を保存して集計を返す。"""
    for path in (output_path, report_path):
        if os.path.lexists(path):
            raise FileExistsError(f"output already exists: {path}")
    excluded = dict.fromkeys(REASONS, 0)
    seen = set()
    bot_accounts = set()
    candidates = []
    digest = hashlib.sha256()
    size = 0
    line_count = 0
    with open(input_path, "rb") as source:
        for line_count, raw in enumerate(source, 1):
            size += len(raw)
            digest.update(raw)
            try:
                game = json.loads(raw.decode("utf-8"))
                if not isinstance(game, dict):
                    raise SelectionError("each line must be a JSON object")
                game_id = game.get("id")
                if not isinstance(game_id, str):
                    raise SelectionError("id must be a string")
                if game_id in seen:
                    raise SelectionError(f"duplicate game id: {game_id}")
                seen.add(game_id)
                players = read_players(game, bot_accounts)
                reason = exclusion(game, players)
                if reason is not None:
                    excluded[reason] += 1
                    continue
                candidates.append(Candidate(
                    game_id,
                    frozenset(p["user"]["id"] for p in players),
                    len(game["moves"].split()),
                    raw if raw.endswith(b"\n") else raw + b"\n",
                ))
            except (ValueError, UnicodeError) as error:
                raise SelectionError(f"NDJSON line {line_count}: {error}") from error

    candidates.sort(key=lambda game: game.id)
    random.Random(SEED).shuffle(candidates)
    counts = Counter()
    selected = []
    for game in candidates:
        if any(counts[player] >= PLAYER_CAP for player in game.players):
            excluded["player_cap"] += 1
            continue
        selected.append(game)
        counts.update(game.players)
    selected.sort(key=lambda game: game.id)
    report = {
        "input": {"path": str(input_path), "size": size, "sha256": digest.hexdigest(), "games": line_count},
        "parameters": {"min_rating": MIN_RATING, "min_plies": MIN_PLIES, "player_cap": PLAYER_CAP,
                       "seed": SEED, "statuses": list(STATUSES)},
        "bot_accounts": sorted(bot_accounts),
        "excluded": excluded,
        "candidates": player_summary(candidates),
        "selected": {**player_summary(selected), "plies": sum(game.plies for game in selected),
                     "ids": [game.id for game in selected]},
    }
    created = []
    try:
        with open(output_path, "xb") as output:
            created.append(Path(output_path))
            for game in selected:
                output.write(game.raw)
        with open(report_path, "x", encoding="utf-8") as output:
            created.append(Path(report_path))
            json.dump(report, output, ensure_ascii=False, indent=2)
            output.write("\n")
    except BaseException:
        for path in reversed(created):
            path.unlink()
        raise
    return report


def main():
    """必須の3パスを読み、件数または1行のエラーを出力する。"""
    parser = ArgumentParser(description=__doc__)
    for name in ("input", "output", "report"):
        parser.add_argument(f"--{name}", required=True, type=Path)
    try:
        args = parser.parse_args()
        report = select_games(args.input, args.output, args.report)
    except (OSError, ValueError) as error:
        print("error: " + " ".join(str(error).splitlines()), file=sys.stderr)
        return 1
    print(f"selected: {report['selected']['games']}")
    for reason, count in report["excluded"].items():
        print(f"{reason}: {count}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
