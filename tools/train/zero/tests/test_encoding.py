"""入力平面と方策ラベルが、Rust側の符号化と一致することを検査する。"""

from __future__ import annotations

import json
from pathlib import Path

import numpy as np
import pytest

from minase_zero.encoding import (
    BLACK,
    INPUT_PLANES,
    LION_CAPTURE_PLANE,
    PIECE_STATE_COUNT,
    POLICY_PLANES,
    SQUARE_COUNT,
    WHITE,
    encode,
    move_label,
    parse_sfen,
    parse_square,
)

FIXTURE = Path(__file__).resolve().parents[4] / "crates/minase/tests/fixtures/nn_encoding.json"
"""Rustが書き出す固定局面の参照値（crates/minase/src/nn/fixture.rs）。"""

INITIAL_SFEN = (
    "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/"
    "3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL b"
)


def sfen(pieces: dict[str, str], side: str, lion_capture: str = "-") -> str:
    """USI表記の升から駒文字への対応で、4欄の拡張SFENを作る。"""
    board = [[""] * 12 for _ in range(12)]
    for square, letter in pieces.items():
        index = parse_square(square)
        rank, file = divmod(index, 12)
        board[11 - rank][file] = letter
    rows = []
    for row in board:
        text, empty = "", 0
        for letter in row:
            if letter:
                text += (str(empty) if empty else "") + letter
                empty = 0
            else:
                empty += 1
        rows.append(text + (str(empty) if empty else ""))
    return f"{'/'.join(rows)} {side} {lion_capture} 1"


def nonzero(planes: np.ndarray) -> list[tuple[int, int]]:
    """値が1の要素の(面, 相対升)を返す。"""
    flat = planes.reshape(INPUT_PLANES, SQUARE_COUNT)
    return sorted((int(p), int(s)) for p, s in zip(*np.nonzero(flat)))


def test_fixture_matches_rust_encoding():
    """Rustが書き出した固定局面集の入力平面と全合法手のラベルを再現する。

    参照値の形式は`{"positions": [{"sfen": ..., "rules": ..., "planes": [[面, 相対升], ...],
    "moves": [[USI表記, ラベル], ...]}, ...]}`である。
    """
    assert FIXTURE.exists(), (
        f"{FIXTURE} does not exist; generate it with the ignored Rust test in crates/minase/src/nn/fixture.rs"
    )
    entries = json.loads(FIXTURE.read_text())["positions"]
    assert entries, "fixture has no positions"
    for entry in entries:
        position = parse_sfen(entry["sfen"])
        expected = sorted((plane, square) for plane, square in entry["planes"])
        assert nonzero(encode(position)) == expected, entry["sfen"]
        assert entry["moves"], entry["sfen"]
        for usi, label in entry["moves"]:
            assert move_label(position, usi) == label, (entry["sfen"], usi)


def test_initial_position_marks_pawns_on_expected_planes():
    """初期局面の先手の歩兵は手番側の面29の段3、後手の歩兵は相手側の面29の段8に立つ。"""
    planes = encode(parse_sfen(INITIAL_SFEN)).reshape(INPUT_PLANES, SQUARE_COUNT)
    assert planes.dtype == np.float32
    assert len(nonzero(planes)) == 92
    for file in range(12):
        assert planes[29, 3 * 12 + file] == 1.0
        assert planes[PIECE_STATE_COUNT + 29, 8 * 12 + file] == 1.0


def test_white_to_move_rotates_board_and_swaps_sides():
    """後手番では盤を180度回転させ、後手の駒を手番側の面へ置く。"""
    position = parse_sfen(INITIAL_SFEN.replace(" b", " w"))
    assert position.side_to_move == WHITE
    planes = encode(position).reshape(INPUT_PLANES, SQUARE_COUNT)
    # 後手の歩兵（段8）は回転後に段3、先手の歩兵（段3）は回転後に段8へ写る。
    for file in range(12):
        assert planes[29, 3 * 12 + file] == 1.0
        assert planes[PIECE_STATE_COUNT + 29, 8 * 12 + file] == 1.0
    # 後手の玉将は内部座標(筋6, 段11)にあり、回転後は(筋5, 段0)。
    assert planes[11, 0 * 12 + 5] == 1.0


def test_promoted_pieces_and_lion_capture_square():
    """成駒は成った後の駒種の状態、先獅子の升は面94に立つ。"""
    position = parse_sfen(
        sfen({"7g": "+C", "6g": "+o", "5g": "N", "1a": "k", "12l": "K"}, "w", lion_capture="7f")
    )
    planes = encode(position).reshape(INPUT_PLANES, SQUARE_COUNT)
    rotated = SQUARE_COUNT - 1
    # 成った銅将は横行の状態4、成った麒麟は獅子の状態20で、獅子と同じ面を使う。
    assert planes[PIECE_STATE_COUNT + 4, rotated - parse_square("7g")] == 1.0
    assert planes[20, rotated - parse_square("6g")] == 1.0
    assert planes[PIECE_STATE_COUNT + 20, rotated - parse_square("5g")] == 1.0
    assert planes[LION_CAPTURE_PLANE, rotated - parse_square("7f")] == 1.0


def test_deferred_promotion_is_rejected():
    """成り権保留の欄を持つ局面は符号化できない。"""
    with pytest.raises(ValueError):
        parse_sfen(INITIAL_SFEN + " - 1 7i")


def test_pawn_advances_have_hand_computed_labels():
    """先手と後手の歩兵の前進は方向0、距離1の面0になる（nn/policy.rs の手計算と同じ値）。"""
    position = parse_sfen(INITIAL_SFEN)
    assert move_label(position, "7i7h") == (3 * 12 + 5) * POLICY_PLANES
    white = parse_sfen(INITIAL_SFEN.replace(" b", " w"))
    assert move_label(white, "7d7e") == (3 * 12 + 6) * POLICY_PLANES


def test_lion_moves_have_hand_computed_labels():
    """獅子のじっと、居喰い、2段階移動、空升経由の跳び、桂馬型の跳び、成りの面を確かめる。"""
    origin = (5 * 12 + 5) * POLICY_PLANES
    kings = {"1a": "k", "12l": "K"}
    empty = parse_sfen(sfen({"7g": "N", **kings}, "b"))
    # じっと：経由升が空いた3升表記。
    assert move_label(empty, "7g7f7g") == origin + 248
    # 空いた経由升を通る2歩は直接の跳びと同じ直進の面：方向0、距離2。
    assert move_label(empty, "7g7f7e") == origin + 1
    # 前へ2升、右へ1升の桂馬型の跳び。
    assert move_label(empty, "7g6e") == origin + 176
    # 左後ろへ3升進んで成る：方向5、距離3。
    assert move_label(empty, "7g10j+") == origin + 88 + 5 * 11 + 2
    captures = parse_sfen(sfen({"7g": "N", "7f": "g", "6g": "g", **kings}, "b"))
    # 前の駒を取って戻る居喰い：方向0。
    assert move_label(captures, "7g7f7g") == origin + 240
    # 右の駒を取って前へ進む2段階移動：第1段は方向2、第2段は方向0。
    assert move_label(captures, "7g6g6f") == origin + 184 + 2 * 7
    # 前の駒を取って左後ろへ進む2段階移動：逆方向4を除くので順位は4。
    assert move_label(captures, "7g7f8g") == origin + 184 + 4


def test_invalid_input_raises_value_error():
    """誤った入力は例外ValueErrorで拒否し、他の例外で落ちない。"""
    position = parse_sfen(INITIAL_SFEN)
    for text in ("", "7i", "7i7i", "13a1a", "7i7h+7g", "7i0h", "7i7hx"):
        with pytest.raises(ValueError):
            move_label(position, text)
    for text in ("", "12/12 b", INITIAL_SFEN.replace(" b", " x"), INITIAL_SFEN.replace("L b", "Z b")):
        with pytest.raises(ValueError):
            parse_sfen(text)
    assert parse_sfen(INITIAL_SFEN).side_to_move == BLACK
