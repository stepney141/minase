"""MNSDから線形PSTを学習し、MNPT重みファイルを生成する。"""

from __future__ import annotations

import argparse
import hashlib
import math
from pathlib import Path
import resource
import struct
import time
from typing import Sequence

import numpy as np
from numpy.typing import NDArray
import torch
from torch import Tensor, nn
from torch.nn import functional as torch_functional

from features import FEATURE_COUNT, INITIAL_BOARD, PADDING_INDEX, feature_indices, mirror
from mnsd import Dataset, NO_LION_SQUARE


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


def _training_scores_results(
    dataset: Dataset, generation: int | None = None
) -> tuple[NDArray[np.int16], NDArray[np.uint8]]:
    """指定世代または全世代の訓練用探索値と結果を集める。"""
    score_chunks: list[NDArray[np.int16]] = []
    result_chunks: list[NDArray[np.uint8]] = []
    for file_generation, records, validation in zip(
        dataset.file_generations, dataset.records, dataset.validation_masks
    ):
        if generation is not None and file_generation != generation:
            continue
        training = ~validation
        score_chunks.append(np.asarray(records["score"][training], dtype=np.int16))
        result_chunks.append(np.asarray(records["result"][training], dtype=np.uint8))
    scores = np.concatenate(score_chunks)
    results = np.concatenate(result_chunks)
    if scores.size == 0:
        scope = "dataset" if generation is None else f"generation {generation}"
        raise ValueError(f"{scope} has no training records")
    return scores, results


def estimate_generation_ks(dataset: Dataset) -> tuple[NDArray[np.float64], list[int]]:
    """各世代の訓練レコードから教師Kと件数を求める。"""
    values = np.empty(dataset.generation_count, dtype=np.float64)
    counts: list[int] = []
    for generation in range(dataset.generation_count):
        scores, results = _training_scores_results(dataset, generation)
        values[generation] = estimate_k(scores, results)
        counts.append(int(scores.size))
    return values, counts


def validation_loss(
    model: nn.Embedding,
    dataset: Dataset,
    teacher_ks: NDArray[np.float64],
    k: float,
    lambda_value: float,
    batch: int,
    device: torch.device,
) -> tuple[float, NDArray[np.float64]]:
    """検証損失をバッチ単位で計算し、全体値と世代別値を返す。"""
    generation_sums = np.zeros(dataset.generation_count, dtype=np.float64)
    generation_counts = np.zeros(dataset.generation_count, dtype=np.int64)
    with torch.no_grad():
        for start in range(0, dataset.validation_indices.size, batch):
            global_indices = dataset.validation_indices[start : start + batch]
            records = dataset.gather(global_indices)
            generations = dataset.generations(global_indices)
            features = feature_indices(
                records["board"], records["stm"], records["lion"]
            )
            targets = build_targets(records, teacher_ks, generations, lambda_value)
            device_features = torch.as_tensor(features, device=device)
            device_targets = torch.as_tensor(targets, device=device)
            losses = torch_functional.binary_cross_entropy_with_logits(
                model_logits(model, device_features, k),
                device_targets,
                reduction="none",
            ).cpu().numpy().astype(np.float64)
            generation_sums += np.bincount(
                generations, weights=losses, minlength=dataset.generation_count
            )
            generation_counts += np.bincount(
                generations, minlength=dataset.generation_count
            )
    generation_losses = np.divide(
        generation_sums,
        generation_counts,
        out=np.full(dataset.generation_count, np.nan, dtype=np.float64),
        where=generation_counts != 0,
    )
    return (
        float(generation_sums.sum() / generation_counts.sum()),
        generation_losses,
    )


def train_epoch(
    model: nn.Embedding,
    optimizer: torch.optim.Optimizer,
    dataset: Dataset,
    teacher_ks: NDArray[np.float64],
    k: float,
    lambda_value: float,
    batch: int,
    generator: torch.Generator,
    device: torch.device,
    count_features: bool = False,
) -> tuple[float, NDArray[np.int64] | None]:
    """訓練レコードをバッチごとに読み、鏡映を選んで1エポック学習する。"""
    order = torch.randperm(
        dataset.training_indices.size, generator=generator, device=device
    )
    total = 0.0
    observations = (
        np.zeros(FEATURE_COUNT, dtype=np.int64) if count_features else None
    )
    for start in range(0, order.shape[0], batch):
        positions = order[start : start + batch]
        global_indices = dataset.training_indices[positions.cpu().numpy()]
        records = dataset.gather(global_indices)
        generations = dataset.generations(global_indices)
        normal = feature_indices(records["board"], records["stm"], records["lion"])
        mirrored_board, mirrored_lion = mirror(records["board"], records["lion"])
        reflected = feature_indices(mirrored_board, records["stm"], mirrored_lion)
        if observations is not None:
            active = normal[normal != PADDING_INDEX]
            observations += np.bincount(active, minlength=FEATURE_COUNT)
        choose_reflected = torch.rand(
            positions.shape[0], generator=generator, device=device
        ) < 0.5
        selected = normal.copy()
        reflected_rows = choose_reflected.cpu().numpy()
        selected[reflected_rows] = reflected[reflected_rows]
        targets = build_targets(records, teacher_ks, generations, lambda_value)
        device_features = torch.as_tensor(selected, device=device)
        device_targets = torch.as_tensor(targets, device=device)
        optimizer.zero_grad(set_to_none=True)
        loss = torch_functional.binary_cross_entropy_with_logits(
            model_logits(model, device_features, k), device_targets
        )
        loss.backward()
        optimizer.step()
        total += float(loss.item()) * positions.shape[0]
    return total / dataset.training_indices.size, observations


def build_targets(
    records: np.ndarray,
    teacher_ks: NDArray[np.float64],
    generations: NDArray[np.int64],
    lambda_value: float,
) -> NDArray[np.float32]:
    """世代別Kで探索値を変換し、最終結果と混合した教師勝率を作る。"""
    teacher_ks = np.asarray(teacher_ks, dtype=np.float64)
    generations = np.asarray(generations, dtype=np.int64)
    if generations.shape != (records.shape[0],):
        raise ValueError("generations must have one value per record")
    if np.any(generations < 0) or np.any(generations >= teacher_ks.size):
        raise ValueError("generation is outside teacher K values")
    scales = teacher_ks[generations]
    if np.any(scales <= 0.0) or not np.all(np.isfinite(scales)):
        raise ValueError("teacher K values must be finite and positive")
    score_probability = 1.0 / (
        1.0 + np.exp(-records["score"].astype(np.float64) / scales)
    )
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
    dataset = Dataset(arguments.data)
    generation_ks, generation_counts = estimate_generation_ks(dataset)
    for generation, (checksum, k, count) in enumerate(
        zip(dataset.generation_checksums, generation_ks, generation_counts)
    ):
        file_count = int(np.count_nonzero(dataset.file_generations == generation))
        print(
            f"generation {generation}: files={file_count} checksum={checksum.hex()} "
            f"training_records={count} K={k:.9f}"
        )
    scores, results = _training_scores_results(dataset)
    mixed_k = estimate_k(scores, results)
    print(f"mixed: training_records={scores.size} K={mixed_k:.9f}")


def _format_validation_loss(
    overall: float, generation_losses: NDArray[np.float64]
) -> str:
    """全体と世代別の検証損失を1行へ整形する。"""
    generations = " ".join(
        f"generation{generation}={loss:.9f}"
        for generation, loss in enumerate(generation_losses)
    )
    return f"validation_loss: overall={overall:.9f} {generations}"


def _print_feature_observations(observations: NDArray[np.int64]) -> None:
    """観測済み特徴の出現回数と未観測数を表示する。"""
    observed = observations[observations > 0]
    if observed.size == 0:
        raise ValueError("no training features were observed")
    percentiles = np.quantile(observed, [0.05, 0.25, 0.5, 0.75, 0.95])
    print(
        "feature observations: "
        f"unobserved={np.count_nonzero(observations == 0)} "
        f"min={observed.min()} p05={percentiles[0]:.3f} "
        f"p25={percentiles[1]:.3f} p50={percentiles[2]:.3f} "
        f"p75={percentiles[3]:.3f} p95={percentiles[4]:.3f} "
        f"max={observed.max()} mean={observed.mean():.3f}"
    )


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
    if device.type == "cuda":
        torch.cuda.reset_peak_memory_stats(device)
    dataset = Dataset(arguments.data)
    if dataset.training_indices.size == 0 or dataset.validation_indices.size == 0:
        raise ValueError("game split produced an empty training or validation set")

    teacher_ks, _ = estimate_generation_ks(dataset)
    teacher_k_log = ", ".join(
        f"generation {generation} = {k:.9f}"
        for generation, k in enumerate(teacher_ks)
    )
    print(f"teacher K: {teacher_k_log}")
    print(
        f"records: total={dataset.record_count} "
        f"training={dataset.training_indices.size} "
        f"validation={dataset.validation_indices.size}"
    )

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
                dataset,
                teacher_ks,
                arguments.k,
                arguments.lambda_value,
                arguments.batch,
                generator,
                device,
            )[0]
            loss, _ = validation_loss(
                model,
                dataset,
                teacher_ks,
                arguments.k,
                arguments.lambda_value,
                arguments.batch,
                device,
            )
            candidates.append((loss, rate))
            print(f"learning-rate candidate: lr={rate:g} train_loss={training_loss:.9f} validation_loss={loss:.9f}")
        selected_rate = min(candidates)[1]
    print(f"selected learning rate: {selected_rate:g}")

    model = make_model(initial, device)
    optimizer = torch.optim.Adam(model.parameters(), lr=selected_rate)
    generator = torch.Generator(device=device).manual_seed(arguments.seed)
    best_loss, generation_losses = validation_loss(
        model,
        dataset,
        teacher_ks,
        arguments.k,
        arguments.lambda_value,
        arguments.batch,
        device,
    )
    best_epoch = 0
    best_weights = model.weight[:FEATURE_COUNT, 0].detach().cpu().clone()
    print(f"epoch 0: {_format_validation_loss(best_loss, generation_losses)}")
    for epoch in range(1, arguments.epochs + 1):
        started = time.perf_counter()
        training_loss, observations = train_epoch(
            model,
            optimizer,
            dataset,
            teacher_ks,
            arguments.k,
            arguments.lambda_value,
            arguments.batch,
            generator,
            device,
            count_features=epoch == 1,
        )
        elapsed = time.perf_counter() - started
        positions_per_second = dataset.training_indices.size / elapsed
        loss, generation_losses = validation_loss(
            model,
            dataset,
            teacher_ks,
            arguments.k,
            arguments.lambda_value,
            arguments.batch,
            device,
        )
        print(
            f"epoch {epoch}: train_loss={training_loss:.9f} "
            f"positions_per_second={positions_per_second:.3f}"
        )
        print(f"epoch {epoch}: {_format_validation_loss(loss, generation_losses)}")
        if observations is not None:
            _print_feature_observations(observations)
        if loss < best_loss:
            best_loss = loss
            best_epoch = epoch
            best_weights = model.weight[:FEATURE_COUNT, 0].detach().cpu().clone()

    print(f"best epoch: {best_epoch} validation_loss={best_loss:.9f}")

    float_weights = best_weights.numpy().astype(np.float32)
    quantized = quantize(float_weights)

    validation_count = dataset.validation_indices.size
    sample_count = min(arguments.validation_sample, validation_count)
    random = np.random.default_rng(arguments.seed)
    sample_indices = random.choice(validation_count, size=sample_count, replace=False)
    sample_records = dataset.gather(dataset.validation_indices[sample_indices])
    sample_features = feature_indices(
        sample_records["board"], sample_records["stm"], sample_records["lion"]
    )
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
    max_rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    print(f"resource usage: max_rss={max_rss} KiB")
    if device.type == "cuda":
        print(
            "resource usage: "
            f"max_vram={torch.cuda.max_memory_allocated(device)} bytes"
        )


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
