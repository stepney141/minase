"""量子化済みNNUEネットを生成、検証および整数推論する。"""

from __future__ import annotations

import argparse
import hashlib
import math
from pathlib import Path
import struct
from typing import NamedTuple, Sequence

import numpy as np
from numpy.typing import NDArray

from features import FEATURE_COUNT, INITIAL_BOARD, NO_LION_SQUARE, feature_indices


HEADER_LENGTH = 96
FORMAT_VERSION = 2
FEATURE_SET = 1
HIDDEN1_WIDTH = 256
HIDDEN2_WIDTH = 32
HIDDEN3_WIDTH = 32
INPUT2_WIDTH = HIDDEN1_WIDTH * 2
RULE_SET = b"L0,P0,R1,E0"
W1_LENGTH = FEATURE_COUNT * HIDDEN1_WIDTH * 2
B1_LENGTH = HIDDEN1_WIDTH * 4
W2_LENGTH = HIDDEN2_WIDTH * INPUT2_WIDTH * 2
B2_LENGTH = HIDDEN2_WIDTH * 4
W3_LENGTH = HIDDEN3_WIDTH * HIDDEN2_WIDTH * 2
B3_LENGTH = HIDDEN3_WIDTH * 4
WO_LENGTH = HIDDEN3_WIDTH * 2
BO_LENGTH = 4
BODY_LENGTH = (
    W1_LENGTH
    + B1_LENGTH
    + W2_LENGTH
    + B2_LENGTH
    + W3_LENGTH
    + B3_LENGTH
    + WO_LENGTH
    + BO_LENGTH
)


class Parameters(NamedTuple):
    """量子化済みNNUEの全パラメータを保持する。"""

    w1: NDArray[np.int16]
    b1: NDArray[np.int32]
    w2: NDArray[np.int16]
    b2: NDArray[np.int32]
    w3: NDArray[np.int16]
    b3: NDArray[np.int32]
    wo: NDArray[np.int16]
    bo: NDArray[np.int32]


_PARAMETER_SPECS = (
    ("W1", np.dtype(np.int16), (FEATURE_COUNT, HIDDEN1_WIDTH)),
    ("b1", np.dtype(np.int32), (HIDDEN1_WIDTH,)),
    ("W2", np.dtype(np.int16), (HIDDEN2_WIDTH, INPUT2_WIDTH)),
    ("b2", np.dtype(np.int32), (HIDDEN2_WIDTH,)),
    ("W3", np.dtype(np.int16), (HIDDEN3_WIDTH, HIDDEN2_WIDTH)),
    ("b3", np.dtype(np.int32), (HIDDEN3_WIDTH,)),
    ("Wo", np.dtype(np.int16), (HIDDEN3_WIDTH,)),
    ("bo", np.dtype(np.int32), (1,)),
)


def _validate_parameters(params: Parameters) -> None:
    """全パラメータの型と形状がファイル契約に一致することを検査する。"""
    if not isinstance(params, Parameters):
        raise TypeError("params must be Parameters")
    for value, (name, dtype, shape) in zip(params, _PARAMETER_SPECS, strict=True):
        if not isinstance(value, np.ndarray):
            raise TypeError(f"{name} must be a numpy array")
        if value.dtype != dtype:
            raise TypeError(f"{name} must have dtype {dtype}")
        if value.shape != shape:
            raise ValueError(f"{name} must have shape {shape}")


def _body_bytes(params: Parameters) -> bytes:
    """検証済みパラメータをMNUE本体の順序でバイト列にする。"""
    _validate_parameters(params)
    chunks = []
    for value in params:
        chunks.append(value.astype(value.dtype.newbyteorder("<"), copy=False).tobytes())
    body = b"".join(chunks)
    if len(body) != BODY_LENGTH:
        raise AssertionError("MNUE body length is inconsistent")
    return body


def write_mnue(path: str | Path, params: Parameters, k: float) -> None:
    """量子化済みパラメータを検査和付きMNUEファイルへ書く。"""
    if not math.isfinite(k) or k <= 0.0:
        raise ValueError("K must be finite and positive")
    body = _body_bytes(params)
    header = (
        b"MNUE"
        + struct.pack(
            "<IIIIII f",
            FORMAT_VERSION,
            FEATURE_SET,
            FEATURE_COUNT,
            HIDDEN1_WIDTH,
            HIDDEN2_WIDTH,
            HIDDEN3_WIDTH,
            k,
        )
        + RULE_SET.ljust(32, b"\0")
        + hashlib.sha256(body).digest()
    )
    if len(header) != HEADER_LENGTH:
        raise AssertionError("MNUE header length is inconsistent")
    Path(path).write_bytes(header + body)


def _read_array(
    body: bytes, offset: int, dtype: np.dtype, shape: tuple[int, ...]
) -> tuple[NDArray[np.generic], int]:
    """本体の指定位置から配列を1個読み、次の位置とともに返す。"""
    count = math.prod(shape)
    little_endian_dtype = dtype.newbyteorder("<")
    length = count * little_endian_dtype.itemsize
    value = np.frombuffer(
        body, dtype=little_endian_dtype, count=count, offset=offset
    ).reshape(shape)
    return value.astype(dtype, copy=True), offset + length


def read_mnue(path: str | Path) -> tuple[Parameters, float]:
    """MNUEファイルの全欄を検証し、パラメータとKを返す。"""
    source = Path(path)
    raw = source.read_bytes()
    expected_length = HEADER_LENGTH + BODY_LENGTH
    if len(raw) != expected_length:
        raise ValueError(f"{source}: length must be {expected_length}, got {len(raw)}")
    magic, version, feature_set, input_width, hidden1, hidden2, hidden3, k = (
        struct.unpack_from("<4sIIIIII f", raw)
    )
    if magic != b"MNUE":
        raise ValueError(f"{source}: invalid MNUE magic {magic!r}")
    if version != FORMAT_VERSION:
        raise ValueError(f"{source}: unsupported MNUE version {version}")
    if feature_set != FEATURE_SET:
        raise ValueError(f"{source}: feature set must be {FEATURE_SET}")
    widths = (input_width, hidden1, hidden2, hidden3)
    expected_widths = (FEATURE_COUNT, HIDDEN1_WIDTH, HIDDEN2_WIDTH, HIDDEN3_WIDTH)
    if widths != expected_widths:
        raise ValueError(f"{source}: layer widths must be {expected_widths}")
    if not math.isfinite(k) or k <= 0.0:
        raise ValueError(f"{source}: K must be finite and positive")
    if raw[32:64] != RULE_SET.ljust(32, b"\0"):
        raise ValueError(f"{source}: unexpected rule-set field")
    body = raw[HEADER_LENGTH:]
    if hashlib.sha256(body).digest() != raw[64:96]:
        raise ValueError(f"{source}: SHA-256 mismatch")

    values = []
    offset = 0
    for _, dtype, shape in _PARAMETER_SPECS:
        value, offset = _read_array(body, offset, dtype, shape)
        values.append(value)
    if offset != BODY_LENGTH:
        raise AssertionError("MNUE parameter layout is inconsistent")
    params = Parameters(*values)
    _validate_parameters(params)
    return params, float(k)


def _rounded_normal(
    rng: np.random.Generator,
    shape: tuple[int, ...],
    standard_deviation: float,
    scale: int,
    dtype: np.dtype,
) -> NDArray[np.generic]:
    """正規分布標本を量子化し、指定整数型の範囲へ制限する。"""
    limits = np.iinfo(dtype)
    values = np.rint(rng.normal(0.0, standard_deviation, size=shape) * scale)
    return np.clip(values, limits.min, limits.max).astype(dtype)


def random_network(seed: int) -> Parameters:
    """指定シードから仕様どおりのランダム初期ネットを作る。"""
    rng = np.random.default_rng(seed)
    return Parameters(
        _rounded_normal(
            rng, (FEATURE_COUNT, HIDDEN1_WIDTH), 0.02, 127, np.dtype(np.int16)
        ),
        _rounded_normal(rng, (HIDDEN1_WIDTH,), 0.1, 127, np.dtype(np.int32)),
        _rounded_normal(
            rng, (HIDDEN2_WIDTH, INPUT2_WIDTH), 0.1, 4096, np.dtype(np.int16)
        ),
        np.zeros(HIDDEN2_WIDTH, dtype=np.int32),
        _rounded_normal(
            rng, (HIDDEN3_WIDTH, HIDDEN2_WIDTH), 0.1, 4096, np.dtype(np.int16)
        ),
        np.zeros(HIDDEN3_WIDTH, dtype=np.int32),
        _rounded_normal(rng, (HIDDEN3_WIDTH,), 0.1, 4096, np.dtype(np.int16)),
        np.zeros(1, dtype=np.int32),
    )


def evaluate(
    params: Parameters,
    k: float,
    board: NDArray[np.uint8],
    stm: int | NDArray[np.uint8],
    lion: int | NDArray[np.uint8],
) -> NDArray[np.int32]:
    """局面バッチを共通仕様の整数NNUEでセンチポーン評価する。"""
    _validate_parameters(params)
    if not math.isfinite(k) or k <= 0.0:
        raise ValueError("K must be finite and positive")
    boards = np.asarray(board, dtype=np.uint8)
    if boards.ndim == 1:
        boards = boards.reshape(1, -1)
    if boards.ndim != 2 or boards.shape[1] != 144:
        raise ValueError("board must have shape (144,) or (B, 144)")
    sides = np.asarray(stm, dtype=np.uint8)
    lions = np.asarray(lion, dtype=np.uint8)
    if sides.ndim == 0:
        sides = np.full(boards.shape[0], sides, dtype=np.uint8)
    if lions.ndim == 0:
        lions = np.full(boards.shape[0], lions, dtype=np.uint8)
    if sides.shape != (boards.shape[0],) or lions.shape != (boards.shape[0],):
        raise ValueError("stm and lion must be scalars or have shape (B,)")

    padded_w1 = np.vstack(
        (params.w1, np.zeros((1, HIDDEN1_WIDTH), dtype=np.int16))
    ).astype(np.int32)
    accumulators = []
    for perspective in (0, 1):
        perspectives = np.full(boards.shape[0], perspective, dtype=np.uint8)
        indices = feature_indices(boards, perspectives, lions)
        accumulators.append(
            params.b1.astype(np.int32)[None, :] + padded_w1[indices].sum(axis=1, dtype=np.int32)
        )
    accumulators_by_perspective = np.stack(accumulators, axis=1)
    rows = np.arange(boards.shape[0])
    first = accumulators_by_perspective[rows, sides]
    second = accumulators_by_perspective[rows, 1 - sides]
    x = np.clip(np.concatenate((first, second), axis=1), 0, 127).astype(np.int32)
    y = params.b2 + x @ params.w2.astype(np.int32).T
    z = np.clip(y >> 12, 0, 127).astype(np.int32)
    y = params.b3 + z @ params.w3.astype(np.int32).T
    z = np.clip(y >> 12, 0, 127).astype(np.int32)
    out = params.bo[0] + np.sum(z * params.wo.astype(np.int32), axis=1, dtype=np.int32)
    scale = np.int64(round(k * 65536.0 / (127 * 4096)))
    centipawns = (out.astype(np.int64) * scale + 32768) >> 16
    return np.clip(centipawns, -28_999, 28_999).astype(np.int32)


def _parse_args(arguments: Sequence[str] | None) -> argparse.Namespace:
    """コマンドライン引数を解析する。"""
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    random_parser = subparsers.add_parser("random", help="ランダム初期ネットを生成する")
    random_parser.add_argument("--output", type=Path, required=True)
    random_parser.add_argument("--seed", type=int, required=True)
    random_parser.add_argument("--k", type=float, required=True)
    return parser.parse_args(arguments)


def main(arguments: Sequence[str] | None = None) -> int:
    """指定コマンドを実行し、成功時に0を返す。"""
    args = _parse_args(arguments)
    if args.command != "random":
        raise AssertionError("argparse returned an unknown command")
    params = random_network(args.seed)
    write_mnue(args.output, params, args.k)
    loaded, stored_k = read_mnue(args.output)
    boards = np.stack((INITIAL_BOARD, INITIAL_BOARD))
    scores = evaluate(
        loaded,
        stored_k,
        boards,
        np.array([0, 1], dtype=np.uint8),
        np.array([NO_LION_SQUARE, NO_LION_SQUARE], dtype=np.uint8),
    )
    print(f"initial black-to-move: {int(scores[0])} cp")
    print(f"initial white-to-move: {int(scores[1])} cp")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
