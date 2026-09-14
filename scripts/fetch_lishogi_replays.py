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

# 終局理由の内訳: royalsLost 4局、bareKing 2局、repetition 3局、
# draw(合意) 2局、resign 2局。lishogiは王駒実捕獲制(E1)のため
# status "mate" の中将棋棋譜は存在しない(2026年8月10日時点の
# 上位8プレイヤー695局の観測)。
# 末尾の3局は、反復裁定の前提条件(RULES.md第31条R1、直前の不可逆手から
# 可逆手が12手以上続いていること)で裁定が分かれる対局である
# (docs/plans/lishogi-bot.md「反復裁定の整合」)。
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
    "uy7y6mP6",  # royalsLost 後手勝ち 128手(9手目で同一局面が4回目に現れるが可逆手が8手のため裁定されない)
    "EHUTJJu4",  # repetition 引き分け 12手(途中局面開始、じっとの往復で可逆手12手目に裁定)
    "A4EO2swa",  # repetition 引き分け 12手(途中局面開始、じっとの往復で可逆手12手目に裁定)
]

# lishogiは、scalashogiのコミットfe9ccf8(2023年5月20日)で反復による終局に
# status "repetition" を割り当てるまで、同じ終局を status "draw" で記録していた。
# それ以前に反復で終局した対局は、フィクスチャでは "repetition" として保存する。
REPETITION_RECORDED_AS_DRAW = {"EHUTJJu4", "A4EO2swa"}
REPETITION_STATUS_SINCE_MS = 1684540800000  # 2023-05-20T00:00:00Z

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
        raise ValueError(f"{game_id}: legacy initialFen field is unsupported")
    # lishogi APIは標準初期局面から始まる対局でinitialSfenを省略する。
    initial_sfen = game.get("initialSfen", STANDARD_INITIAL_SFEN)
    status = game["status"]
    if game_id in REPETITION_RECORDED_AS_DRAW:
        if status != "draw" or game["createdAt"] >= REPETITION_STATUS_SINCE_MS:
            raise ValueError(f"{game_id}: not a pre-2023-05-20 draw-status game")
        status = "repetition"
    return {
        "id": game["id"],
        "initial_sfen": initial_sfen,
        "moves": game["moves"],
        "status": status,
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
