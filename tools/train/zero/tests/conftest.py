"""MNZTの試験で共有する、合成ファイルの書き出しと、Rustが生成した見本ファイルの場所。"""

from __future__ import annotations

import os
import struct
import subprocess
from pathlib import Path

import numpy as np
import pytest

from minase_zero.encoding import _KIND, Position, parse_sfen
from minase_zero.mnzt import HEADER_LEN, POLICY_DTYPE, RECORD_DTYPE

INITIAL_SFEN = (
    "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/"
    "3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b"
)
"""初期局面のSFEN。"""

COMMIT = "0123456789abcdef0123456789abcdef01234567"
"""合成ファイルの生成コミット欄。"""


def piece_code(piece) -> int:
    """Rustの`training::records::encode_piece`と同じ式で駒コードを返す。"""
    return 1 + _KIND[piece.kind] + 29 * int(piece.promoted) + 64 * piece.color


def board_codes(position: Position) -> np.ndarray:
    """局面の盤面を144バイトの駒コードの配列にする。"""
    return np.array(
        [0 if piece is None else piece_code(piece) for piece in position.board], dtype=np.uint8
    )


def write_mnzt(path: Path, records: list[dict], seed: int = 0) -> Path:
    """`records`を合成のMNZTとして書く。

    各要素は`game`、`ply`、`outcome`、`value`、`policy`（（ラベル、確率）の列）を持ち、
    任意で`sfen`（既定は初期局面）を持つ。
    """
    table = np.zeros(len(records), dtype=RECORD_DTYPE)
    entries = []
    for row, record in enumerate(records):
        position = parse_sfen(record.get("sfen", INITIAL_SFEN))
        table[row]["board"] = board_codes(position)
        table[row]["side_to_move"] = position.side_to_move
        lion = position.lion_capture
        table[row]["lion_square"] = 255 if lion is None else lion
        table[row]["outcome"] = record["outcome"]
        table[row]["game_number"] = record["game"]
        table[row]["ply"] = record["ply"]
        table[row]["policy_count"] = len(record["policy"])
        table[row]["value"] = record["value"]
        table[row]["policy_start"] = len(entries)
        entries.extend(record["policy"])
    policy = np.zeros(len(entries), dtype=POLICY_DTYPE)
    for index, (label, probability) in enumerate(entries):
        policy[index] = (label, round(probability * 65535))
    policy_offset = HEADER_LEN + table.nbytes
    header = bytearray(HEADER_LEN)
    struct.pack_into("<4sIII", header, 0, b"MNZT", 1, HEADER_LEN, table.dtype.itemsize)
    header[16:16 + len(b"L0,P0,R1,E0")] = b"L0,P0,R1,E0"
    header[48:88] = COMMIT.encode("ascii")
    header[120] = 0
    struct.pack_into(
        "<6Q", header, 128, 100, seed, len(records), len(entries), HEADER_LEN, policy_offset
    )
    path.write_bytes(bytes(header) + table.tobytes() + policy.tobytes())
    return path


def game_records(game: int, plies: int, outcome: int = 2) -> list[dict]:
    """1局分の合成レコードを作る。方策は手番によらず固定の2手とする。"""
    return [
        {
            "game": game,
            "ply": ply,
            "outcome": outcome if ply % 2 == 0 else 2 - outcome,
            "value": 0.5,
            "policy": [(ply % 100, 0.75), (200 + ply % 100, 0.25)],
        }
        for ply in range(plies)
    ]


def sample_mnzt_path() -> Path:
    """Rustの自己対局生成器が書いた見本MNZTの経路を返す。

    環境変数`MINASE_ZERO_SAMPLE_MNZT`があればその経路、なければ本体リポジトリの
    `data/alphazero-feasibility/sample.mnzt`である（worktreeでも本体の`data/`を指す）。
    """
    override = os.environ.get("MINASE_ZERO_SAMPLE_MNZT")
    if override:
        return Path(override)
    common = subprocess.run(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        cwd=Path(__file__).parent, capture_output=True, text=True, check=True,
    ).stdout.strip()
    return Path(common).parent / "data/alphazero-feasibility/sample.mnzt"


@pytest.fixture(scope="session")
def sample_mnzt() -> Path:
    """見本MNZTの経路。ない場合はスキップせずに失敗する。"""
    path = sample_mnzt_path()
    if not path.exists():
        pytest.fail(
            f"sample MNZT not found at {path}; generate it with "
            "`cargo run --release --bin minase -- data selfplay generate --games 4 --nodes 300 "
            "--seed 7 --random-moves 8 --output <dir>/sample.mnsd --mnzt-output <dir>/sample.mnzt` "
            "or set MINASE_ZERO_SAMPLE_MNZT"
        )
    return path
