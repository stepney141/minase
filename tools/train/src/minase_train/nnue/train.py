"""線形PSTと鏡映共有NNUEを学習し、検査済みのMNUE版4へ保存する。"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import time
from typing import Sequence

import numpy as np
from numpy.typing import NDArray
import torch
from torch import Tensor
from torch.nn import functional as F

from minase_train.checksum import sha256_file
from minase_train.data import mnpt
from minase_train.data.features import FEATURE_COUNT, canonical_feature_indices, feature_indices
from minase_train.data.lookahead import lookahead_options
from minase_train.data.mnsd import Dataset, provenance_path, validate_lambda_override
from minase_train.data.taper import phase_ratios, piece_counts
from minase_train.fm.train import feature_observations
from minase_train.nnue.format import (
    FIRST_LAYER_LIMIT, OUTPUT_BIAS_LIMIT, OUTPUT_WEIGHT_LIMIT, Network,
    read_mnue, validate_width, write_mnue,
)
from minase_train.nnue.model import NNUE, float_evaluate, load_initial
from minase_train.nnue.reference import REFERENCE_BATCH, integer_evaluate
from minase_train.pst.model import expanded_model_weights
from minase_train.pst.removal import make_removal_reference, removal_loss
from minase_train.pst.teacher import build_targets, estimate_generation_ks
from minase_train.pst.train import should_replace_best_epoch, should_stop_early


GRADIENT_STEPS = 5


class _TimeLimitReached(Exception):
    """検証を含む学習時間が尽きたことを学習ループへ伝える。"""


def _check_time(deadline: float | None) -> None:
    if deadline is not None and time.monotonic() >= deadline:
        raise _TimeLimitReached


def training_report_path(output: str | Path) -> Path:
    """既存の学習器と同じ命名でエポック記録を併置する。"""
    return Path(output).with_suffix(".training.json")


def _write_report(path: Path, report: dict) -> None:
    path.write_text(json.dumps(report, indent=2, ensure_ascii=False, allow_nan=False) + "\n")


def _file_record(path: str | Path) -> dict:
    return {"path": str(Path(path).resolve()), "sha256": sha256_file(Path(path)).hex()}


def _batch_inputs(records: NDArray, device: torch.device) -> tuple[Tensor, Tensor, Tensor]:
    features = [torch.as_tensor(feature_indices(records["board"], side, records["lion"]), device=device)
                for side in (records["stm"], 1 - records["stm"])]
    ratios = torch.as_tensor(phase_ratios(records["board"]), dtype=torch.float32, device=device)
    return features[0], features[1], ratios


def validation_loss(model: NNUE, dataset: Dataset, teacher_ks: NDArray, batch: int,
                    deadline: float | None) -> tuple[float, dict]:
    """検証集合全体のBCE、非線形出力の標準偏差、および飽和率を集計する。"""
    total = nonlinear_sum = nonlinear_square_sum = 0.0
    lower = upper = units = 0
    device = model.output_bias.device
    with torch.no_grad():
        for start in range(0, dataset.validation_indices.size, batch):
            _check_time(deadline)
            indices = dataset.validation_indices[start:start + batch]
            records = dataset.gather(indices)
            linear, nonlinear, hidden = model.components(*_batch_inputs(records, device))
            targets = build_targets(records, teacher_ks, dataset.generations(indices), dataset.teacher_lambdas,
                                    scores=dataset.teacher_scores(indices))
            total += float(F.binary_cross_entropy_with_logits(
                linear / model.k + nonlinear, torch.as_tensor(targets, device=device),
                reduction="none").sum(dtype=torch.float64))
            values = nonlinear.cpu().numpy().astype(np.float64) * model.k
            nonlinear_sum += float(values.sum())
            nonlinear_square_sum += float(np.square(values).sum())
            lower += int((hidden <= 0).sum())
            upper += int((hidden >= 1).sum())
            units += hidden.numel()
            _check_time(deadline)
    count = dataset.validation_indices.size
    if not all(math.isfinite(value) for value in (total, nonlinear_sum, nonlinear_square_sum)):
        raise ValueError("non-finite validation loss or NNUE output")
    mean = nonlinear_sum / count
    return total / count, {
        "nonlinear_std_cp": math.sqrt(max(0, nonlinear_square_sum / count - mean * mean)),
        "saturation": {"acc_le_zero": lower / units, "acc_ge_one": upper / units},
        "samples": int(count),
    }


def _linear_movement(model: NNUE, initial: NDArray) -> dict:
    current = expanded_model_weights(model.linear).detach().cpu().numpy().astype(np.float64)
    difference = np.abs(current - initial)
    return {name: {"mean_absolute_cp": float(difference[:, i].mean()),
                   "max_absolute_cp": float(difference[:, i].max())}
            for i, name in enumerate(("middlegame", "endgame"))}


def _record_gradients(optimizer: torch.optim.Optimizer, report: dict) -> None:
    norms = {}
    for name, group in zip(("linear", "nonlinear"), optimizer.param_groups):
        norms[name] = math.sqrt(sum(float(parameter.grad.norm())**2
                                    for parameter in group["params"] if parameter.grad is not None))
    if not all(math.isfinite(value) for value in norms.values()):
        raise ValueError("non-finite loss gradient")
    if report["total_steps"] < GRADIENT_STEPS:
        report["gradients"].append({"step": report["total_steps"] + 1, **norms})


def train_epoch(model: NNUE, dataset: Dataset, teacher_ks: NDArray, reference: Tensor,
                optimizer: torch.optim.Optimizer, generator: torch.Generator,
                args: argparse.Namespace, report: dict, deadline: float) -> dict:
    """局面を並べ替え、除去差分を局面数で分割して1バッチ分の勾配を蓄積する。"""
    _check_time(deadline)
    order = torch.randperm(dataset.training_indices.size, generator=generator).numpy()
    totals = dict.fromkeys(("train_loss", "removal_loss", "total_loss"), 0.0)
    device = model.output_bias.device
    for start in range(0, len(order), args.batch):
        _check_time(deadline)
        indices = dataset.training_indices[order[start:start + args.batch]]
        optimizer.zero_grad(set_to_none=True)
        for offset in range(0, len(indices), args.removal_chunk):
            _check_time(deadline)
            chunk = indices[offset:offset + args.removal_chunk]
            records = dataset.gather(chunk)
            features, opposite, ratios = _batch_inputs(records, device)
            targets = build_targets(records, teacher_ks, dataset.generations(chunk), dataset.teacher_lambdas,
                                    scores=dataset.teacher_scores(chunk))
            bce = F.binary_cross_entropy_with_logits(model(features, opposite, ratios),
                                                     torch.as_tensor(targets, device=device))
            if args.removal_penalty > 0:
                counts = torch.as_tensor(piece_counts(records["board"]), device=device)
                penalty = removal_loss(model.removal_deltas(features, opposite, counts),
                                       features, reference, counts, model.k)
            else:
                penalty = bce.new_zeros(())
            loss = bce + args.removal_penalty * penalty
            if not bool(torch.isfinite(loss)):
                raise ValueError("non-finite training loss")
            (loss * (len(chunk) / len(indices))).backward()
            for name, value in zip(totals, (bce, penalty, loss)):
                totals[name] += float(value.detach()) * len(chunk)
        _record_gradients(optimizer, report)
        _check_time(deadline)
        optimizer.step()
        model.constrain()
        report["total_steps"] += 1
        _check_time(deadline)
    return {name: value / len(order) for name, value in totals.items()}


def quantized_weights(weights: dict[str, NDArray]) -> tuple[NDArray, NDArray, NDArray, NDArray, int]:
    """指定された尺度と偶数丸めを使い、整数型へ変換する前に範囲を検査する。"""
    def rounded(name: str, scale: int, limit: int, dtype: str) -> NDArray:
        result = np.rint(np.asarray(weights[name], dtype=np.float64) * scale)
        if not np.isfinite(result).all() or np.any(np.abs(result) > limit):
            raise ValueError(f"NNUE quantization: {name} exceeds {limit}")
        return result.astype(dtype)

    linear = np.stack([mnpt.quantize(weights[name]) for name in ("middlegame", "endgame")], axis=1)
    return (linear, rounded("weights", 127, FIRST_LAYER_LIMIT, "<i2"),
            rounded("biases", 127, FIRST_LAYER_LIMIT, "<i4"),
            rounded("output_weights", 4096, OUTPUT_WEIGHT_LIMIT, "<i2"),
            int(rounded("output_bias", 127 * 4096, OUTPUT_BIAS_LIMIT, "<i4")))


def quantization_report(dataset: Dataset, weights: dict[str, NDArray], net: Network,
                        batch: int, sample_size: int, seed: int) -> dict:
    """固定シードの検証標本で、倍精度の実数評価と整数評価の誤差を測る。"""
    indices = np.random.default_rng(seed).choice(dataset.validation_indices,
        size=min(sample_size, dataset.validation_indices.size), replace=False)
    errors = []
    for start in range(0, len(indices), min(batch, REFERENCE_BATCH)):
        records = dataset.gather(indices[start:start + min(batch, REFERENCE_BATCH)])
        arguments = (records["board"], records["stm"], records["lion"])
        errors.append(np.abs(float_evaluate(weights, net.k, *arguments) - integer_evaluate(net, *arguments)))
    error = np.concatenate(errors)
    if not np.isfinite(error).all():
        raise ValueError("non-finite quantization error")
    return {"samples": len(indices), "seed": seed, "split": "validation",
            "indices_sha256": hashlib.sha256(indices.astype("<i8").tobytes()).hexdigest(),
            "mean_absolute_error_cp": float(error.mean()), "max_absolute_error_cp": float(error.max()),
            "limit_cp": net.k / 40}


def _fit(model: NNUE, dataset: Dataset, teacher_ks: NDArray, initial: NDArray,
         args: argparse.Namespace, report: dict, path: Path, started: float) -> None:
    device = model.output_bias.device
    optimizer = torch.optim.AdamW([
        {"params": [model.linear.weight], "lr": args.lr_linear, "weight_decay": 0},
        {"params": model.nonlinear_parameters(), "lr": args.lr_nonlinear,
         "weight_decay": args.weight_decay_nonlinear},
    ])
    generator = torch.Generator().manual_seed(args.seed)
    v0 = mnpt.initial_weights()
    reference = make_removal_reference(v0, v0, device)
    deadline = started + args.time_limit_hours * 3600
    best, best_loss, best_epoch = None, math.inf, 0
    try:
        loss, statistics = validation_loss(model, dataset, teacher_ks, args.batch, deadline)
        report["initial_validation_loss"] = loss
        report["epochs"].append({"epoch": 0, "train_loss": None, "removal_loss": None,
            "total_loss": None, "validation_loss": loss, "linear_movement": _linear_movement(model, initial),
            **statistics, "elapsed_seconds": time.monotonic() - started, "steps": 0, "total_steps": 0})
        _write_report(path, report)
        print(f"epoch 0: validation_loss={loss:.9f}", flush=True)
        for epoch in range(1, args.epochs + 1):
            training = train_epoch(model, dataset, teacher_ks, reference, optimizer, generator,
                                   args, report, deadline)
            loss, statistics = validation_loss(model, dataset, teacher_ks, args.batch, deadline)
            report["epochs"].append({"epoch": epoch, **training, "validation_loss": loss,
                "linear_movement": _linear_movement(model, initial), **statistics,
                "elapsed_seconds": time.monotonic() - started,
                "steps": report["steps_per_epoch"], "total_steps": report["total_steps"]})
            if should_replace_best_epoch(loss, best_loss):
                best_loss, best_epoch = loss, epoch
                best = {name: value.detach().cpu().clone() for name, value in model.state_dict().items()}
            report.update(best_epoch=best_epoch, best_validation_loss=best_loss, last_epoch=epoch)
            _write_report(path, report)
            print(f"epoch {epoch}: train_loss={training['train_loss']:.9f} "
                  f"removal_loss={training['removal_loss']:.9f} total_loss={training['total_loss']:.9f} "
                  f"validation_loss={loss:.9f}", flush=True)
            _check_time(deadline)
            if should_stop_early(epoch, best_epoch, args.patience):
                report["stopped_by"] = "early_stopping"
                break
    except _TimeLimitReached:
        report.update(stopped_by="time_limit", convergence="予算の上限で終了し、収束は未確認")
        if best is None:
            raise ValueError("time limit reached with 0 completed training epochs (0エポック); 収束は未確認") from None
    if args.epochs:
        if best is None:
            raise ValueError("no completed training epoch")
        model.load_state_dict(best)
        report["improved_over_epoch_zero"] = best_loss < report["initial_validation_loss"]


def command_train(args: argparse.Namespace) -> None:
    """入力と来歴を記録し、最良の学習済みエポックを量子化して保存する。"""
    started = time.monotonic()
    output = Path(args.output)
    report_path = training_report_path(output)
    float_path = mnpt.float_weights_path(output)
    for path in (output, report_path, float_path):
        if path.exists():
            raise ValueError(f"{path}: training output already exists")
    if len({output, report_path, float_path}) != 3:
        raise ValueError(f"{output}: output, report and float paths must be distinct")
    output.parent.mkdir(parents=True, exist_ok=True)
    report = {"status": "training", "stopped_by": "max_epochs", "epochs": [], "gradients": [],
              "total_steps": 0, "last_epoch": 0, "best_epoch": None, "best_validation_loss": None}
    try:
        validate_width(args.width, str(output))
        if args.epochs < 0 or args.seed < 0:
            raise ValueError(f"{output}: epochs and seed must be nonnegative")
        for name in ("patience", "batch", "removal_chunk", "validation_sample"):
            if getattr(args, name) <= 0:
                raise ValueError(f"{output}: --{name.replace('_', '-')} must be positive")
        for name in ("lr_linear", "lr_nonlinear", "time_limit_hours", "weight_decay_nonlinear", "removal_penalty"):
            value = getattr(args, name)
            if not math.isfinite(value) or value < 0 or (name in ("lr_linear", "lr_nonlinear", "time_limit_hours") and value == 0):
                raise ValueError(f"{output}: invalid --{name.replace('_', '-')}")
        validate_lambda_override(args.lambda_override)
        device = torch.device(args.device)
        if device.type == "cuda" and not torch.cuda.is_available():
            raise ValueError(f"{output}: CUDA unavailable; run training on a GPU host")
        _, _, piece_values, k = mnpt.read_mnpt(args.base)
        initial = load_initial(args.init_pst)
        dataset = Dataset(args.data, lambda_override=args.lambda_override,
                          lookahead=lookahead_options(args.lookahead_gamma, args.lookahead_plies))
        if not dataset.training_indices.size or not dataset.validation_indices.size:
            raise ValueError(f"{output}: training and validation sets must both be nonempty")
        teacher_ks, teacher_counts = estimate_generation_ks(dataset, indices=dataset.training_indices)
        counts = feature_observations(dataset, args.batch)
        model = NNUE(initial, counts > 0, args.width, k, device, args.seed)
        steps = (dataset.training_indices.size + args.batch - 1) // args.batch
        report.update(
            inputs=[dict(_file_record(path), provenance_sha256=sha256_file(provenance_path(Path(path))).hex())
                    for path in args.data], init_pst=_file_record(args.init_pst), base=_file_record(args.base),
            width=args.width, k=k, split={
                "method": "Dataset game hash modulo 20; remainder 0 is validation",
                "training_positions": int(dataset.training_indices.size),
                "validation_positions": int(dataset.validation_indices.size),
                "training_indices_sha256": hashlib.sha256(dataset.training_indices.astype("<i8").tobytes()).hexdigest(),
                "validation_indices_sha256": hashlib.sha256(dataset.validation_indices.astype("<i8").tobytes()).hexdigest(),
            }, teacher_classes=dataset.class_metadata(), teacher_training_counts=teacher_counts,
            teacher_ks=[float(value) if np.isfinite(value) else None for value in teacher_ks],
            teacher_k_source="estimated from training set", lambda_override=args.lambda_override,
            lookahead=dataset.lookahead, feature_observations=counts.tolist(),
            unobserved_features=int(np.count_nonzero(counts == 0)), optimizer="AdamW",
            lr_linear=args.lr_linear, lr_nonlinear=args.lr_nonlinear, weight_decay_linear=0,
            weight_decay_nonlinear=args.weight_decay_nonlinear, removal_penalty=args.removal_penalty,
            removal_chunk=args.removal_chunk, removal_reference="v0 initial_weights at both endpoints",
            batch=args.batch, seed=args.seed, device=args.device, max_epochs=args.epochs, patience=args.patience,
            time_limit_hours=args.time_limit_hours, validation_sample=args.validation_sample,
            steps_per_epoch=int(steps), planned_steps=int(steps * args.epochs), gradient_steps=GRADIENT_STEPS,
            packages={"numpy": np.__version__, "torch": torch.__version__},
        )
        _fit(model, dataset, teacher_ks, initial, args, report, report_path, started)
        floating = model.floating_weights()
        with float_path.open("xb") as stream:
            np.savez(stream, **floating)
        linear, weights, biases, output_weights, output_bias = quantized_weights(floating)
        write_mnue(output, linear, piece_values, k, weights, biases, output_weights, output_bias)
        net = read_mnue(output)
        report["output_sha256"] = sha256_file(output).hex()
        report["float_sha256"] = sha256_file(float_path).hex()
        report["quantization"] = quantization_report(dataset, floating, net, args.batch,
                                                     args.validation_sample, args.seed)
        report["status"] = ("quantization_error" if report["quantization"]["mean_absolute_error_cp"] > k / 40
                            else "candidate")
        if report["status"] == "quantization_error":
            report["reason"] = "quantization mean absolute error exceeds K/40"
        report["elapsed_seconds"] = time.monotonic() - started
        _write_report(report_path, report)
        print(f"best epoch: {report['best_epoch']}; stopped_by: {report['stopped_by']}; "
              f"status: {report['status']}", flush=True)
    except Exception as error:
        report.update(status="error", reason=str(error), elapsed_seconds=time.monotonic() - started)
        _write_report(report_path, report)
        raise
    if report["status"] == "quantization_error":
        raise SystemExit(1)


def command_random(args: argparse.Namespace) -> None:
    """Rust照合用に、鏡映共有と保存範囲を満たす一様整数のネットを生成する。"""
    validate_width(args.width, str(args.output))
    output = Path(args.output)
    if output.exists():
        raise ValueError(f"{output}: random output already exists")
    _, _, values, k = mnpt.read_mnpt(args.base)
    rng = np.random.default_rng(args.seed)
    canonical = canonical_feature_indices(np.arange(FEATURE_COUNT))
    linear = rng.integers(-4000, 4001, size=(FEATURE_COUNT // 2, 2), dtype=np.int16)[canonical]
    weights = rng.integers(-1016, 1017, size=(FEATURE_COUNT // 2, args.width), dtype=np.int16)[canonical]
    reflected = np.arange(FEATURE_COUNT) % 12 >= 6
    weights[reflected] = np.roll(weights[reflected], args.width // 2, axis=1)
    biases = np.tile(rng.integers(-1016, 1017, size=args.width // 2, dtype=np.int32), 2)
    output_weights = np.tile(rng.integers(-8192, 8193, size=(2, args.width // 2), dtype=np.int16), (1, 2))
    output_bias = int(rng.integers(-100_000, 100_001))
    output.parent.mkdir(parents=True, exist_ok=True)
    write_mnue(output, linear, values, k, weights, biases, output_weights, output_bias)


def build_parser() -> argparse.ArgumentParser:
    """学習と照合用ランダムネット生成のCLIを作る。"""
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    train = commands.add_parser("train")
    train.add_argument("--data", nargs="+", required=True, type=Path)
    train.add_argument("--init-pst", required=True, type=Path)
    train.add_argument("--base", required=True, type=Path)
    train.add_argument("--width", required=True, type=int)
    train.add_argument("--output", required=True, type=Path)
    for name, default in (("epochs", 50), ("patience", 5), ("batch", 16384), ("seed", 1),
                          ("removal-chunk", 2048), ("validation-sample", 10000)):
        train.add_argument("--" + name, type=int, default=default)
    for name, default in (("lr-linear", 0.03), ("lr-nonlinear", 1e-3), ("weight-decay-nonlinear", 1e-4),
                          ("removal-penalty", 1000), ("time-limit-hours", 24), ("lambda-override", 1.0)):
        train.add_argument("--" + name, type=float, default=default)
    train.add_argument("--lookahead-gamma", type=float)
    train.add_argument("--lookahead-plies", type=int)
    train.add_argument("--device", choices=("cpu", "cuda"), required=True)
    train.set_defaults(handler=command_train)
    random = commands.add_parser("random")
    for name in ("width", "seed"):
        random.add_argument("--" + name, required=True, type=int)
    for name in ("base", "output"):
        random.add_argument("--" + name, required=True, type=Path)
    random.set_defaults(handler=command_random)
    return parser


def main(arguments: Sequence[str] | None = None) -> None:
    """指定されたサブコマンドを実行し、不合格候補は非0で終了する。"""
    args = build_parser().parse_args(arguments)
    args.handler(args)


if __name__ == "__main__":
    main()
