"""MNUE版5のヘッダ、本体、範囲、および鏡映共有条件を検証して読み書きする。"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import math
from pathlib import Path
import struct

import numpy as np
from numpy.typing import NDArray

from minase_train.data.features import FEATURE_COUNT, PIECE_STATE_COUNT
from minase_train.data.mnpt import RULE_SET, validate_piece_values
from minase_train.nnue.feature_sets import FEATURE_SETS, get_feature_set


HEADER_LENGTH = 192
FORMAT_VERSION = 5
FIRST_LAYER_LIMIT = 1016
OUTPUT_WEIGHT_LIMIT = 8192
OUTPUT_BIAS_LIMIT = 4_161_536


@dataclass(frozen=True)
class Network:
    """検証済みの整数重みを持つ。出力重みは(2, H)で視点順に並ぶ。"""

    feature_set: str
    linear: NDArray[np.int16]
    piece_values: NDArray[np.int32]
    k: float
    weights: NDArray[np.int16]
    biases: NDArray[np.int32]
    output_weights: NDArray[np.int16]
    output_bias: int

    @property
    def width(self) -> int:
        """第1層の隠れユニット数を返す。"""
        return self.weights.shape[1]


def validate_width(width: int, source: str) -> None:
    """MNUE版5が許す偶数幅を検査する。"""
    if (isinstance(width, (bool, np.bool_)) or not isinstance(width, (int, np.integer))
            or not 2 <= width <= 512 or width % 2):
        raise ValueError(f"{source}: width must be even and in 2..512")


def _validate_k(k: float, source: str) -> None:
    if not math.isfinite(k) or not 0 < k <= 100_000:
        raise ValueError(f"{source}: K must be finite and in (0, 100000]")


def _integer_array(values: NDArray, shape: tuple[int, ...], dtype: str,
                   name: str, source: str, limit: int | None = None) -> NDArray:
    values = np.asarray(values)
    bounds = np.iinfo(dtype)
    lower, upper = (bounds.min, bounds.max) if limit is None else (-limit, limit)
    if (values.shape != shape or values.dtype.kind not in "iu"
            or np.any(values < lower) or np.any(values > upper)):
        raise ValueError(f"{source}: {name} must be integers of shape {shape} in {lower}..{upper}")
    return values.astype(dtype)


def architecture_string(feature_set: str, width: int) -> str:
    """入力特徴集合と幅から、版5の構造記述文字列を組み立てる。"""
    spec = get_feature_set(feature_set)
    validate_width(width, "MNUE architecture")
    return f"features={spec.name};ft={width};linear=tapered2;head=direct-screlu"


def _network(linear: NDArray, piece_values: NDArray, k: float, weights: NDArray,
             biases: NDArray, output_weights: NDArray, output_bias: int, feature_set: str, source: str) -> Network:
    spec = get_feature_set(feature_set)
    weights = np.asarray(weights)
    if weights.ndim != 2:
        raise ValueError(f"{source}: weights must have shape ({spec.feature_count}, H)")
    width = weights.shape[1]
    validate_width(width, source)
    _validate_k(k, source)
    linear = _integer_array(linear, (FEATURE_COUNT, 2), "<i2", "linear", source)
    values = _integer_array(piece_values, (PIECE_STATE_COUNT,), "<i4", "piece values", source)
    validate_piece_values(values, source)
    weights = _integer_array(weights, (spec.feature_count, width), "<i2", "weights", source, FIRST_LAYER_LIMIT)
    biases = _integer_array(biases, (width,), "<i4", "biases", source, FIRST_LAYER_LIMIT)
    output = _integer_array(output_weights, (2, width), "<i2", "output weights", source, OUTPUT_WEIGHT_LIMIT)
    bias = _integer_array(np.asarray(output_bias), (), "<i4", "output bias", source, OUTPUT_BIAS_LIMIT)
    mirrored_linear = linear.reshape(95, 12, 12, 2)[:, :, ::-1].reshape(FEATURE_COUNT, 2)
    if not np.array_equal(linear, mirrored_linear):
        raise ValueError(f"{source}: linear mirror sharing violated")
    if spec.mirror == "shared":
        mirrored_weights = weights.reshape(95, 12, 12, width)[:, :, ::-1].reshape(FEATURE_COUNT, width)
        if not np.array_equal(mirrored_weights, np.roll(weights, width // 2, axis=1)):
            raise ValueError(f"{source}: first-layer mirror sharing violated")
        if not np.array_equal(biases, np.roll(biases, width // 2)):
            raise ValueError(f"{source}: bias mirror sharing violated")
        if not np.array_equal(output, np.roll(output, width // 2, axis=1)):
            raise ValueError(f"{source}: output mirror sharing violated")
    return Network(spec.name, linear, values, float(k), weights, biases, output, int(bias))


def write_mnue(path: str | Path, linear: NDArray, piece_values: NDArray, k: float,
               weights: NDArray, biases: NDArray, output_weights: NDArray, output_bias: int,
               *, feature_set: str) -> None:
    """全検査に通った配列だけを、リトルエンディアンのMNUE版5へ保存する。"""
    source = str(path)
    _validate_k(k, source)
    stored_k = struct.unpack("<f", struct.pack("<f", k))[0]
    net = _network(linear, piece_values, stored_k, weights, biases, output_weights, output_bias, feature_set, source)
    body = b"".join(array.tobytes() for array in (
        net.linear, net.piece_values, net.weights, net.biases, net.output_weights,
    )) + struct.pack("<i", net.output_bias)
    architecture = architecture_string(net.feature_set, net.width).encode("ascii")
    header = (struct.pack("<4sI", b"MNUE", FORMAT_VERSION) + architecture.ljust(96, b"\0")
              + struct.pack("<If", net.width, net.k) + RULE_SET.ljust(32, b"\0")
              + hashlib.sha256(body).digest() + bytes(16))
    Path(path).write_bytes(header + body)


def read_mnue(path: str | Path) -> Network:
    """幅から長さと配置を導き、ヘッダ、本体、および鏡映共有をすべて検査する。"""
    source = str(path)
    raw = Path(path).read_bytes()
    if len(raw) < 8:
        raise ValueError(f"{source}: truncated MNUE header")
    magic, version = struct.unpack_from("<4sI", raw)
    if magic != b"MNUE":
        raise ValueError(f"{source}: invalid MNUE magic")
    if version != FORMAT_VERSION:
        raise ValueError(f"{source}: unsupported version {version}")
    if len(raw) < HEADER_LENGTH:
        raise ValueError(f"{source}: truncated MNUE header")
    width, k = struct.unpack_from("<If", raw, 104)
    validate_width(width, source)
    _validate_k(k, source)
    for spec in FEATURE_SETS.values():
        expected = architecture_string(spec.name, width).encode("ascii").ljust(96, b"\0")
        if raw[8:104] == expected:
            break
    else:
        raise ValueError(f"{source}: unsupported architecture string")
    if raw[176:192] != bytes(16):
        raise ValueError(f"{source}: reserved fields must be zero")
    if raw[112:144] != RULE_SET.ljust(32, b"\0"):
        raise ValueError(f"{source}: unexpected rule-set field")
    length = HEADER_LENGTH + FEATURE_COUNT * 4 + PIECE_STATE_COUNT * 4 + spec.feature_count * 2 * width + 8 * width + 4
    if len(raw) != length:
        raise ValueError(f"{source}: length must be {length}, got {len(raw)}")
    body = raw[HEADER_LENGTH:]
    if hashlib.sha256(body).digest() != raw[144:176]:
        raise ValueError(f"{source}: SHA-256 mismatch")
    offset = 0

    def take(dtype: str, shape: tuple[int, ...]) -> NDArray:
        nonlocal offset
        count = math.prod(shape)
        array = np.frombuffer(body, dtype=dtype, count=count, offset=offset).reshape(shape)
        offset += count * np.dtype(dtype).itemsize
        return array

    linear = take("<i2", (FEATURE_COUNT, 2))
    values = take("<i4", (PIECE_STATE_COUNT,))
    weights = take("<i2", (spec.feature_count, width))
    biases = take("<i4", (width,))
    output = take("<i2", (2, width))
    bias = int(take("<i4", ()))
    return _network(linear, values, k, weights, biases, output, bias, spec.name, source)
