"""固定した2端点PSTへ加えるFactorization Machineを学習し、MNPT v3へ保存する。"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
from typing import Sequence

import numpy as np
from numpy.typing import NDArray
import torch
from torch import Tensor, nn
from torch.nn import functional as F

from minase_train.checksum import sha256_file
from minase_train.data.features import FEATURE_COUNT, PADDING_INDEX, feature_indices, mirror
from minase_train.data.lookahead import lookahead_options
from minase_train.data.mnsd import Dataset, provenance_path, validate_lambda_override
from minase_train.data.taper import phase_numerators
from minase_train.data.mnpt import (
    EVALUATION_LIMIT, float_weights_path, read_mnpt, read_mnpt_v3,
    write_mnpt_v3, validate_fixed_base,
)
from minase_train.pst.evaluate import QUANTIZATION_ERROR_LIMIT, integer_evaluate as pst_evaluate
from minase_train.pst.teacher import build_targets, estimate_generation_ks
from minase_train.pst.train import should_replace_best_epoch, should_stop_early

GRADIENT_STEPS = 5
# 診断の大きな局面バッチでも、特徴×次元の一時配列を一定サイズに抑える。
REFERENCE_BATCH = 1024


def fm_phi(v: NDArray, a: NDArray, features: NDArray) -> NDArray[np.float64]:
    """二値特徴の恒等式で、量子化前の勝率ロジット補正を計算する。"""
    extended = np.zeros((FEATURE_COUNT + 1, len(a)), dtype=np.float64)
    extended[:FEATURE_COUNT] = v
    output = np.empty(len(features), dtype=np.float64)
    for start in range(0, len(features), REFERENCE_BATCH):
        selected = extended[features[start:start + REFERENCE_BATCH]]
        pairs = selected.sum(axis=1)**2 - (selected * selected).sum(axis=1)
        output[start:start + REFERENCE_BATCH] = 0.5 * (pairs * a).sum(axis=1)
    return output


def float_evaluate(
    middlegame: NDArray, endgame: NDArray, k: float, v: NDArray, a: NDArray,
    features: NDArray, numerators: NDArray,
) -> NDArray[np.float64]:
    """整数PSTにK×Phiを加える。最終評価の切り詰めは行わない。"""
    return pst_evaluate(middlegame, endgame, features, numerators).astype(np.float64) + k * fm_phi(v, a, features)


def fm_accumulators(u: NDArray, signs: NDArray, features: NDArray) -> tuple[NDArray, NDArray]:
    """量子化表から64ビットでAとCを全再計算する。paddingの自己項は常に0とする。"""
    extended = np.zeros((FEATURE_COUNT + 1, len(signs)), dtype=np.int64)
    extended[:FEATURE_COUNT] = u
    self_terms = (extended * extended * signs).sum(axis=1, dtype=np.int64)
    accumulators = np.empty((len(features), len(signs)), dtype=np.int64)
    c = np.empty(len(features), dtype=np.int64)
    for start in range(0, len(features), REFERENCE_BATCH):
        chunk = features[start:start + REFERENCE_BATCH]
        accumulators[start:start + REFERENCE_BATCH] = extended[chunk].sum(axis=1, dtype=np.int64)
        c[start:start + REFERENCE_BATCH] = self_terms[chunk].sum(axis=1, dtype=np.int64)
    return accumulators, c


def fm_correction(accumulators: NDArray, c: NDArray, signs: NDArray, exponent: int) -> NDArray[np.int64]:
    """FMの累算値を正の分母で除し、負値も0方向へ切り捨てる。"""
    a = np.asarray(accumulators, dtype=np.int64)
    numerator = (a * a * signs).sum(axis=1, dtype=np.int64) - c
    return np.sign(numerator) * (np.abs(numerator) // np.int64(1 << (2 * exponent + 1)))


def integer_evaluate(
    middlegame: NDArray, endgame: NDArray, u: NDArray, signs: NDArray, exponent: int,
    features: NDArray, numerators: NDArray,
) -> NDArray[np.int32]:
    """整数PSTとFMを64ビットで加え、最終評価だけを既存上限へ切り詰める。"""
    a, c = fm_accumulators(u, signs, features)
    baseline = pst_evaluate(middlegame, endgame, features, numerators).astype(np.int64)
    return np.clip(baseline + fm_correction(a, c, signs, exponent),
                   -EVALUATION_LIMIT, EVALUATION_LIMIT).astype(np.int32)


def quantize(v: NDArray, a: NDArray, k: float) -> tuple[NDArray, NDArray, int]:
    """出力係数とKを吸収し、最大絶対値16,383以下となる最大の指数で量子化する。"""
    v, a = np.asarray(v, dtype=np.float64), np.asarray(a, dtype=np.float64)
    if (v.ndim != 2 or v.shape[0] != FEATURE_COUNT or a.shape != (v.shape[1],)
            or not a.size or not np.isfinite(v).all() or not np.isfinite(a).all()
            or not math.isfinite(k) or k <= 0):
        raise ValueError("invalid floating FM weights or K")
    absorbed = math.sqrt(k) * v * np.sqrt(np.abs(a))
    if not np.isfinite(absorbed).all() or not np.any(absorbed):
        raise ValueError("absorbed FM embeddings must be finite and nonzero")
    signs = np.where(a < 0, -1, 1).astype(np.int8)
    for exponent in range(30, -1, -1):
        rounded = np.rint(absorbed * (1 << exponent))
        if np.max(np.abs(rounded)) <= 16383:
            return rounded.astype("<i2"), signs, exponent
    raise ValueError("FM embeddings cannot be quantized within 16383 at k=0")


def feature_observations(dataset: Dataset, batch: int) -> NDArray[np.int64]:
    """訓練集合とその全左右鏡映から特徴の出現回数を数える。"""
    counts = np.zeros(FEATURE_COUNT, dtype=np.int64)
    for start in range(0, dataset.training_indices.size, batch):
        records = dataset.gather(dataset.training_indices[start:start + batch])
        reflected_board, reflected_lion = mirror(records["board"], records["lion"])
        for board, lion in ((records["board"], records["lion"]), (reflected_board, reflected_lion)):
            features = feature_indices(board, records["stm"], lion)
            counts += np.bincount(features[features != PADDING_INDEX], minlength=FEATURE_COUNT)
    return counts


class FM(nn.Module):
    """観測行だけを学習する埋め込みと、符号付き出力係数を保持する。"""

    def __init__(self, rank: int, observed: NDArray, device: torch.device) -> None:
        super().__init__()
        if rank < 1 or np.asarray(observed).shape != (FEATURE_COUNT,):
            raise ValueError("invalid rank or observation mask")
        self.embedding = nn.Embedding(FEATURE_COUNT + 1, rank, padding_idx=PADDING_INDEX, device=device)
        self.a = nn.Parameter(torch.zeros(rank, device=device))
        self.register_buffer("observed", torch.as_tensor(np.append(observed, False), dtype=torch.bool, device=device))
        with torch.no_grad():
            self.embedding.weight.normal_(0, math.sqrt(1 / 55))
            self.fix_unobserved()

    @torch.no_grad()
    def fix_unobserved(self) -> None:
        """未観測行とpadding行を、更新後も厳密に0へ固定する。"""
        self.embedding.weight[~self.observed] = 0

    def forward(self, features: Tensor) -> Tensor:
        selected = self.embedding(features) * self.observed[features].unsqueeze(-1)
        pairs = selected.sum(dim=1).square() - selected.square().sum(dim=1)
        return 0.5 * (pairs * self.a).sum(dim=1)


def batch_logits(model: FM, mg: NDArray, eg: NDArray, k: float,
                 features: NDArray, numerators: NDArray, device: torch.device) -> tuple[Tensor, Tensor]:
    """鏡映を選んだ特徴から整数PSTを計算し、FMのロジットと補正を返す。"""
    baseline = pst_evaluate(mg, eg, features, numerators)
    phi = model(torch.as_tensor(features, device=device))
    return torch.as_tensor(baseline, dtype=phi.dtype, device=device) / k + phi, phi


def validation_loss(model: FM, dataset: Dataset, mg: NDArray, eg: NDArray, k: float,
                    teacher_ks: NDArray, batch: int,
                    device: torch.device) -> tuple[float, dict]:
    """正則化を含まない検証BCEと、全検証局面のPhi分布を返す。"""
    total = phi_sum = phi_square_sum = phi_max = 0.0
    with torch.no_grad():
        for start in range(0, dataset.validation_indices.size, batch):
            indices = dataset.validation_indices[start:start + batch]
            records = dataset.gather(indices)
            features = feature_indices(records["board"], records["stm"], records["lion"])
            logits, phi = batch_logits(model, mg, eg, k, features, phase_numerators(records["board"]), device)
            targets = build_targets(records, teacher_ks, dataset.generations(indices), dataset.teacher_lambdas,
                                    scores=dataset.teacher_scores(indices))
            total += float(F.binary_cross_entropy_with_logits(logits, torch.as_tensor(targets, device=device), reduction="sum"))
            values = phi.cpu().numpy().astype(np.float64)
            phi_sum += float(values.sum())
            phi_square_sum += float((values * values).sum())
            phi_max = max(phi_max, float(np.abs(values).max()))
    count = dataset.validation_indices.size
    mean = phi_sum / count
    loss = total / count
    if not all(math.isfinite(value) for value in (loss, mean, phi_square_sum, phi_max)):
        raise ValueError("non-finite validation loss or FM residual")
    return loss, {"mean": mean, "std": math.sqrt(max(0, phi_square_sum / count - mean * mean)),
                  "max_absolute": phi_max, "samples": int(count)}


def zero_correction_report(u: NDArray, signs: NDArray, exponent: int) -> dict:
    """全特徴対を調べ、145特徴までの補正が0となる十分条件も検査する。

    対ごとに切り捨てた値が0でも、和を取ってから切り捨てるFMは非0に
    なり得る。見送りには、対の絶対値の和による上界が1cp未満であることを要する。
    """
    rows = np.asarray(u, dtype=np.int64)
    rows = rows[np.any(rows != 0, axis=1)]
    maximum = absolute_sum = 0
    for start in range(0, len(rows), 256):
        pairs = (rows[start:start + 256] * signs) @ rows.T
        for offset, values in enumerate(pairs):
            values = np.abs(values[start + offset + 1:])
            if values.size:
                maximum = max(maximum, int(values.max()))
                absolute_sum += int(values.sum())
    denominator = 1 << (2 * exponent)
    active = min(145, len(rows))
    upper = min(absolute_sum, maximum * active * (active - 1) // 2)
    return {"all_pair_corrections_zero": maximum < denominator,
            "identically_zero": upper < denominator,
            "max_absolute_pair_numerator": maximum,
            "absolute_sum_bound": upper, "pair_denominator": denominator}


def quantization_report(dataset: Dataset, mg: NDArray, eg: NDArray, k: float,
                        v: NDArray, a: NDArray, u: NDArray, signs: NDArray,
                        exponent: int, batch: int, sample_size: int, seed: int) -> tuple[dict, dict]:
    """同じ検証標本で、補正1/4の前後の誤差と補正の標準偏差を記録する。"""
    random = np.random.default_rng(seed)
    indices = random.choice(dataset.validation_indices, min(sample_size, dataset.validation_indices.size), replace=False)
    errors, quarter_errors, corrections, quarter_corrections = [], [], [], []
    for start in range(0, len(indices), batch):
        records = dataset.gather(indices[start:start + batch])
        features = feature_indices(records["board"], records["stm"], records["lion"])
        baseline = pst_evaluate(mg, eg, features, phase_numerators(records["board"])).astype(np.int64)
        floating = k * fm_phi(v, a, features)
        accumulators, c = fm_accumulators(u, signs, features)
        integer = fm_correction(accumulators, c, signs, exponent)
        # trunc0(trunc0(N / D) / 4) == trunc0(N / (4D)), including negatives.
        quarter = np.sign(integer) * (np.abs(integer) // 4)
        errors.append(np.abs(baseline + floating - np.clip(baseline + integer, -EVALUATION_LIMIT, EVALUATION_LIMIT)))
        quarter_errors.append(np.abs(baseline + floating / 4 - np.clip(baseline + quarter, -EVALUATION_LIMIT, EVALUATION_LIMIT)))
        corrections.append(integer)
        quarter_corrections.append(quarter)
    error, quarter_error = np.concatenate(errors), np.concatenate(quarter_errors)
    if not np.isfinite(error).all() or not np.isfinite(quarter_error).all():
        raise ValueError("non-finite quantization error")
    return ({"exponent": exponent, "samples": len(indices), "seed": seed,
             "mean_absolute_error_cp": float(error.mean()), "max_absolute_error_cp": float(error.max()),
             "quarter_mean_absolute_error_cp": float(quarter_error.mean()),
             "quarter_max_absolute_error_cp": float(quarter_error.max())},
            {"samples": len(indices), "seed": seed, "split": "validation",
             "std_before_quarter": float(np.concatenate(corrections).std()),
             "std_after_quarter": float(np.concatenate(quarter_corrections).std())})


def quarter_weights(source: str | Path, output: str | Path) -> dict:
    """指数だけを1増やし、PST、K、埋め込み、符号をバイト単位で保持する。"""
    source, output = Path(source), Path(output)
    if output.exists():
        raise ValueError("quarter output already exists")
    mg, eg, values, k, u, signs, exponent = read_mnpt_v3(source)
    if exponent == 30:
        raise ValueError("quarter correction exponent would exceed 30")
    check = zero_correction_report(u, signs, exponent + 1)
    write_mnpt_v3(output, mg, eg, values, k, u, signs, exponent + 1)
    return {"input_sha256": sha256_file(source).hex(), "output_sha256": sha256_file(output).hex(),
            "exponent_before": exponent, "exponent_after": exponent + 1,
            "zero_check": check}


def command_quarter(args: argparse.Namespace) -> None:
    report_path = args.output.with_suffix(".quarter.json")
    if report_path.exists():
        raise ValueError("quarter report already exists")
    report = quarter_weights(args.input, args.output)
    report_path.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")
    print(json.dumps(report, allow_nan=False))


def training_report_path(output: str | Path) -> Path:
    """エポックと勾配を保存するJSONのパスを返す。"""
    output = Path(output)
    return output.with_suffix(".training.json")


def command_train(args: argparse.Namespace) -> None:
    """対局単位の分割でFMだけを学習し、最良エポックの保存物を出力する。"""
    for name in ("rank", "epochs", "patience", "batch", "validation_sample"):
        if getattr(args, name) <= 0:
            raise ValueError(f"--{name.replace('_', '-')} must be positive")
    for name in ("lr", "weight_decay", "lambda_res"):
        value = getattr(args, name)
        if not math.isfinite(value) or value < 0 or (name == "lr" and value == 0):
            raise ValueError(f"invalid --{name.replace('_', '-')}")
    validate_lambda_override(args.lambda_override)
    device = torch.device(args.device)
    if device.type == "cuda" and not torch.cuda.is_available():
        raise ValueError("CUDA unavailable; run training on a GPU host")
    mg, eg, piece_values, k = read_mnpt(args.init)
    dataset = Dataset(args.data, lambda_override=args.lambda_override,
                      lookahead=lookahead_options(args.lookahead_gamma, args.lookahead_plies))
    if not dataset.training_indices.size or not dataset.validation_indices.size:
        raise ValueError("training and validation sets must both be nonempty")
    teacher_ks, teacher_counts = estimate_generation_ks(dataset, indices=dataset.training_indices)
    output = Path(args.output)
    report_path = training_report_path(output)
    if any(path.exists() for path in (output, report_path, float_weights_path(output))):
        raise ValueError("training outputs already exist")
    output.parent.mkdir(parents=True, exist_ok=True)
    torch.manual_seed(args.seed)
    if device.type == "cuda":
        torch.cuda.manual_seed_all(args.seed)
    counts = feature_observations(dataset, args.batch)
    model = FM(args.rank, counts > 0, device)
    optimizer = torch.optim.AdamW(model.parameters(), lr=args.lr, weight_decay=args.weight_decay)
    generator = torch.Generator(device=device).manual_seed(args.seed)
    steps = (dataset.training_indices.size + args.batch - 1) // args.batch
    report = {"feature_observations": counts.tolist(), "unobserved_features": int(np.count_nonzero(counts == 0)),
              "steps_per_epoch": int(steps), "planned_steps": int(steps * args.epochs), "total_steps": 0,
              "teacher_ks": [float(value) if np.isfinite(value) else None for value in teacher_ks], "k": k, "gradient_steps": GRADIENT_STEPS,
              "gradients": [], "epochs": [], "status": "training"}

    report.update(
        inputs=[{"path": str(Path(path).resolve()), "sha256": sha256_file(Path(path)).hex(),
                 "provenance_sha256": sha256_file(provenance_path(Path(path))).hex()}
                for path in args.data],
        init={"path": str(Path(args.init).resolve()), "sha256": sha256_file(Path(args.init)).hex()},
        split={"method": "Dataset game hash modulo 20; remainder 0 is validation",
               "training_positions": int(dataset.training_indices.size),
               "validation_positions": int(dataset.validation_indices.size),
               "training_indices_sha256": hashlib.sha256(dataset.training_indices.astype("<i8").tobytes()).hexdigest(),
               "validation_indices_sha256": hashlib.sha256(dataset.validation_indices.astype("<i8").tobytes()).hexdigest()},
        teacher_classes=dataset.class_metadata(), teacher_training_counts=teacher_counts,
        teacher_k_source="estimated from training set", lambda_override=args.lambda_override,
        lookahead=dataset.lookahead, rank=args.rank, optimizer="AdamW", lr=args.lr,
        weight_decay=args.weight_decay, lambda_res=args.lambda_res, batch=args.batch,
        seed=args.seed, device=args.device, max_epochs=args.epochs, patience=args.patience,
        validation_sample=args.validation_sample, last_epoch=0,
        packages={"numpy": np.__version__, "torch": torch.__version__},
    )

    def save_report() -> None:
        report_path.write_text(json.dumps(report, indent=2, ensure_ascii=False, allow_nan=False) + "\n")

    try:
        initial_loss, distribution = validation_loss(model, dataset, mg, eg, k, teacher_ks, args.batch, device)
        best_epoch = 0
        best_loss, best = math.inf, None
        report["epochs"].append({"epoch": 0, "train_loss": None, "validation_loss": initial_loss, "phi": distribution})
        report.update(best_epoch=None, best_validation_loss=None, initial_validation_loss=initial_loss)
        save_report()
        print(f"epoch 0: validation_loss={initial_loss:.9f}", flush=True)
        for epoch in range(1, args.epochs + 1):
            order = torch.randperm(dataset.training_indices.size, generator=generator, device=device)
            total = bce_total = 0.0
            for start in range(0, len(order), args.batch):
                indices = dataset.training_indices[order[start:start + args.batch].cpu().numpy()]
                records = dataset.gather(indices)
                features = feature_indices(records["board"], records["stm"], records["lion"])
                board, lion = mirror(records["board"], records["lion"])
                reflected = feature_indices(board, records["stm"], lion)
                selected = (torch.rand(len(indices), generator=generator, device=device) < 0.5).cpu().numpy()
                features[selected] = reflected[selected]
                targets = build_targets(records, teacher_ks, dataset.generations(indices), dataset.teacher_lambdas,
                                        scores=dataset.teacher_scores(indices))
                optimizer.zero_grad(set_to_none=True)
                logits, phi = batch_logits(model, mg, eg, k, features, phase_numerators(records["board"]), device)
                bce = F.binary_cross_entropy_with_logits(logits, torch.as_tensor(targets, device=device))
                loss = bce + args.lambda_res * phi.square().mean()
                if not torch.isfinite(loss):
                    raise ValueError("non-finite training loss")
                loss.backward()
                if report["total_steps"] < GRADIENT_STEPS:
                    norms = {"step": report["total_steps"] + 1,
                             "a": float(model.a.grad.norm()), "embedding": float(model.embedding.weight.grad.norm())}
                    if not math.isfinite(norms["a"]) or not math.isfinite(norms["embedding"]):
                        raise ValueError("non-finite loss gradient")
                    report["gradients"].append(norms)
                    if len(report["gradients"]) == GRADIENT_STEPS and all(g["embedding"] == 0 for g in report["gradients"]):
                        raise ValueError(f"embedding loss gradient stayed zero for {GRADIENT_STEPS} steps")
                optimizer.step()
                model.fix_unobserved()
                report["total_steps"] += 1
                total += float(loss.detach()) * len(indices)
                bce_total += float(bce.detach()) * len(indices)
            loss, distribution = validation_loss(model, dataset, mg, eg, k, teacher_ks, args.batch, device)
            report["epochs"].append({"epoch": epoch, "train_loss": bce_total / dataset.training_indices.size,
                                     "total_loss": total / dataset.training_indices.size,
                                     "validation_loss": loss, "phi": distribution, "steps": int(steps)})
            print(f"epoch {epoch}: train_loss={bce_total / dataset.training_indices.size:.9f} total_loss={total / dataset.training_indices.size:.9f} validation_loss={loss:.9f}", flush=True)
            if should_replace_best_epoch(loss, best_loss):
                best_loss, best_epoch = loss, epoch
                best = {name: value.detach().cpu().clone() for name, value in model.state_dict().items()}
            report.update(best_epoch=best_epoch, best_validation_loss=best_loss, last_epoch=epoch)
            save_report()
            if should_stop_early(epoch, best_epoch, args.patience):
                break
        assert best is not None
        model.load_state_dict(best)
        v = model.embedding.weight[:FEATURE_COUNT].detach().cpu().numpy()
        a = model.a.detach().cpu().numpy()
        with float_weights_path(output).open("xb") as stream:
            np.savez(stream, V=v, a=a, mask=counts > 0)
        report["phi"] = report["epochs"][best_epoch]["phi"]
        report["improved_over_epoch_zero"] = best_loss < initial_loss
        # A zero learned output is a recorded exclusion, not a quantization failure.
        if not np.any(v * np.sqrt(np.abs(a))):
            u, signs, exponent = np.zeros(v.shape, dtype=np.int16), np.ones(len(a), dtype=np.int8), 0
        else:
            u, signs, exponent = quantize(v, a, k)
        report["quarter_zero_check"] = zero_correction_report(u, signs, exponent + 1)
        if exponent == 30:
            # The format cannot store 31. A nonzero 1/4 correction is impossible
            # at this scale under the i16, rank and 145-feature bounds used here.
            if not report["quarter_zero_check"]["identically_zero"]:
                raise ValueError("quarter correction exponent would exceed 30")
            report.update(status="excluded_zero_correction", reason="integer FM correction is identically zero")
        else:
            report["status"] = ("excluded_zero_correction" if report["quarter_zero_check"]["identically_zero"]
                                else "candidate")
        report["quantization"], report["correction_cp"] = quantization_report(
            dataset, mg, eg, k, v, a, u, signs, exponent, args.batch, args.validation_sample, args.seed)
        if max(report["quantization"]["mean_absolute_error_cp"],
               report["quantization"]["quarter_mean_absolute_error_cp"]) > QUANTIZATION_ERROR_LIMIT:
            raise ValueError("FM quantization mean absolute error exceeds 2 cp")
        write_mnpt_v3(output, mg, eg, piece_values, k, u, signs, exponent)
        validate_fixed_base(args.init, output)
        report["output_sha256"] = sha256_file(output).hex()
        report["float_sha256"] = sha256_file(float_weights_path(output)).hex()
        if report["status"] == "excluded_zero_correction":
            report["reason"] = "integer quarter FM correction is identically zero; do not run matches"
        save_report()
        print(f"best epoch: {best_epoch}; status: {report['status']}", flush=True)
    except Exception as error:
        report.update(status="error", reason=str(error))
        save_report()
        raise


def build_parser() -> argparse.ArgumentParser:
    """FM学習と補正1/4の変換のCLIを作る。教師Kは訓練集合から推定する。"""
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    train = commands.add_parser("train")
    train.add_argument("--data", nargs="+", required=True)
    train.add_argument("--init", required=True)
    train.add_argument("--output", required=True)
    train.add_argument("--epochs", type=int, required=True)
    train.add_argument("--patience", type=int, required=True)
    for name, default in (("rank", 32), ("batch", 4096), ("seed", 1), ("validation-sample", 10000)):
        train.add_argument("--" + name, type=int, default=default)
    for name, default in (("lr", 1e-3), ("weight-decay", 1e-4), ("lambda-res", 1e-4)):
        train.add_argument("--" + name, type=float, default=default)
    train.add_argument("--lambda-override", type=float, default=1.0)
    train.add_argument("--lookahead-gamma", type=float)
    train.add_argument("--lookahead-plies", type=int)
    train.add_argument("--device", choices=("cpu", "cuda"), required=True)
    train.set_defaults(handler=command_train)
    quarter = commands.add_parser("quarter", help="学習後のMNPT v3の補正を1/4にする")
    quarter.add_argument("--input", required=True, type=Path)
    quarter.add_argument("--output", required=True, type=Path)
    quarter.set_defaults(handler=command_quarter)
    return parser


def main(arguments: Sequence[str] | None = None) -> None:
    """CLI引数に従ってFM学習を実行する。"""
    args = build_parser().parse_args(arguments)
    args.handler(args)


if __name__ == "__main__":
    main()
