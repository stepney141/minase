"""MNSDから線形PSTを学習し、MNPT重みファイルを生成する。"""

from __future__ import annotations

import argparse
import json
import math
import resource
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence

import numpy as np
import torch
from numpy.typing import NDArray
from torch import Tensor, nn
from torch.nn import functional as torch_functional

from minase_train.data.features import FEATURE_COUNT, PADDING_INDEX, feature_indices, mirror
from minase_train.data.lookahead import lookahead_options
from minase_train.data.mnpt import (
    float_weights_path,
    initial_piece_values,
    initial_weights,
    quantize,
    read_mnpt,
    write_mnpt,
)
from minase_train.data.mnsd import Dataset, ORIGINS, hash64, validate_lambda_override
from minase_train.data.taper import (
    BAND_COUNT,
    band_indices,
    band_label,
    phase_numerators,
    phase_ratios,
    piece_counts,
)
from minase_train.pst.evaluate import (
    QUANTIZATION_ERROR_LIMIT,
    float_evaluate,
    initial_position_score,
    integer_evaluate,
)
from minase_train.pst.model import (
    MODEL_KINDS,
    MirroredEmbedding,
    expanded_model_weights,
    make_model,
    model_logits,
)
from minase_train.pst.removal import REMOVAL_MARGIN_CP, make_removal_reference, removal_loss
from minase_train.pst.teacher import (
    _selected_indices,
    build_targets,
    estimate_generation_ks,
    estimate_mixed_k,
)


def validation_loss(
    model: nn.Module,
    dataset: Dataset,
    teacher_ks: NDArray[np.float64],
    k: float,
    batch: int,
    device: torch.device,
    *,
    indices: NDArray[np.int64],
    breakdown: dict | None = None,
) -> tuple[float, NDArray[np.float64]]:
    """明示した局面集合の純粋なBCEを計算し、全体値と世代別値を返す。"""
    indices = _selected_indices(dataset, indices)
    generation_sums = np.zeros(dataset.generation_count, dtype=np.float64)
    generation_counts = np.zeros(dataset.generation_count, dtype=np.int64)
    group_sums = np.zeros(3)
    group_counts = np.zeros(3, dtype=np.int64)
    origin_sums, origin_counts = np.zeros(3), np.zeros(3, dtype=np.int64)
    band_sums, band_counts = np.zeros(BAND_COUNT), np.zeros(BAND_COUNT, dtype=np.int64)
    sign_matches, decisive_counts = np.zeros(3, dtype=np.int64), np.zeros(3, dtype=np.int64)
    class_origins = np.array([ORIGINS.index(c.origin) for c in dataset.teacher_classes])
    games = {}
    with torch.no_grad():
        for start in range(0, indices.size, batch):
            global_indices = indices[start : start + batch]
            records = dataset.gather(global_indices)
            generations = dataset.generations(global_indices)
            features = feature_indices(
                records["board"], records["stm"], records["lion"]
            )
            targets = build_targets(records, teacher_ks, generations, dataset.teacher_lambdas,
                                    scores=dataset.teacher_scores(global_indices))
            device_features = torch.as_tensor(features, device=device)
            device_targets = torch.as_tensor(targets, device=device)
            device_phi = torch.as_tensor(
                phase_ratios(records["board"]).astype(np.float32), device=device
            )
            logits = model_logits(model, device_features, device_phi, k)
            losses = torch_functional.binary_cross_entropy_with_logits(
                logits,
                device_targets,
                reduction="none",
            ).cpu().numpy().astype(np.float64)
            generation_sums += np.bincount(
                generations, weights=losses, minlength=dataset.generation_count
            )
            generation_counts += np.bincount(
                generations, minlength=dataset.generation_count
            )
            if breakdown is not None:
                origins = class_origins[generations]
                bands = band_indices(phase_ratios(records["board"]))
                origin_sums += np.bincount(origins, weights=losses, minlength=3)
                origin_counts += np.bincount(origins, minlength=3)
                band_sums += np.bincount(bands, weights=losses, minlength=BAND_COUNT)
                band_counts += np.bincount(bands, minlength=BAND_COUNT)
                decisive = records["result"] != 1
                signs = np.sign(logits.cpu().numpy())
                matches = signs == (records["result"].astype(np.int16) - 1)
                sign_matches += np.bincount(origins[decisive & matches], minlength=3)
                decisive_counts += np.bincount(origins[decisive], minlength=3)
                human = origins == ORIGINS.index("human-game")
                for key, loss in zip(dataset.game_keys(global_indices[human]), losses[human]):
                    total, count = games.get(key, (0.0, 0))
                    games[key] = (total + float(loss), count + 1)
                files = np.searchsorted(dataset.offsets[1:], global_indices, side="right")
                game_half = np.zeros(len(records), dtype=np.bool_)
                for file in np.unique(files):
                    selected = files == file
                    game_half[selected] = hash64(dataset.headers[file].seed, records["game"][selected]) % np.uint64(40) == 0
                board = records["board"].astype(np.int16)
                kind = (board % 64 - 1) % 29
                royal = (board != 0) & ((kind == 11) | (kind == 21))
                two = ((np.count_nonzero(royal & (board < 65), axis=1) == 2)
                       | (np.count_nonzero(royal & (board >= 65), axis=1) == 2))
                for group, mask in enumerate((game_half, two, ~two)):
                    group_sums[group] += losses[mask].sum()
                    group_counts[group] += np.count_nonzero(mask)
    generation_losses = np.divide(
        generation_sums,
        generation_counts,
        out=np.full(dataset.generation_count, np.nan, dtype=np.float64),
        where=generation_counts != 0,
    )
    if breakdown is not None:
        def summaries(names, counts, sums):
            return {name: {"positions": int(count), "loss": float(total / count) if count else None}
                    for name, count, total in zip(names, counts, sums)}
        breakdown["origins"] = summaries(ORIGINS, origin_counts, origin_sums)
        breakdown["piece_bands"] = summaries([band_label(i) for i in range(BAND_COUNT)], band_counts, band_sums)
        breakdown["sign_agreement"] = {
            name: {"positions": int(count), "agreement": float(matches / count) if count else None}
            for name, count, matches in zip(ORIGINS, decisive_counts, sign_matches)
        }
        breakdown["human_game_mean"] = {
            "games": len(games), "positions": sum(count for _, count in games.values()),
            "loss": float(np.mean([total / count for total, count in games.values()])) if games else None,
        }
        breakdown.update({
            name: {"positions": int(count), "loss": float(total / count) if count else None}
            for name, count, total in zip(
                ("game_half", "two_royals", "other_royals"), group_counts, group_sums
            )
        })
    return (
        float(generation_sums.sum() / generation_counts.sum()),
        generation_losses,
    )


@dataclass(frozen=True)
class TrainEpochResult:
    """同じ更新前バッチで測った純BCE、係数適用前の除去損失、総損失を保持する。"""

    bce_loss: float
    removal_loss: float
    total_loss: float
    observations: NDArray[np.int64] | None


def train_epoch(
    model: nn.Module,
    optimizer: torch.optim.Optimizer,
    dataset: Dataset,
    teacher_ks: NDArray[np.float64],
    k: float,
    batch: int,
    generator: torch.Generator,
    device: torch.device,
    count_features: bool = False,
    *,
    indices: NDArray[np.int64],
    removal_penalty: float,
    removal_reference: Tensor | None,
) -> TrainEpochResult:
    """訓練レコードを1エポック学習し、重み共有しないモデルには鏡映拡張を施す。"""
    indices = _selected_indices(dataset, indices)
    if not math.isfinite(removal_penalty) or removal_penalty < 0.0:
        raise ValueError("removal penalty must be finite and nonnegative")
    if removal_penalty > 0.0:
        if not isinstance(model, MirroredEmbedding):
            raise ValueError("positive removal penalty requires model mirrored")
        if removal_reference is None:
            raise ValueError("positive removal penalty requires a reference")
    elif removal_reference is not None:
        raise ValueError("zero removal penalty requires reference None")
    order = torch.randperm(indices.size, generator=generator, device=device)
    bce_total = removal_total = total = 0.0
    observations = (
        np.zeros(FEATURE_COUNT, dtype=np.int64) if count_features else None
    )
    for start in range(0, order.shape[0], batch):
        positions = order[start : start + batch]
        global_indices = indices[positions.cpu().numpy()]
        records = dataset.gather(global_indices)
        generations = dataset.generations(global_indices)
        normal = feature_indices(records["board"], records["stm"], records["lion"])
        if observations is not None:
            active = normal[normal != PADDING_INDEX]
            observations += np.bincount(active, minlength=FEATURE_COUNT)
        selected = normal
        if not isinstance(model, MirroredEmbedding):
            mirrored_board, mirrored_lion = mirror(records["board"], records["lion"])
            reflected = feature_indices(mirrored_board, records["stm"], mirrored_lion)
            choose_reflected = torch.rand(
                positions.shape[0], generator=generator, device=device
            ) < 0.5
            selected = normal.copy()
            reflected_rows = choose_reflected.cpu().numpy()
            selected[reflected_rows] = reflected[reflected_rows]
        targets = build_targets(records, teacher_ks, generations, dataset.teacher_lambdas,
                                scores=dataset.teacher_scores(global_indices))
        device_features = torch.as_tensor(selected, device=device)
        device_targets = torch.as_tensor(targets, device=device)
        # 鏡映は駒数を変えないので、補間係数は鏡映前の盤面から計算してよい。
        device_phi = torch.as_tensor(
            phase_ratios(records["board"]).astype(np.float32), device=device
        )
        optimizer.zero_grad(set_to_none=True)
        bce = torch_functional.binary_cross_entropy_with_logits(
            model_logits(model, device_features, device_phi, k), device_targets
        )
        if removal_penalty > 0.0:
            device_counts = torch.as_tensor(piece_counts(records["board"]), device=device)
            penalty = removal_loss(
                model(device_features), device_features, removal_reference, device_counts, k
            )
            loss = bce + removal_penalty * penalty
        else:
            penalty = bce.new_zeros(())
            loss = bce
        loss.backward()
        optimizer.step()
        # MNPTの1/8センチポーン単位のi16に収まる範囲で学習する。
        with torch.no_grad():
            if any(not bool(torch.isfinite(parameter).all()) for parameter in model.parameters()):
                raise ValueError("trained weights contain a non-finite value")
            for parameter in model.parameters():
                parameter.clamp_(min=-4096.0, max=4095.875)
        bce_total += float(bce.item()) * positions.shape[0]
        removal_total += float(penalty.item()) * positions.shape[0]
        total += float(loss.item()) * positions.shape[0]
    return TrainEpochResult(
        bce_total / indices.size, removal_total / indices.size, total / indices.size, observations
    )


def command_init(arguments: argparse.Namespace) -> None:
    """initサブコマンドを実行する。両端点にv0駒価値を置き、探索用駒価値もv0に固定する。"""
    weights = initial_weights()
    write_mnpt(arguments.output, weights, weights, initial_piece_values(), arguments.k)
    print(f"initial position evaluation: {initial_position_score(weights, weights)} cp")


def command_estimate_k(arguments: argparse.Namespace) -> None:
    """estimate-kサブコマンドを実行する。"""
    dataset = Dataset(arguments.data, rescore=arguments.rescore,
                      lambda_override=arguments.lambda_override,
                      lookahead=lookahead_options(arguments.lookahead_gamma, arguments.lookahead_plies))
    generation_ks, generation_counts = estimate_generation_ks(dataset, indices=dataset.training_indices)
    for generation, (checksum, k, count) in enumerate(
        zip(dataset.generation_checksums, generation_ks, generation_counts)
    ):
        file_count = sum(np.any(rows == generation) for rows in dataset.row_generations)
        print(
            f"generation {generation}: files={file_count} checksum={checksum.hex()} "
            f"training_records={count} K={k:.9f}"
        )
    mixed_k = estimate_mixed_k(dataset, indices=dataset.training_indices)
    print(f"mixed: training_records={dataset.training_indices.size} K={mixed_k}")


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


def should_replace_best_epoch(candidate_loss: float, best_loss: float) -> bool:
    """検証損失が厳密に改善した場合だけ最良エポックを更新する。"""
    return candidate_loss < best_loss


def command_train(arguments: argparse.Namespace) -> None:
    """trainサブコマンドを実行する。"""
    validate_lambda_override(arguments.lambda_override)
    if arguments.epochs <= 0 or arguments.batch <= 0 or arguments.validation_sample <= 0:
        raise ValueError("--epochs, --batch, and --validation-sample must be positive")
    if any(rate <= 0.0 or not math.isfinite(rate) for rate in arguments.lr):
        raise ValueError("every --lr value must be finite and positive")
    if arguments.k <= 0.0 or not math.isfinite(arguments.k):
        raise ValueError("--k must be finite and positive")
    if not math.isfinite(arguments.removal_penalty) or arguments.removal_penalty < 0.0:
        raise ValueError("--removal-penalty must be finite and nonnegative")
    if arguments.removal_penalty > 0.0 and arguments.model != "mirrored":
        raise ValueError("positive --removal-penalty requires --model mirrored")

    torch.manual_seed(arguments.seed)
    if torch.cuda.is_available():
        torch.cuda.manual_seed_all(arguments.seed)
    device = torch.device(arguments.device)
    if device.type == "cuda":
        torch.cuda.reset_peak_memory_stats(device)
    dataset = Dataset(arguments.data, rescore=arguments.rescore,
                      lambda_override=arguments.lambda_override,
                      lookahead=lookahead_options(arguments.lookahead_gamma, arguments.lookahead_plies))
    if dataset.training_indices.size == 0 or dataset.validation_indices.size == 0:
        raise ValueError("game split produced an empty training or validation set")

    training_halves = dataset.training_halves()
    training_half_counts = [int(indices.size) for indices in training_halves]
    training_indices = (dataset.training_indices if arguments.train_half is None
                        else training_halves[arguments.train_half])
    if training_indices.size == 0:
        raise ValueError("--train-half selected an empty training set")
    # 教師Kと先読み教師は、選択した半分にかかわらず全体から計算する。
    teacher_ks, _ = estimate_generation_ks(dataset, indices=dataset.training_indices)
    teacher_k_log = ", ".join(
        f"generation {generation} = {k:.9f}"
        for generation, k in enumerate(teacher_ks)
    )
    print(f"teacher K: {teacher_k_log}")
    print(f"rescore exclusions: {json.dumps(dataset.exclusions)}")
    print(
        f"records: total={dataset.record_count} "
        f"training={training_indices.size} "
        f"validation={dataset.validation_indices.size}"
    )

    initial_middlegame, initial_endgame, piece_values, _ = read_mnpt(arguments.init)
    removal_reference = (
        make_removal_reference(initial_middlegame, initial_endgame, device)
        if arguments.removal_penalty > 0.0 else None
    )
    if arguments.model == "single":
        # 単一PSTは1組のパラメータを学習する。初期値の両端点は一致していなければならない。
        if not np.array_equal(initial_middlegame, initial_endgame):
            raise ValueError("--model single requires an initial MNPT whose endpoints are identical")
        columns = (initial_middlegame,)
    else:
        columns = (initial_middlegame, initial_endgame)
    initial = torch.as_tensor(
        np.stack(columns, axis=1).astype(np.float32) / 8.0, device=device
    )
    print(f"model: {arguments.model} columns={initial.shape[1]}")
    print(f"removal penalty: coefficient={arguments.removal_penalty:g} margin_cp={REMOVAL_MARGIN_CP:g}")

    selected_rate = arguments.lr[0]
    updates_per_epoch = (training_indices.size + arguments.batch - 1) // arguments.batch
    total_updates = 0
    if len(arguments.lr) > 1:
        candidates: list[tuple[float, float]] = []
        for rate in arguments.lr:
            model = make_model(initial, device, arguments.model)
            optimizer = torch.optim.Adam(model.parameters(), lr=rate)
            generator = torch.Generator(device=device).manual_seed(arguments.seed)
            training = train_epoch(
                model,
                optimizer,
                dataset,
                teacher_ks,
                arguments.k,
                arguments.batch,
                generator,
                device,
                indices=training_indices,
                removal_penalty=arguments.removal_penalty,
                removal_reference=removal_reference,
            )
            # 学習率候補の試行で実行した更新も総更新回数に含める。
            total_updates += updates_per_epoch
            loss, _ = validation_loss(
                model,
                dataset,
                teacher_ks,
                arguments.k,
                arguments.batch,
                device,
                indices=dataset.validation_indices,
            )
            candidates.append((loss, rate))
            print(
                f"learning-rate candidate: lr={rate:g} train_loss={training.bce_loss:.9f} "
                f"removal_loss={training.removal_loss:.9f} total_loss={training.total_loss:.9f} "
                f"validation_loss={loss:.9f}"
            )
        selected_rate = min(candidates)[1]
    print(f"selected learning rate: {selected_rate:g}")

    model = make_model(initial, device, arguments.model)
    optimizer = torch.optim.Adam(model.parameters(), lr=selected_rate)
    generator = torch.Generator(device=device).manual_seed(arguments.seed)
    breakdown = {}
    history = []
    best_loss, generation_losses = validation_loss(
        model,
        dataset,
        teacher_ks,
        arguments.k,
        arguments.batch,
        device,
        indices=dataset.validation_indices, breakdown=breakdown,
    )
    best_epoch = 0
    best_weights = expanded_model_weights(model).detach().cpu().clone()

    def log_validation(epoch: int, loss: float, generations: np.ndarray, groups: dict) -> None:
        entry = {"epoch": epoch, "loss": loss,
                 "generations": [float(v) if np.isfinite(v) else None for v in generations],
                 **groups}
        history.append(entry)
        print(f"epoch {epoch}: {_format_validation_loss(loss, generations)}")
        print(f"epoch {epoch}: validation_groups={json.dumps(groups, allow_nan=False)}")

    log_validation(0, best_loss, generation_losses, breakdown)
    for epoch in range(1, arguments.epochs + 1):
        started = time.perf_counter()
        training = train_epoch(
            model,
            optimizer,
            dataset,
            teacher_ks,
            arguments.k,
            arguments.batch,
            generator,
            device,
            count_features=epoch == 1,
            indices=training_indices,
            removal_penalty=arguments.removal_penalty,
            removal_reference=removal_reference,
        )
        total_updates += updates_per_epoch
        elapsed = time.perf_counter() - started
        positions_per_second = training_indices.size / elapsed
        breakdown = {}
        loss, generation_losses = validation_loss(
            model,
            dataset,
            teacher_ks,
            arguments.k,
            arguments.batch,
            device,
            indices=dataset.validation_indices, breakdown=breakdown,
        )
        print(
            f"epoch {epoch}: train_loss={training.bce_loss:.9f} "
            f"removal_loss={training.removal_loss:.9f} total_loss={training.total_loss:.9f} "
            f"positions_per_second={positions_per_second:.3f}"
        )
        log_validation(epoch, loss, generation_losses, breakdown)
        if training.observations is not None:
            _print_feature_observations(training.observations)
        if should_replace_best_epoch(loss, best_loss):
            best_loss = loss
            best_epoch = epoch
            best_weights = expanded_model_weights(model).detach().cpu().clone()

    print(f"best epoch: {best_epoch} validation_loss={best_loss:.9f}")

    float_columns = best_weights.numpy().astype(np.float32)
    if float_columns.shape[1] == 1:
        # 単一PSTは出力時に両端点へ複製する。
        float_columns = np.repeat(float_columns, 2, axis=1)
    float_middlegame = np.ascontiguousarray(float_columns[:, 0])
    float_endgame = np.ascontiguousarray(float_columns[:, 1])
    middlegame = quantize(float_middlegame)
    endgame = quantize(float_endgame)

    validation_count = dataset.validation_indices.size
    sample_count = min(arguments.validation_sample, validation_count)
    random = np.random.default_rng(arguments.seed)
    sample_indices = random.choice(validation_count, size=sample_count, replace=False)
    selected_indices = dataset.validation_indices[sample_indices]
    sample_records = dataset.gather(selected_indices)
    sample_features = feature_indices(
        sample_records["board"], sample_records["stm"], sample_records["lion"]
    )
    floating_scores = float_evaluate(
        float_middlegame, float_endgame, sample_features, phase_ratios(sample_records["board"])
    )
    integer_scores = integer_evaluate(
        middlegame, endgame, sample_features, phase_numerators(sample_records["board"])
    )
    errors = np.abs(floating_scores - integer_scores.astype(np.float32))
    mean_absolute_error = float(errors.mean())
    print(f"quantization error: samples={sample_count} mean_absolute={mean_absolute_error:.9f} max={float(errors.max()):.9f} cp")
    if not math.isfinite(mean_absolute_error) or mean_absolute_error > QUANTIZATION_ERROR_LIMIT:
        raise ValueError(
            f"quantization mean absolute error {mean_absolute_error} exceeds {QUANTIZATION_ERROR_LIMIT} cp"
        )
    write_mnpt(arguments.output, middlegame, endgame, piece_values, arguments.k)
    # 診断が共通標本で量子化誤差を測れるよう、量子化前の重みも保存する。
    with float_weights_path(arguments.output).open("xb") as stream:
        np.savez(stream, middlegame=float_middlegame, endgame=float_endgame)
    print(f"initial position evaluation: {initial_position_score(middlegame, endgame)} cp")
    with Path(arguments.output).with_suffix(".training.json").open("x") as stream:
        json.dump({"validation": history, "best_epoch": best_epoch,
                   "teacher_classes": dataset.class_metadata(),
                   "lookahead": dataset.lookahead,
                   "lambda_override": dataset.lambda_override,
                   "train_half": arguments.train_half,
                   "training_half_counts": training_half_counts,
                   "total_updates": total_updates,
                   "teacher_ks": [float(k) if np.isfinite(k) else None for k in teacher_ks],
                   "rescore_exclusions": dataset.exclusions}, stream, indent=2, allow_nan=False)
        stream.write("\n")
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
    estimate_parser.add_argument("--rescore", nargs="+")
    estimate_parser.add_argument("--lambda-override", type=float)
    estimate_parser.add_argument("--lookahead-gamma", type=float)
    estimate_parser.add_argument("--lookahead-plies", type=int)
    estimate_parser.set_defaults(handler=command_estimate_k)

    train_parser = commands.add_parser("train", help="学習PSTを訓練する")
    train_parser.add_argument("--data", required=True, nargs="+")
    train_parser.add_argument("--train-half", type=int, choices=(0, 1),
                              help="対局単位で2分割した訓練データの一方だけを使う")
    train_parser.add_argument("--rescore", nargs="+")
    train_parser.add_argument("--lambda-override", type=float)
    train_parser.add_argument("--lookahead-gamma", type=float)
    train_parser.add_argument("--lookahead-plies", type=int)
    train_parser.add_argument("--output", required=True)
    train_parser.add_argument("--init", required=True)
    train_parser.add_argument("--model", required=True, choices=MODEL_KINDS)
    train_parser.add_argument("--k", required=True, type=float)
    train_parser.add_argument("--removal-penalty", required=True, type=float)
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
