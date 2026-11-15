"""MNSDの駒コードと正準形の着手を、Rustのnnと同じ入力面・方策ラベルへ写す。"""

import minase_mnsd_reader
import numpy as np
from numpy.typing import NDArray
import torch


INPUT_PLANES = 159
BOARD_SQUARES = 144
POLICY_PLANES = 249
POLICY_SIZE = 35_856
PIECE_STATE_COUNT = 47

# PieceKind::ALLとcan_promoteの順。成った後の駒種番号をそのまま扱う。
PROMOTABLE_KINDS = (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16, 17, 18, 19)
# PieceCode::new_promotedが受理する、成った後の18種。
PROMOTED_KINDS = (4, 5, 6, 7, 8, 9, 10, 12, 17, 20, 21, 22, 23, 24, 25, 26, 27, 28)


def _piece_planes() -> tuple[int, ...]:
    states = list(range(29))
    for state, kind in enumerate(PROMOTABLE_KINDS, start=29):
        states[kind] = state
    table = [-1] * 128
    for color in range(2):
        for kind in range(21):
            table[1 + kind + 64 * color] = color * PIECE_STATE_COUNT + states[kind]
        for kind in PROMOTED_KINDS:
            table[1 + kind + 29 + 64 * color] = color * PIECE_STATE_COUNT + kind
    return tuple(table)


PIECE_CODE_PLANES = _piece_planes()
_DIRECTION_STEPS = ((0, 1), (1, 1), (1, 0), (1, -1), (0, -1), (-1, -1), (-1, 0), (-1, 1))
_KNIGHT_STEPS = ((1, 2), (2, 1), (2, -1), (1, -2), (-1, -2), (-2, -1), (-2, 1), (-1, 2))
_DIRECTIONS = np.full((3, 3), -1, dtype=np.int64)
for _direction, (_dx, _dy) in enumerate(_DIRECTION_STEPS):
    _DIRECTIONS[_dx + 1, _dy + 1] = _direction
_KNIGHT_DIRECTIONS = np.full((5, 5), -1, dtype=np.int64)
for _direction, (_dx, _dy) in enumerate(_KNIGHT_STEPS):
    _KNIGHT_DIRECTIONS[_dx + 2, _dy + 2] = _direction


def policy_mask() -> NDArray[np.bool_]:
    """盤面によらず可能な16,108ラベルを真とする長さ35,856の定数マスクを返す。

    policy_labelsと同じ面・相対座標で、移動先と中継升が盤内にあることを検査する。
    成りの直線移動は、移動元か移動先の相対段が敵陣の8〜11であることも要求する。
    駒の配置や種類、反復の履歴は参照しない。
    """
    squares = np.arange(BOARD_SQUARES)
    x, y = squares % 12, squares // 12
    mask = np.empty((POLICY_PLANES, BOARD_SQUARES), dtype=np.bool_)

    def inside(x: NDArray[np.integer], y: NDArray[np.integer]) -> NDArray[np.bool_]:
        return (0 <= x) & (x < 12) & (0 <= y) & (y < 12)

    for plane in range(POLICY_PLANES):
        if plane < 176:
            direction, distance = divmod(plane % 88, 11)
            dx, dy = _DIRECTION_STEPS[direction]
            tx, ty = x + dx * (distance + 1), y + dy * (distance + 1)
            mask[plane] = inside(tx, ty)
            if plane >= 88:
                mask[plane] &= (y >= 8) | (ty >= 8)
        elif plane < 184:
            dx, dy = _KNIGHT_STEPS[plane - 176]
            mask[plane] = inside(x + dx, y + dy)
        elif plane < 240:
            first, second = divmod(plane - 184, 7)
            dx, dy = _DIRECTION_STEPS[first]
            mx, my = x + dx, y + dy
            reverse = (first + 4) % 8
            sx, sy = _DIRECTION_STEPS[second + (second >= reverse)]
            mask[plane] = inside(mx, my) & inside(mx + sx, my + sy)
        elif plane < 248:
            dx, dy = _DIRECTION_STEPS[plane - 240]
            mask[plane] = inside(x + dx, y + dy)
        else:
            mask[plane] = True
    return mask.reshape(POLICY_SIZE)


def encode_planes(boards: torch.Tensor, sides: torch.Tensor, lions: torch.Tensor) -> torch.Tensor:
    """同じ機器のuint8の(B,144)、(B)、(B)からfloat32の(B,159,12,12)を返す。

    後手番は升と陣営を反転する。先獅子なしは255で表す。
    成り権の保留印、不正な駒コード、形、型、手番、升はValueErrorとする。
    """
    if any(tensor.dtype != torch.uint8 for tensor in (boards, sides, lions)):
        raise ValueError("boards, sides and lions must have dtype uint8")
    if boards.ndim != 2 or boards.shape[1] != BOARD_SQUARES:
        raise ValueError("boards must have shape (B, 144)")
    count = boards.shape[0]
    if sides.shape != (count,) or lions.shape != (count,):
        raise ValueError("sides and lions must have shape (B,)")
    if boards.device != sides.device or boards.device != lions.device:
        raise ValueError("boards, sides and lions must be on the same device")
    if torch.any(boards & 128):
        raise ValueError(f"deferred promotion cannot be encoded in {INPUT_PLANES} planes")
    if torch.any(sides > 1):
        raise ValueError("side must be 0 or 1")
    if torch.any((lions >= BOARD_SQUARES) & (lions != 255)):
        raise ValueError("lion square must be in 0..143 or 255")

    table = torch.tensor(PIECE_CODE_PLANES, dtype=torch.int64, device=boards.device)
    planes = table[boards.to(torch.int64)]
    occupied = boards != 0
    if torch.any(occupied & (planes < 0)):
        raise ValueError("invalid MNSD piece code")
    output = torch.zeros((count, INPUT_PLANES, BOARD_SQUARES), dtype=torch.float32, device=boards.device)
    batch, square = occupied.nonzero(as_tuple=True)
    relative_square = torch.where(sides[batch] == 0, square, 143 - square)
    relative_plane = (planes[batch, square] + sides[batch].to(torch.int64) * 47) % 94
    output[batch, relative_plane, relative_square] = 1.0

    batch = (lions != 255).nonzero(as_tuple=True)[0]
    square = lions[batch].to(torch.int64)
    relative_square = torch.where(sides[batch] == 0, square, 143 - square)
    output[batch, 94, relative_square] = 1.0

    control = minase_mnsd_reader.control_planes(boards.cpu().numpy(), sides.cpu().numpy())
    output[:, 95:] = torch.from_numpy(control).to(device=boards.device)
    return output.reshape(count, INPUT_PLANES, 12, 12)


def _directions(dx: NDArray[np.int64], dy: NDArray[np.int64]) -> NDArray[np.int64]:
    if np.any((np.abs(dx) > 1) | (np.abs(dy) > 1) | ((dx == 0) & (dy == 0))):
        raise ValueError("two-step paths must consist of adjacent, nonstationary steps")
    return _DIRECTIONS[dx + 1, dy + 1]


def policy_labels(
    from_sq: NDArray[np.integer],
    mid_sq: NDArray[np.integer],
    to_sq: NDArray[np.integer],
    promote: NDArray[np.integer] | NDArray[np.bool_],
    sides: NDArray[np.integer],
) -> NDArray[np.int64]:
    """候補ごとの1次元配列から、面×144＋相対移動元のint64のラベルを返す。

    升はdense_index、経由升なしは255、成りと手番は0か1とする。
    正準形でない経路、2歩・じっと・桂馬型の成り、符号化できない移動はValueErrorとする。
    盤を参照しないため、駒の動きや経由升の相手駒の有無は検査しない。
    """
    arrays = (from_sq, mid_sq, to_sq, promote, sides)
    if any(array.ndim != 1 or array.shape != from_sq.shape for array in arrays):
        raise ValueError("policy arrays must be one-dimensional and have equal lengths")
    if any(array.dtype.kind not in "iu" for array in (from_sq, mid_sq, to_sq, sides)):
        raise ValueError("squares and sides must be integer arrays")
    if promote.dtype.kind not in "biu":
        raise ValueError("promotion must be an integer or boolean array")
    if np.any((from_sq < 0) | (from_sq >= 144) | (to_sq < 0) | (to_sq >= 144)):
        raise ValueError("from and to squares must be in 0..143")
    if np.any((mid_sq != 255) & ((mid_sq < 0) | (mid_sq >= 144))):
        raise ValueError("middle square must be in 0..143 or 255")
    if np.any((promote < 0) | (promote > 1) | (sides < 0) | (sides > 1)):
        raise ValueError("promotion and sides must be 0 or 1")

    start, middle, end, promotion, side = (array.astype(np.int64, copy=False) for array in arrays)
    sign = 1 - 2 * side
    dx = (end % 12 - start % 12) * sign
    dy = (end // 12 - start // 12) * sign
    two_step = middle != 255
    stationary = ~two_step & (start == end)
    straight = ~two_step & ~stationary & ((dx == 0) | (dy == 0) | (np.abs(dx) == np.abs(dy)))
    knight = ~(two_step | stationary | straight)
    if np.any((two_step | stationary | knight) & (promotion != 0)):
        raise ValueError("two-step, stationary and knight-shaped moves cannot promote")
    plane = np.empty(start.shape, dtype=np.int64)
    plane[stationary] = 248
    distance = np.maximum(np.abs(dx[straight]), np.abs(dy[straight]))
    plane[straight] = (
        _directions(np.sign(dx[straight]), np.sign(dy[straight])) * 11
        + distance - 1 + promotion[straight] * 88
    )

    kx, ky = dx[knight], dy[knight]
    if np.any(~(((np.abs(kx) == 1) & (np.abs(ky) == 2)) | ((np.abs(kx) == 2) & (np.abs(ky) == 1)))):
        raise ValueError("move cannot be encoded as a ray or knight-shaped jump")
    plane[knight] = 176 + _KNIGHT_DIRECTIONS[kx + 2, ky + 2]

    first = _directions(
        (middle[two_step] % 12 - start[two_step] % 12) * sign[two_step],
        (middle[two_step] // 12 - start[two_step] // 12) * sign[two_step],
    )
    second = _directions(
        (end[two_step] % 12 - middle[two_step] % 12) * sign[two_step],
        (end[two_step] // 12 - middle[two_step] // 12) * sign[two_step],
    )
    returning = start[two_step] == end[two_step]
    reverse = (first + 4) % 8
    if np.any(~returning & (second == reverse)):
        raise ValueError("a reversed second step must return to the starting square")
    plane[two_step] = np.where(
        returning, 240 + first, 184 + first * 7 + second - (second > reverse)
    )
    return plane * BOARD_SQUARES + np.where(side == 0, start, 143 - start)
