#!/usr/bin/env python3
r"""2つのネットの検証損失を、対局単位の対応のあるブートストラップで比べる。

作業ディレクトリをtools/train/zeroにして、すべての引数を明示して実行する。
出力先は、既存のディレクトリ内の、まだ存在しないファイルを指定する。

    uv run --frozen python ../../../docs/measurements/alphazero-multi-gpu/compare-validation.py \
        --reference /tmp/reference/checkpoint.pt --candidate /tmp/candidate/checkpoint.pt \
        --data /tmp/cycle-1.mnsd /tmp/cycle-2.mnsd --value-lambda 0.333 \
        --batch 256 --resamples 10000 --seed 42 --device cpu --out /tmp/comparison.json

各ファイルの基本シードと対局番号でdata.is_validationが真となる対局から、
学習器のdata.read_fileが採用する全局面を読む。
対局は基本シードと対局番号の組で識別し、重複する入力はエラーにする。
両ネットをeval()とtorch.inference_mode()で動かし、float32で損失を計算する。
方策の損失は、学習器と同じ定数マスク内で正規化した方策と教師との交差エントロピーである。
価値の損失は、教師を(1−λ)×対局結果＋λ×根の価値とする二値交差エントロピーである。
対局結果と根の価値は手番側の値を使い、量子化した根の価値を65,535で割る。

対局gの採用局面数をn_g、ネットAとBの損失和をS_A,gとS_B,gとする。
点推定はΣ_g(S_B,g−S_A,g) / Σ_g n_gとし、損失和とブートストラップはfloat64で計算する。
各回、G対局からG対局を復元抽出し、同じ抽出列を両ネットと両損失に当てる。
乱数はnumpy.random.Generator(PCG64(seed))を使い、抽出後の損失和の差を抽出後の局面数で割る。
この差をresamples回求め、2.5%点と97.5%点を線形補間して95%区間とする。
両損失の区間の上限がともに0以下なら「候補は劣らない」と判定する。
いずれかの下限が0より大きければ「候補は劣る」、それ以外は「判定できない」とする。
"""

import argparse
import json
import math
from pathlib import Path
import sys

import numpy as np
from numpy.typing import NDArray
import torch
from torch.nn import functional as F


sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools" / "train" / "zero"))

from minase_zero.data import read_file
from minase_zero.encoding import policy_mask
from minase_zero.train import _batch, _load_checkpoint, _model, _sha256, _validate_device


LOSS_NAMES = ("policy", "value")


def per_position_losses(
    policy_logits: torch.Tensor, value_logits: torch.Tensor,
    rows: torch.Tensor, labels: torch.Tensor, probabilities: torch.Tensor,
    results: torch.Tensor, values: torch.Tensor, value_lambda: float, mask: torch.Tensor,
) -> tuple[torch.Tensor, torch.Tensor]:
    """compute_lossesの主方策と主価値の損失を、平均前の局面ごとの値で返す。"""
    policy_logits, value_logits = policy_logits.float(), value_logits.float().flatten()
    probabilities, results, values = probabilities.float(), results.float(), values.float()
    if torch.any((labels < 0) | (labels >= len(mask))) or not torch.all(mask[labels]):
        raise ValueError("policy target label is outside the constant mask")
    log_policy = F.log_softmax(policy_logits.masked_fill(~mask, -torch.inf), dim=1)
    candidate_losses = -log_policy[rows, labels] * (probabilities / 65_535)
    policy = torch.zeros(len(results), dtype=torch.float32, device=policy_logits.device)
    policy.scatter_add_(0, rows, candidate_losses)
    target = (1 - value_lambda) * results + value_lambda * (values / 65_535)
    value = F.binary_cross_entropy_with_logits(value_logits, target, reduction="none")
    return policy, value


def paired_game_bootstrap(
    reference_sums: NDArray[np.float64], candidate_sums: NDArray[np.float64],
    counts: NDArray[np.int64], resamples: int, seed: int,
) -> dict:
    """対局ごとの(policy, value)の和と局面数から、B−Aの点推定と95%区間を返す。"""
    counts = np.asarray(counts)
    reference_sums = np.asarray(reference_sums, dtype=np.float64)
    candidate_sums = np.asarray(candidate_sums, dtype=np.float64)
    if counts.ndim != 1 or len(counts) == 0 or counts.dtype.kind not in "iu" or np.any(counts <= 0):
        raise ValueError("each validation game must have a positive integer position count")
    if reference_sums.shape != (len(counts), 2) or candidate_sums.shape != reference_sums.shape:
        raise ValueError("loss sums must have shape (game_count, 2) for both networks")
    if not np.isfinite(reference_sums).all() or not np.isfinite(candidate_sums).all():
        raise ValueError("validation loss sums must be finite")
    if resamples <= 0 or seed < 0:
        raise ValueError("resamples must be positive and seed must be nonnegative")

    position_count = int(counts.sum())
    differences = candidate_sums - reference_sums
    estimate = differences.sum(axis=0) / position_count
    rng = np.random.Generator(np.random.PCG64(seed))
    sampled = np.empty((resamples, 2), dtype=np.float64)
    for index in range(resamples):
        games = rng.integers(0, len(counts), size=len(counts))
        sampled[index] = differences[games].sum(axis=0) / counts[games].sum()
    lower, upper = np.quantile(sampled, [0.025, 0.975], axis=0, method="linear")
    if np.all(upper <= 0):
        decision = "候補は劣らない"
    elif np.any(lower > 0):
        decision = "候補は劣る"
    else:
        decision = "判定できない"
    return {
        "game_count": len(counts), "position_count": position_count,
        "reference": dict(zip(LOSS_NAMES, (reference_sums.sum(axis=0) / position_count).tolist(), strict=True)),
        "candidate": dict(zip(LOSS_NAMES, (candidate_sums.sum(axis=0) / position_count).tolist(), strict=True)),
        "difference": {
            name: {"estimate": float(estimate[index]), "ci95": [float(lower[index]), float(upper[index])]}
            for index, name in enumerate(LOSS_NAMES)
        },
        "resamples": resamples, "seed": seed, "decision": decision,
    }


@torch.inference_mode()
def compare_validation(args: argparse.Namespace) -> dict:
    """学習器の読み込みとバッチ作成を使い、全入力の検証対局を集計する。"""
    device = _validate_device(args.device, "fp32")
    paths = [path.resolve(strict=True) for path in args.data]
    if len(set(paths)) != len(paths):
        raise ValueError("duplicate normalized MNSD path")
    checkpoints = [args.reference.resolve(strict=True), args.candidate.resolve(strict=True)]
    hashes = [_sha256(path) for path in checkpoints]
    models = [_model(_load_checkpoint(path)).to(device).eval() for path in checkpoints]
    mask = torch.from_numpy(policy_mask()).to(device)
    seen_games: set[tuple[int, int]] = set()
    all_sums, all_counts, files = [], [], []
    for path in paths:
        data, info = read_file(path)
        for game in info.game_numbers:
            key = (info.header.seed, game)
            if key in seen_games:
                raise ValueError(f"{path}: duplicate game (seed={key[0]}, game={key[1]})")
            seen_games.add(key)
        indices = np.flatnonzero(data.validation)
        games, groups, counts = np.unique(data.games[indices], return_inverse=True, return_counts=True)
        files.append({
            "path": str(path), "seed": info.header.seed,
            "game_count": len(games), "position_count": len(indices),
        })
        sums = np.zeros((2, len(games), 2), dtype=np.float64)
        for start in range(0, len(indices), args.batch):
            selected = indices[start:start + args.batch]
            for model_index, model in enumerate(models):
                logits, policy, results, values = _batch(
                    model.forward_logits, data, selected, device, {"precision": "fp32"},
                )
                losses = torch.stack(per_position_losses(
                    logits.policy, logits.value[:, 0], policy.rows, policy.labels,
                    policy.probabilities, results, values, args.value_lambda, mask,
                ), dim=1)
                if not torch.isfinite(losses).all():
                    raise ValueError(f"{path}: non-finite validation loss")
                np.add.at(sums[model_index], groups[start:start + args.batch], losses.cpu().double().numpy())
        all_sums.append(sums)
        all_counts.append(counts)
    sums = np.concatenate(all_sums, axis=1)
    summary = paired_game_bootstrap(sums[0], sums[1], np.concatenate(all_counts), args.resamples, args.seed)
    for name, path, checksum in zip(("reference", "candidate"), checkpoints, hashes, strict=True):
        summary[name] = {"path": str(path), "sha256": checksum, "mean_losses": summary[name]}
    return {
        **summary, "data": files, "value_lambda": args.value_lambda,
        "batch": args.batch, "device": args.device,
    }


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter,
                                     allow_abbrev=False)
    for name in ("reference", "candidate", "out"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--data", type=Path, nargs="+", required=True)
    parser.add_argument("--value-lambda", type=float, required=True)
    for name in ("batch", "resamples", "seed"):
        parser.add_argument(f"--{name}", type=int, required=True)
    parser.add_argument("--device", choices=("cuda", "cpu"), required=True)
    args = parser.parse_args(argv)
    if not math.isfinite(args.value_lambda) or not 0 <= args.value_lambda <= 1:
        parser.error("--value-lambda must be finite and between 0 and 1")
    if args.batch <= 0 or args.resamples <= 0:
        parser.error("--batch and --resamples must be positive")
    if args.seed < 0:
        parser.error("--seed must be nonnegative")
    return args


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        if args.out.exists():
            raise FileExistsError(f"output already exists: {args.out}")
        summary = compare_validation(args)
        serialized = json.dumps(summary, ensure_ascii=False, allow_nan=False, indent=2) + "\n"
        with args.out.open("x", encoding="utf-8") as output:
            output.write(serialized)
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
