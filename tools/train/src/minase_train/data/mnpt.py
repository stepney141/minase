"""MNPT重みファイルの読み書き、初期値、および量子化を提供する。"""

from __future__ import annotations

import hashlib
import math
import struct
from pathlib import Path

import numpy as np
from numpy.typing import NDArray

from minase_train.data.features import (
    FEATURE_COUNT,
    PAWN_STATE,
    PIECE_STATE_COUNT,
    REACHABLE_NON_ROYAL_STATES,
    ROYAL_STATES,
)


HEADER_LENGTH = 80
FORMAT_VERSION = 2
FM_FORMAT_VERSION = 3
RULE_SET = b"L0,P0,R1,E0"
# 設計書「MNPT形式を更新する」: 序中盤13,680 i16、終盤13,680 i16、探索用駒価値47 i32。
# MNPT本体末尾の探索用駒価値(47個のi32)。
PIECE_VALUE_BYTES = PIECE_STATE_COUNT * 4
BODY_LENGTH = FEATURE_COUNT * 2 * 2 + PIECE_STATE_COUNT * 4
FILE_LENGTH = HEADER_LENGTH + BODY_LENGTH
EVALUATION_LIMIT = 28_999
PIECE_VALUES = np.array(
    [
        100, 125, 375, 375, 500, 500, 625, 750, 875, 1000,
        1500, 2600, 503, 375, 380, 250, 250, 378, 385, 383,
        2500, 2600, 875, 625, 1000, 1000, 750, 1250, 1375,
    ],
    dtype=np.int32,
)
PROMOTABLE_KINDS = np.array(
    [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16, 17, 18, 19],
    dtype=np.int32,
)
# 駒状態番号: 成駒または成れない駒は駒種番号(0..28)、成れる駒は29以降。
STATE_KIND = np.arange(PIECE_STATE_COUNT, dtype=np.int32)
STATE_KIND[29:] = PROMOTABLE_KINDS


def validate_piece_values(values: NDArray[np.int32], source: str) -> None:
    """探索用駒価値の正値、王駒の整合、および範囲を検査する。"""
    values = np.asarray(values)
    if values.shape != (PIECE_STATE_COUNT,):
        raise ValueError(f"{source}: piece values must have shape ({PIECE_STATE_COUNT},)")
    if np.any(values < 0) or np.any(values > EVALUATION_LIMIT):
        raise ValueError(f"{source}: piece values must be in 0..{EVALUATION_LIMIT}")
    reachable = values[list(REACHABLE_NON_ROYAL_STATES)]
    if np.any(reachable <= 0):
        raise ValueError(f"{source}: reachable non-royal piece value is not positive")
    royal = int(reachable.max()) + int(values[PAWN_STATE])
    if any(int(values[state]) != royal for state in ROYAL_STATES):
        raise ValueError(f"{source}: royal piece values must equal {royal}")


def _rule_field(rule_set: bytes) -> bytes:
    """規則セット名のUTF-8とNUL埋めの規約を検査する。"""
    if len(rule_set) > 32 or b"\0" in rule_set:
        raise ValueError("invalid rule-set name")
    rule_set.decode("utf-8")
    return rule_set.ljust(32, b"\0")


def write_mnpt(
    path: str | Path,
    middlegame: NDArray[np.int16],
    endgame: NDArray[np.int16],
    piece_values: NDArray[np.int32],
    k: float,
    *,
    rule_set: bytes = RULE_SET,
) -> None:
    """両端点の量子化重みと固定駒価値を検査和付きMNPTファイルへ書く。"""
    middlegame = np.asarray(middlegame, dtype="<i2")
    endgame = np.asarray(endgame, dtype="<i2")
    if middlegame.shape != (FEATURE_COUNT,) or endgame.shape != (FEATURE_COUNT,):
        raise ValueError(f"weights must have shape ({FEATURE_COUNT},)")
    if not math.isfinite(k) or k <= 0.0:
        raise ValueError("K must be finite and positive")
    validate_piece_values(piece_values, str(path))
    body = (
        middlegame.tobytes()
        + endgame.tobytes()
        + np.asarray(piece_values, dtype="<i4").tobytes()
    )
    _write_body(path, body, k, FORMAT_VERSION, rule_set)


def read_mnpt(
    path: str | Path,
    *,
    rule_set: bytes = RULE_SET,
) -> tuple[NDArray[np.int16], NDArray[np.int16], NDArray[np.int32], float]:
    """MNPTファイルを完全検証し、両端点の量子化重み、駒価値、Kを返す。"""
    source = Path(path)
    raw = source.read_bytes()
    if len(raw) < HEADER_LENGTH:
        raise ValueError(f"{source}: truncated MNPT header")
    if len(raw) != FILE_LENGTH:
        raise ValueError(f"{source}: length must be {FILE_LENGTH}, got {len(raw)}")
    body, k = _decode_header(raw, source, FORMAT_VERSION, rule_set)
    return (*_decode_pst(body, source), k)


def _decode_header(raw: bytes, source: str | Path, expected_version: int,
                   rule_set: bytes = RULE_SET) -> tuple[bytes, float]:
    magic, version, stored_feature_count, k = struct.unpack_from("<4sII f", raw)
    if magic != b"MNPT":
        raise ValueError(f"{source}: invalid MNPT magic {magic!r}")
    if version != expected_version:
        raise ValueError(f"{source}: unsupported MNPT version {version}")
    if stored_feature_count != FEATURE_COUNT:
        raise ValueError(f"{source}: feature count must be {FEATURE_COUNT}")
    if not math.isfinite(k) or k <= 0.0:
        raise ValueError(f"{source}: K must be finite and positive")
    if raw[16:48] != _rule_field(rule_set):
        raise ValueError(f"{source}: unexpected rule-set field")
    body = raw[HEADER_LENGTH:]
    if hashlib.sha256(body).digest() != raw[48:80]:
        raise ValueError(f"{source}: SHA-256 mismatch")
    return body, float(k)


def _decode_pst(body: bytes, source: str | Path) -> tuple[NDArray, NDArray, NDArray]:
    weights = np.frombuffer(body, dtype="<i2", count=FEATURE_COUNT * 2).reshape(2, FEATURE_COUNT)
    values = np.frombuffer(body, dtype="<i4", count=PIECE_STATE_COUNT, offset=FEATURE_COUNT * 4).copy()
    validate_piece_values(values, str(source))
    return weights[0].copy(), weights[1].copy(), values


def _write_body(path: str | Path, body: bytes, k: float, version: int, rule_set: bytes) -> None:
    if not math.isfinite(k) or not 0 < k <= np.finfo(np.float32).max:
        raise ValueError("K must be finite and positive in float32")
    encoded_k = struct.pack("<f", k)
    if struct.unpack("<f", encoded_k)[0] <= 0:
        raise ValueError("K underflows float32")
    header = (b"MNPT" + struct.pack("<II", version, FEATURE_COUNT) + encoded_k
              + _rule_field(rule_set) + hashlib.sha256(body).digest())
    Path(path).write_bytes(header + body)


def initial_weights() -> NDArray[np.int16]:
    """v0駒価値を全升へ配置した量子化初期重みを作る。"""
    weights = np.zeros(FEATURE_COUNT, dtype=np.int32)
    for relative_color, sign in ((0, 1), (1, -1)):
        for state, kind in enumerate(STATE_KIND):
            start = (relative_color * PIECE_STATE_COUNT + state) * 144
            weights[start : start + 144] = sign * PIECE_VALUES[kind] * 8
    if np.any((weights < np.iinfo(np.int16).min) | (weights > np.iinfo(np.int16).max)):
        raise OverflowError("initial weights do not fit i16")
    return weights.astype("<i2")


def initial_piece_values() -> NDArray[np.int32]:
    """v0駒価値を駒状態番号順に並べた探索用駒価値を作る。"""
    return PIECE_VALUES[STATE_KIND].astype(np.int32)


def quantize(weights: NDArray[np.float32]) -> NDArray[np.int16]:
    """センチポーン重みを1/8センチポーン単位のi16へ量子化する。"""
    if not np.all(np.isfinite(weights)):
        raise ValueError("trained weights contain a non-finite value")
    rounded = np.rint(weights.astype(np.float64) * 8.0)
    if np.any((rounded < np.iinfo(np.int16).min) | (rounded > np.iinfo(np.int16).max)):
        raise OverflowError("trained weight does not fit i16")
    return rounded.astype("<i2")


def float_weights_path(output: str | Path) -> Path:
    """MNPT出力に併置する量子化前の重みファイルのパスを返す。"""
    output = Path(output)
    return output.with_name(output.stem + "-float.npz")


def _integer_array(values: NDArray, shape: tuple[int, ...], dtype: str, name: str) -> NDArray:
    """整数配列を範囲検査してから、保存時の型へ変換する。"""
    values = np.asarray(values)
    bounds = np.iinfo(dtype)
    if (values.shape != shape or not np.issubdtype(values.dtype, np.integer)
            or np.any(values < bounds.min) or np.any(values > bounds.max)):
        raise ValueError(f"{name} must have shape {shape} and fit {dtype}")
    return values.astype(dtype)


def write_mnpt_v3(
    path: str | Path, middlegame: NDArray, endgame: NDArray, piece_values: NDArray,
    k: float, embeddings: NDArray, signs: NDArray, exponent: int,
) -> None:
    """固定PST、駒価値、尺度と量子化FMを、検査和付きv3へ書く。"""
    embeddings = np.asarray(embeddings)
    if embeddings.ndim != 2 or not 1 <= embeddings.shape[1] <= 2**32 - 1:
        raise ValueError("FM rank must be positive and fit u32")
    rank = embeddings.shape[1]
    if type(exponent) is not int or not 0 <= exponent <= 30:
        raise ValueError("FM exponent must be in 0..30")
    mg = _integer_array(middlegame, (FEATURE_COUNT,), "<i2", "middlegame")
    eg = _integer_array(endgame, (FEATURE_COUNT,), "<i2", "endgame")
    values = _integer_array(piece_values, (47,), "<i4", "piece values")
    validate_piece_values(values, str(path))
    u = _integer_array(embeddings, (FEATURE_COUNT, rank), "<i2", "embeddings")
    d = _integer_array(signs, (rank,), "i1", "signs")
    if not np.all((d == 1) | (d == -1)):
        raise ValueError("FM signs must be +1 or -1")
    body = mg.tobytes() + eg.tobytes() + values.tobytes()
    body += struct.pack("<II", rank, exponent) + d.tobytes() + u.tobytes()
    _write_body(path, body, k, FM_FORMAT_VERSION, RULE_SET)


def read_mnpt_v3(path: str | Path) -> tuple[NDArray, NDArray, NDArray, float, NDArray, NDArray, int]:
    """v3を完全検証し、両端点、駒価値、K、U、符号、指数を返す。"""
    raw = Path(path).read_bytes()
    if len(raw) < HEADER_LENGTH + BODY_LENGTH + 8:
        raise ValueError(f"{path}: truncated MNPT v3")
    body, k = _decode_header(raw, path, FM_FORMAT_VERSION)
    rank, exponent = struct.unpack_from("<II", body, BODY_LENGTH)
    if rank < 1 or exponent > 30:
        raise ValueError(f"{path}: invalid FM rank or exponent")
    if len(body) != BODY_LENGTH + 8 + rank + FEATURE_COUNT * rank * 2:
        raise ValueError(f"{path}: invalid MNPT v3 length")
    signs = np.frombuffer(body, dtype="i1", count=rank, offset=BODY_LENGTH + 8).copy()
    if not np.all((signs == 1) | (signs == -1)):
        raise ValueError(f"{path}: FM signs must be +1 or -1")
    u = np.frombuffer(body, dtype="<i2", offset=BODY_LENGTH + 8 + rank).reshape(FEATURE_COUNT, rank).copy()
    return (*_decode_pst(body, path), k, u, signs, exponent)


def validate_fixed_base(base_path: str | Path, candidate_path: str | Path) -> None:
    """v3の両端点、探索用駒価値、およびKが基準v2と厳密に一致することを確かめる。"""
    base = read_mnpt(base_path)
    candidate = read_mnpt_v3(candidate_path)
    for name, expected, actual in zip(("middlegame", "endgame", "piece values", "K"), base, candidate[:4]):
        if not np.array_equal(expected, actual):
            raise ValueError(f"FM candidate changed fixed {name}")
