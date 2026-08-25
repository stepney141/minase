"""MNSDから線形PSTを学習し、MNPT重みファイルを生成する。"""

from __future__ import annotations

import argparse
import hashlib
import math
from pathlib import Path
import struct
from typing import Sequence

import numpy as np
from numpy.typing import NDArray
import torch
from torch import Tensor, nn
from torch.nn import functional as torch_functional

from features import FEATURE_COUNT, INITIAL_BOARD, PADDING_INDEX, feature_indices, mirror
from mnsd import NO_LION_SQUARE, load_records


HEADER_LENGTH = 80
FORMAT_VERSION = 1
RULE_SET = b"L0,P0,R1,E0"
# 設計書「量子化と整数推論」の合格条件: 浮動小数点評価との平均絶対誤差2センチポーン以下。
QUANTIZATION_ERROR_LIMIT = 2.0
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


def write_mnpt(path: str | Path, weights: NDArray[np.int16], k: float) -> None:
    """量子化済み重みを検査和付きMNPTファイルへ書く。"""
    weights = np.asarray(weights, dtype="<i2")
    if weights.shape != (FEATURE_COUNT,):
        raise ValueError(f"weights must have shape ({FEATURE_COUNT},)")
    if not math.isfinite(k) or k <= 0.0:
        raise ValueError("K must be finite and positive")
    body = weights.tobytes()
    rule_field = RULE_SET.ljust(32, b"\0")
    header = (
        b"MNPT"
        + struct.pack("<II f", FORMAT_VERSION, FEATURE_COUNT, k)
        + rule_field
        + hashlib.sha256(body).digest()
    )
    if len(header) != HEADER_LENGTH:
        raise AssertionError("MNPT header length is inconsistent")
    Path(path).write_bytes(header + body)


def read_mnpt(path: str | Path) -> tuple[NDArray[np.int16], float]:
    """MNPTファイルを完全検証し、量子化重みとKを返す。"""
    source = Path(path)
    raw = source.read_bytes()
    expected_length = HEADER_LENGTH + FEATURE_COUNT * 2
    if len(raw) != expected_length:
        raise ValueError(f"{source}: length must be {expected_length}, got {len(raw)}")
    magic, version, feature_count, k = struct.unpack_from("<4sII f", raw)
    if magic != b"MNPT":
        raise ValueError(f"{source}: invalid MNPT magic {magic!r}")
    if version != FORMAT_VERSION:
        raise ValueError(f"{source}: unsupported MNPT version {version}")
    if feature_count != FEATURE_COUNT:
        raise ValueError(f"{source}: feature count must be {FEATURE_COUNT}")
    if not math.isfinite(k) or k <= 0.0:
        raise ValueError(f"{source}: K must be finite and positive")
    rule_field = raw[16:48]
    if rule_field != RULE_SET.ljust(32, b"\0"):
        raise ValueError(f"{source}: unexpected rule-set field")
    body = raw[HEADER_LENGTH:]
    if hashlib.sha256(body).digest() != raw[48:80]:
        raise ValueError(f"{source}: SHA-256 mismatch")
    return np.frombuffer(body, dtype="<i2").copy(), float(k)


def initial_weights() -> NDArray[np.int16]:
    """v0駒価値を全升へ配置した量子化初期重みを作る。"""
    state_kind = np.arange(47, dtype=np.int32)
    state_kind[29:] = PROMOTABLE_KINDS
    weights = np.zeros(FEATURE_COUNT, dtype=np.int32)
    for relative_color, sign in ((0, 1), (1, -1)):
        for state, kind in enumerate(state_kind):
            start = (relative_color * 47 + state) * 144
            weights[start : start + 144] = sign * PIECE_VALUES[kind] * 8
    if np.any((weights < np.iinfo(np.int16).min) | (weights > np.iinfo(np.int16).max)):
        raise OverflowError("initial weights do not fit i16")
    return weights.astype("<i2")


def binary_cross_entropy_sum(scores: NDArray[np.int16], results: NDArray[np.uint8], k: float) -> float:
    """探索値から最終結果を予測する二値交差エントロピー合計を返す。"""
    logits = scores.astype(np.float64) / k
    targets = results.astype(np.float64) / 2.0
    return float(np.sum(np.logaddexp(0.0, logits) - targets * logits, dtype=np.float64))


def estimate_k(scores: NDArray[np.int16], results: NDArray[np.uint8]) -> float:
    """区間50から4000で損失を最小化するKを黄金分割探索で求める。"""
    lower = 50.0
    upper = 4000.0
    ratio = (math.sqrt(5.0) - 1.0) / 2.0
    left = upper - ratio * (upper - lower)
    right = lower + ratio * (upper - lower)
    left_loss = binary_cross_entropy_sum(scores, results, left)
    right_loss = binary_cross_entropy_sum(scores, results, right)
    for _ in range(64):
        if left_loss <= right_loss:
            upper = right
            right = left
            right_loss = left_loss
            left = upper - ratio * (upper - lower)
            left_loss = binary_cross_entropy_sum(scores, results, left)
        else:
            lower = left
            left = right
            left_loss = right_loss
            right = lower + ratio * (upper - lower)
            right_loss = binary_cross_entropy_sum(scores, results, right)
    return (lower + upper) / 2.0


def make_model(initial: Tensor, device: torch.device) -> nn.Embedding:
    """指定初期値からpadding行付き線形PSTモデルを作る。"""
    model = nn.Embedding(FEATURE_COUNT + 1, 1, padding_idx=PADDING_INDEX, device=device)
    with torch.no_grad():
        model.weight.zero_()
        model.weight[:FEATURE_COUNT, 0].copy_(initial)
    return model


def model_logits(model: nn.Embedding, features: Tensor, k: float) -> Tensor:
    """特徴番号バッチから勝率ロジットを計算する。"""
    return model(features).sum(dim=1).squeeze(1) / k


def validation_loss(model: nn.Embedding, features: Tensor, targets: Tensor, k: float, batch: int) -> float:
    """検証集合全体の平均二値交差エントロピーを計算する。"""
    total = 0.0
    with torch.no_grad():
        for start in range(0, features.shape[0], batch):
            end = min(start + batch, features.shape[0])
            loss = torch_functional.binary_cross_entropy_with_logits(
                model_logits(model, features[start:end], k),
                targets[start:end],
                reduction="sum",
            )
            total += float(loss.item())
    return total / features.shape[0]


def train_epoch(
    model: nn.Embedding,
    optimizer: torch.optim.Optimizer,
    features: Tensor,
    mirrored_features: Tensor,
    targets: Tensor,
    k: float,
    batch: int,
    generator: torch.Generator,
) -> float:
    """鏡映を標本ごとに選び、訓練集合を1エポック学習する。"""
    order = torch.randperm(features.shape[0], generator=generator, device=features.device)
    total = 0.0
    for start in range(0, order.shape[0], batch):
        indices = order[start : start + batch]
        normal = features[indices]
        reflected = mirrored_features[indices]
        choose_reflected = torch.rand(
            indices.shape[0], generator=generator, device=features.device
        ) < 0.5
        selected = torch.where(choose_reflected[:, None], reflected, normal)
        selected_targets = targets[indices]
        optimizer.zero_grad(set_to_none=True)
        loss = torch_functional.binary_cross_entropy_with_logits(
            model_logits(model, selected, k), selected_targets
        )
        loss.backward()
        optimizer.step()
        total += float(loss.item()) * indices.shape[0]
    return total / features.shape[0]


def build_targets(records: np.ndarray, k: float, lambda_value: float) -> NDArray[np.float32]:
    """探索値と最終結果を混合した教師勝率を作る。"""
    score_probability = 1.0 / (1.0 + np.exp(-records["score"].astype(np.float64) / k))
    result_probability = records["result"].astype(np.float64) / 2.0
    return (
        lambda_value * score_probability + (1.0 - lambda_value) * result_probability
    ).astype(np.float32)


def quantize(weights: NDArray[np.float32]) -> NDArray[np.int16]:
    """センチポーン重みを1/8センチポーン単位のi16へ量子化する。"""
    if not np.all(np.isfinite(weights)):
        raise ValueError("trained weights contain a non-finite value")
    rounded = np.rint(weights.astype(np.float64) * 8.0)
    if np.any((rounded < np.iinfo(np.int16).min) | (rounded > np.iinfo(np.int16).max)):
        raise OverflowError("trained weight does not fit i16")
    return rounded.astype("<i2")


def integer_evaluate(weights: NDArray[np.int16], features: NDArray[np.int32]) -> NDArray[np.int32]:
    """共通仕様どおり量子化重みで局面バッチを評価する。"""
    extended = np.zeros(FEATURE_COUNT + 1, dtype=np.int32)
    extended[:FEATURE_COUNT] = weights.astype(np.int32)
    sums = extended[features].sum(axis=1, dtype=np.int32)
    values = np.trunc(sums.astype(np.float64) / 8.0).astype(np.int32)
    return np.clip(values, -28_999, 28_999)


def initial_position_score(weights: NDArray[np.int16]) -> int:
    """量子化重みで中将棋初期局面を先手視点から評価する。"""
    features = feature_indices(
        INITIAL_BOARD[None, :],
        np.array([0], dtype=np.uint8),
        np.array([NO_LION_SQUARE], dtype=np.uint8),
    )
    return int(integer_evaluate(weights, features)[0])


def command_init(arguments: argparse.Namespace) -> None:
    """initサブコマンドを実行する。"""
    weights = initial_weights()
    write_mnpt(arguments.output, weights, arguments.k)
    print(f"initial position evaluation: {initial_position_score(weights)} cp")


def command_estimate_k(arguments: argparse.Namespace) -> None:
    """estimate-kサブコマンドを実行する。"""
    records = load_records(arguments.data)
    training = records[records["game"] % 20 != 0]
    k = estimate_k(training["score"], training["result"])
    loss = binary_cross_entropy_sum(training["score"], training["result"], k)
    print(f"K: {k:.9f}")
    print(f"training BCE sum: {loss:.9f}")


def command_train(arguments: argparse.Namespace) -> None:
    """trainサブコマンドを実行する。"""
    if not 0.0 <= arguments.lambda_value <= 1.0:
        raise ValueError("--lambda must be in 0..1")
    if arguments.epochs <= 0 or arguments.batch <= 0 or arguments.validation_sample <= 0:
        raise ValueError("--epochs, --batch, and --validation-sample must be positive")
    if any(rate <= 0.0 or not math.isfinite(rate) for rate in arguments.lr):
        raise ValueError("every --lr value must be finite and positive")
    if arguments.k <= 0.0 or not math.isfinite(arguments.k):
        raise ValueError("--k must be finite and positive")

    torch.manual_seed(arguments.seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(arguments.seed)
    device = torch.device(arguments.device)
    records = load_records(arguments.data)
    validation_mask = records["game"] % 20 == 0
    if not np.any(validation_mask) or np.all(validation_mask):
        raise ValueError("game split produced an empty training or validation set")

    print(f"records: total={records.size} training={np.count_nonzero(~validation_mask)} validation={np.count_nonzero(validation_mask)}")
    print("building feature indices")
    normal = feature_indices(records["board"], records["stm"], records["lion"])
    mirrored_board, mirrored_lion = mirror(records["board"], records["lion"])
    reflected = feature_indices(mirrored_board, records["stm"], mirrored_lion)
    targets = build_targets(records, arguments.k, arguments.lambda_value)

    training_features = torch.as_tensor(normal[~validation_mask], device=device)
    training_reflected = torch.as_tensor(reflected[~validation_mask], device=device)
    training_targets = torch.as_tensor(targets[~validation_mask], device=device)
    validation_features = torch.as_tensor(normal[validation_mask], device=device)
    validation_targets = torch.as_tensor(targets[validation_mask], device=device)

    initial_quantized, _ = read_mnpt(arguments.init)
    initial = torch.as_tensor(initial_quantized.astype(np.float32) / 8.0, device=device)

    selected_rate = arguments.lr[0]
    if len(arguments.lr) > 1:
        candidates: list[tuple[float, float]] = []
        for rate in arguments.lr:
            model = make_model(initial, device)
            optimizer = torch.optim.Adam(model.parameters(), lr=rate)
            generator = torch.Generator(device=device).manual_seed(arguments.seed)
            training_loss = train_epoch(
                model,
                optimizer,
                training_features,
                training_reflected,
                training_targets,
                arguments.k,
                arguments.batch,
                generator,
            )
            loss = validation_loss(
                model,
                validation_features,
                validation_targets,
                arguments.k,
                arguments.batch,
            )
            candidates.append((loss, rate))
            print(f"learning-rate candidate: lr={rate:g} train_loss={training_loss:.9f} validation_loss={loss:.9f}")
        selected_rate = min(candidates)[1]
    print(f"selected learning rate: {selected_rate:g}")

    model = make_model(initial, device)
    optimizer = torch.optim.Adam(model.parameters(), lr=selected_rate)
    generator = torch.Generator(device=device).manual_seed(arguments.seed)
    for epoch in range(1, arguments.epochs + 1):
        training_loss = train_epoch(
            model,
            optimizer,
            training_features,
            training_reflected,
            training_targets,
            arguments.k,
            arguments.batch,
            generator,
        )
        loss = validation_loss(
            model,
            validation_features,
            validation_targets,
            arguments.k,
            arguments.batch,
        )
        print(f"epoch {epoch}: train_loss={training_loss:.9f} validation_loss={loss:.9f}")

    float_weights = model.weight[:FEATURE_COUNT, 0].detach().cpu().numpy().astype(np.float32)
    quantized = quantize(float_weights)

    validation_count = validation_features.shape[0]
    sample_count = min(arguments.validation_sample, validation_count)
    random = np.random.default_rng(arguments.seed)
    sample_indices = random.choice(validation_count, size=sample_count, replace=False)
    sample_features = normal[validation_mask][sample_indices]
    extended_float = np.zeros(FEATURE_COUNT + 1, dtype=np.float32)
    extended_float[:FEATURE_COUNT] = float_weights
    floating_scores = extended_float[sample_features].sum(axis=1, dtype=np.float32)
    integer_scores = integer_evaluate(quantized, sample_features)
    errors = np.abs(floating_scores - integer_scores.astype(np.float32))
    mean_absolute_error = float(errors.mean())
    print(f"quantization error: samples={sample_count} mean_absolute={mean_absolute_error:.9f} max={float(errors.max()):.9f} cp")
    if not math.isfinite(mean_absolute_error) or mean_absolute_error > QUANTIZATION_ERROR_LIMIT:
        raise ValueError(
            f"quantization mean absolute error {mean_absolute_error} exceeds {QUANTIZATION_ERROR_LIMIT} cp"
        )
    write_mnpt(arguments.output, quantized, arguments.k)
    print(f"initial position evaluation: {initial_position_score(quantized)} cp")


def build_parser() -> argparse.ArgumentParser:
    """学習器CLIの引数解析器を作る。"""
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)

    init_parser = commands.add_parser("init", help="v0駒価値でMNPTを初期化する")
    init_parser.add_argument("--output", required=True)
    init_parser.add_argument("--k", required=True, type=float)
    init_parser.set_defaults(handler=command_init)

    estimate_parser = commands.add_parser("estimate-k", help="探索値の勝率尺度Kを推定する")
    estimate_parser.add_argument("--data", required=True, nargs="+")
    estimate_parser.set_defaults(handler=command_estimate_k)

    train_parser = commands.add_parser("train", help="学習PSTを訓練する")
    train_parser.add_argument("--data", required=True, nargs="+")
    train_parser.add_argument("--output", required=True)
    train_parser.add_argument("--init", required=True)
    train_parser.add_argument("--k", required=True, type=float)
    train_parser.add_argument("--lambda", dest="lambda_value", type=float, default=0.75)
    train_parser.add_argument("--lr", type=float, nargs="+", required=True)
    train_parser.add_argument("--epochs", type=int, default=10)
    train_parser.add_argument("--batch", type=int, default=16384)
    train_parser.add_argument("--seed", type=int, default=1)
    train_parser.add_argument("--validation-sample", type=int, default=10000)
    train_parser.add_argument("--device", default="cuda")
    train_parser.set_defaults(handler=command_train)
    return parser


def main(arguments: Sequence[str] | None = None) -> None:
    """CLI引数を解析し、選択されたサブコマンドを実行する。"""
    parsed = build_parser().parse_args(arguments)
    parsed.handler(parsed)


if __name__ == "__main__":
    main()
