"""固定した2端点PSTへ加えるFactorization Machineを学習し、MNPT v3へ保存する。"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
from typing import Sequence

import numpy as np
from numpy.typing import NDArray
import torch
from torch import Tensor, nn
from torch.nn import functional as F

from features import FEATURE_COUNT, PADDING_INDEX, feature_indices, mirror
from mnsd import Dataset
from taper import phase_numerators
from train_pst import (
    BODY_LENGTH, HEADER_LENGTH, EVALUATION_LIMIT, RULE_SET, QUANTIZATION_ERROR_LIMIT,
    build_targets, float_weights_path, integer_evaluate as pst_evaluate,
    read_mnpt, should_replace_best_epoch, validate_piece_values,
)

FORMAT_VERSION = 3
GRADIENT_STEPS = 5
# 診断の大きな局面バッチでも、特徴×次元の一時配列を一定サイズに抑える。
REFERENCE_BATCH = 1024


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
    if not math.isfinite(k) or not 0 < k <= np.finfo(np.float32).max:
        raise ValueError("K must be finite and positive in float32")
    encoded_k = struct.pack("<f", k)
    if struct.unpack("<f", encoded_k)[0] <= 0:
        raise ValueError("K underflows float32")
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
    header = (b"MNPT" + struct.pack("<II", FORMAT_VERSION, FEATURE_COUNT) + encoded_k
              + RULE_SET.ljust(32, b"\0") + hashlib.sha256(body).digest())
    Path(path).write_bytes(header + body)


def read_mnpt_v3(path: str | Path) -> tuple[NDArray, NDArray, NDArray, float, NDArray, NDArray, int]:
    """v3を完全検証し、両端点、駒価値、K、U、符号、指数を返す。"""
    raw = Path(path).read_bytes()
    if len(raw) < HEADER_LENGTH + BODY_LENGTH + 8:
        raise ValueError(f"{path}: truncated MNPT v3")
    magic, version, count, k = struct.unpack_from("<4sIIf", raw)
    if magic != b"MNPT" or version != FORMAT_VERSION or count != FEATURE_COUNT:
        raise ValueError(f"{path}: invalid MNPT magic, version, or feature count")
    if not math.isfinite(k) or k <= 0:
        raise ValueError(f"{path}: K must be finite and positive")
    if raw[16:48] != RULE_SET.ljust(32, b"\0"):
        raise ValueError(f"{path}: unexpected rule-set field")
    body = raw[HEADER_LENGTH:]
    if hashlib.sha256(body).digest() != raw[48:80]:
        raise ValueError(f"{path}: SHA-256 mismatch")
    rank, exponent = struct.unpack_from("<II", body, BODY_LENGTH)
    if rank < 1 or exponent > 30:
        raise ValueError(f"{path}: invalid FM rank or exponent")
    if len(body) != BODY_LENGTH + 8 + rank + FEATURE_COUNT * rank * 2:
        raise ValueError(f"{path}: invalid MNPT v3 length")
    weights = np.frombuffer(body, dtype="<i2", count=FEATURE_COUNT * 2).reshape(2, FEATURE_COUNT)
    values = np.frombuffer(body, dtype="<i4", count=47, offset=FEATURE_COUNT * 4).copy()
    validate_piece_values(values, str(path))
    signs = np.frombuffer(body, dtype="i1", count=rank, offset=BODY_LENGTH + 8).copy()
    if not np.all((signs == 1) | (signs == -1)):
        raise ValueError(f"{path}: FM signs must be +1 or -1")
    u = np.frombuffer(body, dtype="<i2", offset=BODY_LENGTH + 8 + rank).reshape(FEATURE_COUNT, rank).copy()
    return weights[0].copy(), weights[1].copy(), values, float(k), u, signs, exponent


def validate_fixed_base(base_path: str | Path, candidate_path: str | Path) -> None:
    """v3の両端点、探索用駒価値、およびKが基準v2と厳密に一致することを確かめる。"""
    base = read_mnpt(base_path)
    candidate = read_mnpt_v3(candidate_path)
    for name, expected, actual in zip(("middlegame", "endgame", "piece values", "K"), base, candidate[:4]):
        if not np.array_equal(expected, actual):
            raise ValueError(f"FM candidate changed fixed {name}")


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
                    teacher_ks: NDArray, lambda_value: float, batch: int,
                    device: torch.device) -> tuple[float, dict]:
    """正則化を含まない検証BCEと、全検証局面のPhi分布を返す。"""
    total = phi_sum = phi_square_sum = phi_max = 0.0
    with torch.no_grad():
        for start in range(0, dataset.validation_indices.size, batch):
            indices = dataset.validation_indices[start:start + batch]
            records = dataset.gather(indices)
            features = feature_indices(records["board"], records["stm"], records["lion"])
            logits, phi = batch_logits(model, mg, eg, k, features, phase_numerators(records["board"]), device)
            targets = build_targets(records, teacher_ks, dataset.generations(indices), lambda_value)
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


def training_report_path(output: str | Path) -> Path:
    """エポックと勾配を保存するJSONのパスを返す。"""
    output = Path(output)
    return output.with_name(output.stem + "-training.json")


def command_train(args: argparse.Namespace) -> None:
    """対局単位の分割でFMだけを学習し、最良エポックの保存物を出力する。"""
    for name in ("rank", "epochs", "patience", "batch", "validation_sample"):
        if getattr(args, name) <= 0:
            raise ValueError(f"--{name.replace('_', '-')} must be positive")
    for name in ("lr", "weight_decay", "lambda_res"):
        value = getattr(args, name)
        if not math.isfinite(value) or value < 0 or (name == "lr" and value == 0):
            raise ValueError(f"invalid --{name.replace('_', '-')}")
    if not 0 <= args.lambda_value <= 1:
        raise ValueError("--lambda must be in 0..1")
    device = torch.device(args.device)
    if device.type == "cuda" and not torch.cuda.is_available():
        raise ValueError("CUDA unavailable; run training on a GPU host")
    mg, eg, piece_values, k = read_mnpt(args.init)
    if args.k != k:
        raise ValueError("--k must equal the decoded baseline K")
    dataset = Dataset(args.data)
    if not dataset.training_indices.size or not dataset.validation_indices.size:
        raise ValueError("training and validation sets must both be nonempty")
    teacher_ks = np.asarray(args.teacher_ks, dtype=np.float64)
    if (teacher_ks.shape != (dataset.generation_count,) or not np.isfinite(teacher_ks).all()
            or np.any(teacher_ks <= 0)):
        raise ValueError("--teacher-ks must give one finite positive K per generation")
    output = Path(args.output)
    report_path = training_report_path(output)
    if any(path.exists() for path in (output, report_path, float_weights_path(output))):
        raise ValueError("training outputs already exist")
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
              "teacher_ks": teacher_ks.tolist(), "k": k, "gradient_steps": GRADIENT_STEPS,
              "gradients": [], "epochs": [], "status": "training"}

    def save_report() -> None:
        report_path.write_text(json.dumps(report, indent=2, ensure_ascii=False, allow_nan=False) + "\n")

    try:
        best_loss, distribution = validation_loss(model, dataset, mg, eg, k, teacher_ks, args.lambda_value, args.batch, device)
        best_epoch = 0
        best = {name: value.detach().cpu().clone() for name, value in model.state_dict().items()}
        report["epochs"].append({"epoch": 0, "train_loss": None, "validation_loss": best_loss, "phi": distribution})
        report.update(best_epoch=0, best_validation_loss=best_loss)
        save_report()
        print(f"epoch 0: validation_loss={best_loss:.9f}", flush=True)
        for epoch in range(1, args.epochs + 1):
            order = torch.randperm(dataset.training_indices.size, generator=generator, device=device)
            total = 0.0
            for start in range(0, len(order), args.batch):
                indices = dataset.training_indices[order[start:start + args.batch].cpu().numpy()]
                records = dataset.gather(indices)
                features = feature_indices(records["board"], records["stm"], records["lion"])
                board, lion = mirror(records["board"], records["lion"])
                reflected = feature_indices(board, records["stm"], lion)
                selected = (torch.rand(len(indices), generator=generator, device=device) < 0.5).cpu().numpy()
                features[selected] = reflected[selected]
                targets = build_targets(records, teacher_ks, dataset.generations(indices), args.lambda_value)
                optimizer.zero_grad(set_to_none=True)
                logits, phi = batch_logits(model, mg, eg, k, features, phase_numerators(records["board"]), device)
                loss = (F.binary_cross_entropy_with_logits(logits, torch.as_tensor(targets, device=device))
                        + args.lambda_res * phi.square().mean())
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
            loss, distribution = validation_loss(model, dataset, mg, eg, k, teacher_ks, args.lambda_value, args.batch, device)
            report["epochs"].append({"epoch": epoch, "train_loss": total / dataset.training_indices.size,
                                     "validation_loss": loss, "phi": distribution, "steps": int(steps)})
            print(f"epoch {epoch}: train_loss={total / dataset.training_indices.size:.9f} validation_loss={loss:.9f}", flush=True)
            if should_replace_best_epoch(loss, best_loss):
                best_loss, best_epoch = loss, epoch
                best = {name: value.detach().cpu().clone() for name, value in model.state_dict().items()}
            report.update(best_epoch=best_epoch, best_validation_loss=best_loss)
            save_report()
            if epoch - best_epoch >= args.patience:
                break
        model.load_state_dict(best)
        v = model.embedding.weight[:FEATURE_COUNT].detach().cpu().numpy()
        a = model.a.detach().cpu().numpy()
        with float_weights_path(output).open("xb") as stream:
            np.savez(stream, V=v, a=a, mask=counts > 0)
        report["phi"] = report["epochs"][best_epoch]["phi"]
        if best_epoch == 0:
            report.update(status="excluded_epoch_zero", quantization=None,
                          reason="best epoch is 0; excluded from quantization and candidate selection")
        else:
            u, signs, exponent = quantize(v, a, k)
            random = np.random.default_rng(args.seed)
            indices = random.choice(dataset.validation_indices, min(args.validation_sample, dataset.validation_indices.size), replace=False)
            errors = []
            for start in range(0, len(indices), args.batch):
                records = dataset.gather(indices[start:start + args.batch])
                features = feature_indices(records["board"], records["stm"], records["lion"])
                q = phase_numerators(records["board"])
                floating = float_evaluate(mg, eg, k, v, a, features, q)
                integer = integer_evaluate(mg, eg, u, signs, exponent, features, q)
                errors.append(np.abs(floating - integer))
            error = np.concatenate(errors)
            report["quantization"] = {"exponent": exponent, "samples": len(indices),
                                      "mean_absolute_error_cp": float(error.mean()), "max_absolute_error_cp": float(error.max())}
            if not np.isfinite(error).all() or error.mean() > QUANTIZATION_ERROR_LIMIT:
                raise ValueError("FM quantization mean absolute error exceeds 2 cp")
            write_mnpt_v3(output, mg, eg, piece_values, k, u, signs, exponent)
            validate_fixed_base(args.init, output)
            report["status"] = "candidate"
        save_report()
        print(f"best epoch: {best_epoch}; status: {report['status']}", flush=True)
    except Exception as error:
        report.update(status="error", reason=str(error))
        save_report()
        raise


def build_parser() -> argparse.ArgumentParser:
    """FM学習CLIを作る。教師Kはデータの世代番号順に受け取る。"""
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    train = commands.add_parser("train")
    train.add_argument("--data", nargs="+", required=True)
    train.add_argument("--init", required=True)
    train.add_argument("--output", required=True)
    train.add_argument("--teacher-ks", type=float, nargs="+", required=True)
    train.add_argument("--k", type=float, required=True)
    for name in ("rank", "epochs", "patience", "batch", "seed", "validation-sample"):
        train.add_argument("--" + name, type=int, required=True)
    for name in ("lr", "weight-decay", "lambda-res"):
        train.add_argument("--" + name, type=float, required=True)
    train.add_argument("--lambda", dest="lambda_value", type=float, default=0.75)
    train.add_argument("--device", choices=("cpu", "cuda"), required=True)
    train.set_defaults(handler=command_train)
    return parser


def main(arguments: Sequence[str] | None = None) -> None:
    """CLI引数に従ってFM学習を実行する。"""
    args = build_parser().parse_args(arguments)
    args.handler(args)


if __name__ == "__main__":
    main()
