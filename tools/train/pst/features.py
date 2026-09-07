"""学習PSTの駒状態、視点変換および左右鏡映を実装する。"""

from __future__ import annotations

import numpy as np
from numpy.typing import NDArray


PIECE_STATE_COUNT = 47
BOARD_SQUARE_COUNT = 144
BOARD_FEATURE_COUNT = 2 * PIECE_STATE_COUNT * BOARD_SQUARE_COUNT
FEATURE_COUNT = BOARD_FEATURE_COUNT + BOARD_SQUARE_COUNT
PADDING_INDEX = FEATURE_COUNT
NO_LION_SQUARE = 255

PROMOTABLE_KINDS = np.array(
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16, 17, 18, 19],
    dtype=np.int16,
)


def _build_piece_tables() -> tuple[NDArray[np.int16], NDArray[np.int8]]:
    """盤面バイトから駒状態と陣営を得る256要素の表を作る。"""
    states = np.full(256, -1, dtype=np.int16)
    colors = np.full(256, -1, dtype=np.int8)
    promotion_rank = np.full(29, -1, dtype=np.int16)
    promotion_rank[PROMOTABLE_KINDS] = np.arange(18, dtype=np.int16)
    for color, base in enumerate((1, 65)):
        for encoded in range(base, base + 58):
            payload = encoded - base
            promoted = payload >= 29
            kind = payload % 29
            state = kind
            if not promoted and promotion_rank[kind] >= 0:
                state = 29 + int(promotion_rank[kind])
            states[encoded] = state
            colors[encoded] = color
    return states, colors


def _build_square_maps() -> NDArray[np.int16]:
    """先手・後手の視点別に144升の写像表を作る。"""
    dense = np.arange(BOARD_SQUARE_COUNT, dtype=np.int16)
    file = dense % 12
    rank = dense // 12
    return np.stack((dense, (11 - rank) * 12 + file))


PIECE_STATE_BY_BYTE, COLOR_BY_BYTE = _build_piece_tables()
SQUARE_BY_PERSPECTIVE = _build_square_maps()

INITIAL_BOARD = np.array(
    [
        3, 14, 16, 17, 18, 12, 13, 18, 17, 16, 14, 3,
        4, 0, 7, 0, 15, 19, 20, 15, 0, 7, 0, 4,
        5, 6, 8, 9, 10, 21, 11, 10, 9, 8, 6, 5,
        1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
        0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 66, 0, 0, 0, 0, 66, 0, 0, 0,
        65, 65, 65, 65, 65, 65, 65, 65, 65, 65, 65, 65,
        69, 70, 72, 73, 74, 75, 85, 74, 73, 72, 70, 69,
        68, 0, 71, 0, 79, 84, 83, 79, 0, 71, 0, 68,
        67, 78, 80, 81, 82, 77, 76, 82, 81, 80, 78, 67,
    ],
    dtype=np.uint8,
)


def feature_indices(
    board: NDArray[np.uint8],
    stm: NDArray[np.uint8],
    lion: NDArray[np.uint8],
) -> NDArray[np.int32]:
    """局面バッチを手番側視点の特徴番号行列へ変換する。"""
    board = np.asarray(board, dtype=np.uint8)
    stm = np.asarray(stm, dtype=np.uint8)
    lion = np.asarray(lion, dtype=np.uint8)
    if board.ndim != 2 or board.shape[1] != BOARD_SQUARE_COUNT:
        raise ValueError("board must have shape (B, 144)")
    if stm.shape != (board.shape[0],) or lion.shape != (board.shape[0],):
        raise ValueError("stm and lion must have shape (B,)")
    if np.any(stm > 1):
        raise ValueError("stm contains a value outside 0..1")

    batch_size = board.shape[0]
    states = PIECE_STATE_BY_BYTE[board]
    colors = COLOR_BY_BYTE[board]
    occupied = board != 0
    if np.any(occupied & ((states < 0) | (colors < 0))):
        raise ValueError("board contains an invalid piece byte")
    relative_color = colors != stm[:, None]
    squares = SQUARE_BY_PERSPECTIVE[
        stm[:, None], np.arange(BOARD_SQUARE_COUNT, dtype=np.int16)[None, :]
    ]
    indices = (
        (relative_color.astype(np.int32) * PIECE_STATE_COUNT + states.astype(np.int32))
        * BOARD_SQUARE_COUNT
        + squares.astype(np.int32)
    )
    indices[~occupied] = PADDING_INDEX

    result = np.full((batch_size, BOARD_SQUARE_COUNT + 1), PADDING_INDEX, dtype=np.int32)
    result[:, :BOARD_SQUARE_COUNT] = indices
    has_lion = lion != NO_LION_SQUARE
    if np.any(has_lion):
        if np.any(lion[has_lion] >= BOARD_SQUARE_COUNT):
            raise ValueError("lion contains a value outside 0..143 or 255")
        rows = np.nonzero(has_lion)[0]
        result[rows, BOARD_SQUARE_COUNT] = (
            BOARD_FEATURE_COUNT
            + SQUARE_BY_PERSPECTIVE[stm[rows], lion[rows]].astype(np.int32)
        )
    return result


def mirror(
    board: NDArray[np.uint8], lion: NDArray[np.uint8]
) -> tuple[NDArray[np.uint8], NDArray[np.uint8]]:
    """盤面と先獅子対象升を筋方向へ左右鏡映する。"""
    board = np.asarray(board, dtype=np.uint8)
    lion = np.asarray(lion, dtype=np.uint8)
    if board.ndim != 2 or board.shape[1] != BOARD_SQUARE_COUNT:
        raise ValueError("board must have shape (B, 144)")
    if lion.shape != (board.shape[0],):
        raise ValueError("lion must have shape (B,)")
    mirrored_board = board.reshape(-1, 12, 12)[:, :, ::-1].reshape(-1, 144).copy()
    mirrored_lion = lion.copy()
    has_lion = lion != NO_LION_SQUARE
    if np.any(lion[has_lion] >= BOARD_SQUARE_COUNT):
        raise ValueError("lion contains a value outside 0..143 or 255")
    mirrored_lion[has_lion] = (
        (lion[has_lion] // 12) * 12 + (11 - lion[has_lion] % 12)
    )
    return mirrored_board, mirrored_lion
