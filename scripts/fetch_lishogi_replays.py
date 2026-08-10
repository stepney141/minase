#!/usr/bin/env python3
"""lishogi棋譜リプレイ照合フィクスチャの取得スクリプト。

対象棋譜は下記GAME_IDSで固定する。lishogi APIから各棋譜を取得し、
tests/fixtures/lishogi_replays.ndjson.gz を再生成する。
実行はコーパス更新時に限る（docs/plans/protocol-layer.md フェーズ4）。
"""

import gzip
import json
import sys
import time
import urllib.request
from pathlib import Path

# 終局理由の内訳: royalsLost 3局、bareKing 2局、repetition 1局、
# draw(合意) 2局、resign 2局。lishogiは王駒実捕獲制(E1)のため
# status "mate" の中将棋棋譜は存在しない(2026年8月10日時点の
# 上位8プレイヤー695局の観測)。
GAME_IDS = [
    "SNjoPiHz",  # royalsLost 後手勝ち 22手(経由升が空の獅子2段移動を含む)
    "Lqyj1bLC",  # royalsLost 先手勝ち 81手
    "msxNDjN8",  # royalsLost 後手勝ち 620手
    "hgNaEt5P",  # bareKing 後手勝ち 328手
    "VoPpZxG5",  # bareKing 先手勝ち 341手
    "OGH2sJc2",  # repetition 引き分け 103手
    "UUnYczs0",  # draw 合意引き分け 27手
    "mU1oGkUg",  # draw 合意引き分け 367手
    "gZ0HcfLK",  # resign 後手勝ち 18手
    "2u7dwJf9",  # resign 先手勝ち 555手(経由升が空の獅子2段移動を含む)
]

STANDARD_INITIAL_SFEN = (
    "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/"
    "3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b - 1"
)

EXPORT_URL = "https://lishogi.org/game/export/{game_id}?moves=true"


def fetch_game(game_id: str) -> dict:
    request = urllib.request.Request(
        EXPORT_URL.format(game_id=game_id),
        headers={"Accept": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        game = json.load(response)
    if game["variant"] != "chushogi":
        raise ValueError(f"{game_id}: variant is {game['variant']}, not chushogi")
    if "initialFen" in game:
        raise ValueError(f"{game_id}: non-standard initial position is unsupported")
    return {
        "id": game["id"],
        "initial_sfen": STANDARD_INITIAL_SFEN,
        "moves": game["moves"],
        "status": game["status"],
        "winner": game.get("winner"),
    }


def main() -> None:
    output_path = (
        Path(__file__).resolve().parent.parent
        / "tests"
        / "fixtures"
        / "lishogi_replays.ndjson.gz"
    )
    output_path.parent.mkdir(parents=True, exist_ok=True)
    lines = []
    for index, game_id in enumerate(GAME_IDS):
        if index > 0:
            time.sleep(2)
        record = fetch_game(game_id)
        lines.append(json.dumps(record, ensure_ascii=False, sort_keys=True))
        print(f"fetched {game_id}: {record['status']}", file=sys.stderr)
    payload = ("\n".join(lines) + "\n").encode("utf-8")
    with output_path.open("wb") as file:
        with gzip.GzipFile(fileobj=file, mode="wb", mtime=0) as archive:
            archive.write(payload)
    print(f"wrote {output_path} ({output_path.stat().st_size} bytes)", file=sys.stderr)


if __name__ == "__main__":
    main()
