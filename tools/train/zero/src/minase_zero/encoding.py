"""局面からネットの入力平面への符号化と、着手の方策ラベル。

Rust側の`crates/minase/src/nn/encoding.rs`と`nn/policy.rs`を同じ規約で再実装する。
両者の一致は、Rustが書き出した固定局面の参照値`crates/minase/tests/fixtures/nn_encoding.json`を
`tests/test_encoding.py`が照合して確かめる。

升は内部座標の(筋, 段)で表し、筋0は先手から見て左端（USI表記の筋12）、段0は先手側の最下段
（USI表記の段l）である。入力と方策は手番側から見た相対座標を使い、先手番では内部座標のまま、
後手番では盤を180度回転させた(11 − 筋, 11 − 段)を使う。升の番号は`段 * 12 + 筋`である。
"""

from __future__ import annotations

from dataclasses import dataclass
from itertools import pairwise

import numpy as np

BOARD_SIZE = 12
"""盤の筋と段の数。"""
SQUARE_COUNT = BOARD_SIZE * BOARD_SIZE
"""盤の升の数。"""
PIECE_STATE_COUNT = 47
"""駒種と現在の成り可否を区別した駒状態の数。"""
INPUT_PLANES = 2 * PIECE_STATE_COUNT + 2
"""入力平面の数。手番側の47状態、相手側の47状態、先獅子の升の1面、および成り権保留の升の1面。"""
LION_CAPTURE_PLANE = 2 * PIECE_STATE_COUNT
"""先獅子の升を示す面の番号。"""
PROMOTION_DEFERRED_PLANE = LION_CAPTURE_PLANE + 1
"""成り権を保留している駒（規則P1、P2、P5）の升を示す面の番号。"""
POLICY_PLANES = 249
"""方策の移動元の升ごとの面の数。"""
POLICY_SIZE = SQUARE_COUNT * POLICY_PLANES
"""方策の出力の要素数。"""

BLACK = 0
"""先手を表す手番の値。"""
WHITE = 1
"""後手を表す手番の値。"""

# 駒種番号は crates/minase-core/src/piece/kind.rs の PieceKind の順に合わせる。
_KIND_NAMES = (
    "Pawn", "GoBetween", "Lance", "ReverseChariot", "SideMover", "VerticalMover",
    "Bishop", "Rook", "DragonHorse", "DragonKing", "FreeKing", "King",
    "DrunkElephant", "FerociousLeopard", "BlindTiger", "CopperGeneral",
    "SilverGeneral", "GoldGeneral", "Kirin", "Phoenix", "Lion", "CrownPrince",
    "WhiteHorse", "Whale", "FlyingOx", "FreeBoar", "FlyingStag", "HornedFalcon",
    "SoaringEagle",
)
_KIND = {name: index for index, name in enumerate(_KIND_NAMES)}

# 成れる駒種から成った後の駒種への対応（PieceKind::promoted）。
_PROMOTED = {
    "Pawn": "GoldGeneral",
    "GoBetween": "DrunkElephant",
    "Lance": "WhiteHorse",
    "ReverseChariot": "Whale",
    "SideMover": "FreeBoar",
    "VerticalMover": "FlyingOx",
    "Bishop": "DragonHorse",
    "Rook": "DragonKing",
    "DragonHorse": "HornedFalcon",
    "DragonKing": "SoaringEagle",
    "DrunkElephant": "CrownPrince",
    "FerociousLeopard": "Bishop",
    "BlindTiger": "FlyingStag",
    "CopperGeneral": "SideMover",
    "SilverGeneral": "VerticalMover",
    "GoldGeneral": "Rook",
    "Kirin": "Lion",
    "Phoenix": "FreeKing",
}

# SFENの駒文字（大文字）から成る前の駒種への対応（notation/sfen.rs）。
_SFEN_LETTERS = {
    "K": "King",
    "P": "Pawn",
    "L": "Lance",
    "S": "SilverGeneral",
    "G": "GoldGeneral",
    "B": "Bishop",
    "R": "Rook",
    "F": "FerociousLeopard",
    "C": "CopperGeneral",
    "E": "DrunkElephant",
    "A": "ReverseChariot",
    "T": "BlindTiger",
    "O": "Kirin",
    "X": "Phoenix",
    "M": "SideMover",
    "V": "VerticalMover",
    "H": "DragonHorse",
    "D": "DragonKing",
    "N": "Lion",
    "Q": "FreeKing",
    "I": "GoBetween",
}


def _unpromoted_states() -> dict[str, int]:
    """成っていない駒の状態番号を作る。成れる駒種には駒種番号順に29から番号を振る。"""
    states = {}
    next_state = 29
    for name in _KIND_NAMES:
        if name in _PROMOTED:
            states[name] = next_state
            next_state += 1
        else:
            states[name] = _KIND[name]
    return states


_UNPROMOTED_STATES = _unpromoted_states()

# 8方向の(段差, 筋差)。nn/policy.rs の DIRECTIONS と同じく、前(+1, 0)から時計回りに番号を振る。
DIRECTIONS = ((1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1))
"""方策の8方向の(段差, 筋差)。手番側の前方は相対座標で段が増える向きである。"""
# 桂馬型の跳びの(段差, 筋差)。nn/policy.rs の KNIGHT_JUMPS と同じ順。
KNIGHT_JUMPS = ((2, 1), (1, 2), (-1, 2), (-2, 1), (-2, -1), (-1, -2), (1, -2), (2, -1))
"""獅子の桂馬型の跳びの(段差, 筋差)。"""

_PROMOTION_OFFSET = 88
_KNIGHT_OFFSET = 176
_TWO_STEP_OFFSET = 184
_IGUI_OFFSET = 240
_JITTO_PLANE = 248


@dataclass(frozen=True)
class Piece:
    """盤上の駒。`kind`は現在の駒種、`promoted`は成駒かどうかを表す。"""

    color: int
    kind: str
    promoted: bool

    def state(self) -> int:
        """駒種と現在の成り可否からなる47状態の番号を返す（eval/pst/features.rs の piece_state）。"""
        if self.promoted:
            return _KIND[self.kind]
        return _UNPROMOTED_STATES[self.kind]


@dataclass(frozen=True)
class Position:
    """符号化に必要な局面の情報。

    `board`は内部座標の升番号`段 * 12 + 筋`ごとの駒、`lion_capture`は先獅子の升、
    `promotion_deferred`は成り権を保留している駒の升番号を昇順に並べた組である。
    """

    board: tuple[Piece | None, ...]
    side_to_move: int
    lion_capture: int | None
    promotion_deferred: tuple[int, ...]


def square_index(file: int, rank: int) -> int:
    """内部座標の(筋, 段)を升番号`段 * 12 + 筋`へ変換する。"""
    return rank * BOARD_SIZE + file


def relative_square(square: int, side_to_move: int) -> int:
    """升番号を手番側から見た相対座標の升番号へ写す。後手番では盤を180度回転させる。"""
    if side_to_move == BLACK:
        return square
    return SQUARE_COUNT - 1 - square


def parse_square(text: str) -> int:
    """USI表記の升（例 `7i`、`12a`）を内部座標の升番号へ変換する。

    Raises:
        ValueError: 盤上の升を表さない場合。
    """
    if not (2 <= len(text) <= 3) or not text.isascii():
        raise ValueError(f"invalid square {text!r}")
    file_text, rank_text = text[:-1], text[-1]
    if not file_text.isdigit() or file_text.startswith("0"):
        raise ValueError(f"invalid square {text!r}")
    file_name = int(file_text)
    rank_name = ord(rank_text) - ord("a")
    if not (1 <= file_name <= BOARD_SIZE) or not (0 <= rank_name < BOARD_SIZE):
        raise ValueError(f"invalid square {text!r}")
    return square_index(BOARD_SIZE - file_name, BOARD_SIZE - 1 - rank_name)


def parse_sfen(sfen: str) -> Position:
    """2欄の基本形、または4欄か5欄の拡張SFENを読む。

    拡張SFENの第3欄は先獅子の升（`-`はなし）、第4欄は手数、第5欄は成り権を保留している駒
    （規則P1、P2、P5）の升をコンマで区切った一覧（`-`はなし）である。
    第5欄の升は、Rust側の`parse_extended_sfen`と同じく升番号の狭義昇順に並び、
    成っていない成れる駒がなければならない。

    Raises:
        ValueError: 構文が誤っている場合。
    """
    fields = sfen.split()
    if len(fields) not in (2, 4, 5):
        raise ValueError(f"SFEN must have 2, 4, or 5 fields: {sfen!r}")
    if fields[1] not in ("b", "w"):
        raise ValueError(f"invalid side to move {fields[1]!r}")
    side_to_move = BLACK if fields[1] == "b" else WHITE
    lion_capture = None
    if len(fields) >= 4:
        if fields[2] != "-":
            lion_capture = parse_square(fields[2])
        if not fields[3].isdigit():
            raise ValueError(f"invalid move number {fields[3]!r}")
    board = _parse_board(fields[0])
    promotion_deferred = ()
    if len(fields) == 5 and fields[4] != "-":
        promotion_deferred = _parse_promotion_deferred(fields[4], board)
    return Position(board, side_to_move, lion_capture, promotion_deferred)


def _parse_promotion_deferred(field: str, board: tuple[Piece | None, ...]) -> tuple[int, ...]:
    """拡張SFENの第5欄（成り権保留の升の一覧）を読む。

    Raises:
        ValueError: 升が誤っている場合、狭義昇順でない場合、または升に成っていない成れる駒がない場合。
    """
    squares = tuple(parse_square(text) for text in field.split(","))
    if any(previous >= current for previous, current in pairwise(squares)):
        raise ValueError(f"promotion-deferred squares are not strictly ascending: {field!r}")
    for square in squares:
        piece = board[square]
        if piece is None or piece.promoted or piece.kind not in _PROMOTED:
            raise ValueError(f"promotion-deferred square {square} has no unpromoted promotable piece")
    return squares


def _parse_board(board: str) -> tuple[Piece | None, ...]:
    """SFENの盤面欄を読む。段は内部rank 11から0へ、筋は内部file 0から11へ進む。"""
    rows = board.split("/")
    if len(rows) != BOARD_SIZE:
        raise ValueError(f"SFEN board has {len(rows)} rows; expected {BOARD_SIZE}")
    squares: list[Piece | None] = [None] * SQUARE_COUNT
    for row_index, row in enumerate(rows):
        rank = BOARD_SIZE - 1 - row_index
        file = 0
        index = 0
        while index < len(row):
            character = row[index]
            if character.isdigit():
                end = index
                while end < len(row) and row[end].isdigit():
                    end += 1
                empty = int(row[index:end])
                if empty == 0:
                    raise ValueError(f"SFEN row {row_index + 1} has an empty count of 0")
                file += empty
                index = end
                if file > BOARD_SIZE:
                    raise ValueError(f"SFEN row {row_index + 1} is too wide")
                continue
            promoted = character == "+"
            if promoted:
                index += 1
                if index == len(row):
                    raise ValueError(f"SFEN row {row_index + 1} ends with '+'")
                character = row[index]
            base = _SFEN_LETTERS.get(character.upper())
            if base is None:
                raise ValueError(f"unsupported SFEN piece letter {character!r}")
            if promoted and base not in _PROMOTED:
                raise ValueError(f"SFEN piece {character!r} has no promoted form")
            if file >= BOARD_SIZE:
                raise ValueError(f"SFEN row {row_index + 1} is too wide")
            color = BLACK if character.isupper() else WHITE
            kind = _PROMOTED[base] if promoted else base
            squares[square_index(file, rank)] = Piece(color, kind, promoted)
            file += 1
            index += 1
        if file != BOARD_SIZE:
            raise ValueError(f"SFEN row {row_index + 1} has width {file}; expected {BOARD_SIZE}")
    return tuple(squares)


def encode(position: Position) -> np.ndarray:
    """局面を手番側から見た入力平面`[96, 12, 12]`（float32、値は0か1）へ符号化する。

    面0〜46は手番側の駒、面47〜93は相手側の駒、面94は先獅子の升、面95は成り権を保留している
    駒の升を表す。面95は手番側と相手側の駒を1面にまとめる。
    """
    planes = np.zeros((INPUT_PLANES, SQUARE_COUNT), dtype=np.float32)
    side = position.side_to_move
    for square, piece in enumerate(position.board):
        if piece is None:
            continue
        owner = 0 if piece.color == side else 1
        planes[owner * PIECE_STATE_COUNT + piece.state(), relative_square(square, side)] = 1.0
    if position.lion_capture is not None:
        planes[LION_CAPTURE_PLANE, relative_square(position.lion_capture, side)] = 1.0
    for square in position.promotion_deferred:
        planes[PROMOTION_DEFERRED_PLANE, relative_square(square, side)] = 1.0
    return planes.reshape(INPUT_PLANES, BOARD_SIZE, BOARD_SIZE)


def parse_usi_move(position: Position, text: str) -> tuple[int, int | None, int, bool]:
    """lishogi系拡張USIの着手を(移動元, 経由升, 移動先, 成り)へ変換する。

    notation/usi.rs の parse と同じく、経由升は相手駒がある場合だけ残し、
    空いた経由升を通る3升表記は直接の移動（移動先が移動元ならじっと）へ正規化する。

    Raises:
        ValueError: 表記が誤っている場合。
    """
    if not text.isascii() or not text:
        raise ValueError(f"invalid USI move {text!r}")
    promote = False
    body = text
    if text[-1] in "+=?":
        promote = text[-1] == "+"
        body = text[:-1]
    if any(character in "+=?" for character in body):
        raise ValueError(f"invalid USI move suffix {text!r}")
    squares = []
    index = 0
    while index < len(body):
        end = index
        while end < len(body) and body[end].isdigit():
            end += 1
        squares.append(parse_square(body[index:end + 1]))
        index = end + 1
    if len(squares) not in (2, 3):
        raise ValueError(f"USI move must have 2 or 3 squares: {text!r}")
    origin = squares[0]
    mid = squares[1] if len(squares) == 3 else None
    destination = squares[-1]
    if mid is None and destination == origin:
        raise ValueError(f"USI jitto must contain an intermediate square: {text!r}")
    if mid is not None:
        piece = position.board[mid]
        if piece is None or piece.color == position.side_to_move:
            mid = None
    return origin, mid, destination, promote


def move_label(position: Position, text: str) -> int:
    """USI表記の着手の方策ラベル`相対移動元 * 249 + 面`を返す（nn/policy.rs の label）。

    Raises:
        ValueError: 表記が誤っている場合、または249面のどれにも当たらない着手の場合。
    """
    origin, mid, destination, promote = parse_usi_move(position, text)
    side = position.side_to_move
    source = relative_square(origin, side)
    target = relative_square(destination, side)
    plane = _plane(source, None if mid is None else relative_square(mid, side), target, promote)
    if plane is None:
        raise ValueError(f"move {text!r} has no policy label")
    return source * POLICY_PLANES + plane


def _plane(source: int, mid: int | None, target: int, promote: bool) -> int | None:
    """相対座標の着手から249面の番号を返す。割当がなければ`None`を返す。"""
    if mid is None:
        if target == source:
            return None if promote else _JITTO_PLANE
        delta = _delta(source, target)
        line = _straight(*delta)
        if line is not None:
            direction, distance = line
            return (_PROMOTION_OFFSET if promote else 0) + direction * 11 + distance - 1
        if promote or delta not in KNIGHT_JUMPS:
            return None
        return _KNIGHT_OFFSET + KNIGHT_JUMPS.index(delta)
    if promote:
        return None
    first = _step(source, mid)
    if first is None:
        return None
    if target == source:
        return _IGUI_OFFSET + first
    second = _step(mid, target)
    if second is None:
        return None
    reverse = (first + 4) % 8
    order = second if second < reverse else second - 1
    return _TWO_STEP_OFFSET + first * 7 + order


def _delta(source: int, target: int) -> tuple[int, int]:
    """2升の(段差, 筋差)を返す。"""
    return (
        target // BOARD_SIZE - source // BOARD_SIZE,
        target % BOARD_SIZE - source % BOARD_SIZE,
    )


def _straight(rank_delta: int, file_delta: int) -> tuple[int, int] | None:
    """直線上の差分なら(方向番号, 距離)を返す。"""
    distance = max(abs(rank_delta), abs(file_delta))
    is_line = rank_delta == 0 or file_delta == 0 or abs(rank_delta) == abs(file_delta)
    if distance == 0 or not is_line:
        return None
    return DIRECTIONS.index((rank_delta // distance, file_delta // distance)), distance


def _step(source: int, target: int) -> int | None:
    """隣接する2升なら、`source`から`target`への方向番号を返す。"""
    line = _straight(*_delta(source, target))
    if line is None or line[1] != 1:
        return None
    return line[0]
