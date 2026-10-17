"""同じ一様非復元標本で、手番側から見た整数PST評価値の分布を比較する。

tools/train/.venv/bin/pythonで実行する。回帰は
candidate = intercept + slope * baseline とし、切片を含める。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

import numpy as np

from minase_train.checksum import sha256_file
from minase_train.data.features import feature_indices
from minase_train.data.mnpt import read_mnpt
from minase_train.data.mnsd import Dataset
from minase_train.data.taper import phase_numerators
from minase_train.pst.evaluate import integer_evaluate


BATCH_SIZE = 8192
PHASE_INTERVALS = ((0, 29), (30, 59), (60, 90))
BASELINE_ABSOLUTE_INTERVALS = ((0, 99), (100, 299), (300, 999), (1000, None))


def average_ranks(values: np.ndarray) -> np.ndarray:
    """同値の要素には、その要素が占める順位の平均を割り当てる。"""
    _, inverse, counts = np.unique(values, return_inverse=True, return_counts=True)
    ends = np.cumsum(counts)
    return (ends - (counts - 1) / 2)[inverse]


def scale_verdict(ratio: float) -> str:
    """全局面の標準偏差比だけで散らばりの尺度を判定する。"""
    return ("全体の尺度はほとんど変えていない" if 0.9 <= ratio <= 1.1
            else "散らばりの尺度を変えた")


def summarize(scores: np.ndarray) -> dict:
    """母標準偏差と、既定の線形補間による絶対値の百分位点を返す。"""
    if scores.size == 0:
        raise ValueError("cannot summarize an empty evaluation distribution")
    absolute = np.abs(scores.astype(np.float64))
    percentiles = (5, 25, 50, 75, 95)
    return {
        "mean_cp": float(np.mean(scores)),
        "std_cp": float(np.std(scores, ddof=0)),
        "mean_absolute_cp": float(np.mean(absolute)),
        "absolute_percentiles_cp": dict(zip(
            map(str, percentiles), np.percentile(absolute, percentiles).tolist()
        )),
    }


def compare(baseline: np.ndarray, candidate: np.ndarray) -> dict:
    """候補を目的変数、基点を説明変数とする切片あり回帰で比較する。"""
    if baseline.size < 2 or baseline.shape != candidate.shape:
        raise ValueError("comparison requires at least two paired evaluations")
    baseline = baseline.astype(np.float64)
    candidate = candidate.astype(np.float64)
    base_centered = baseline - baseline.mean()
    candidate_centered = candidate - candidate.mean()
    base_std = float(np.std(baseline, ddof=0))
    candidate_std = float(np.std(candidate, ddof=0))
    if base_std == 0 or candidate_std == 0:
        raise ValueError("both evaluation distributions must have nonzero variance")
    ratio = candidate_std / base_std
    slope = float(np.dot(base_centered, candidate_centered)
                  / np.dot(base_centered, base_centered))
    difference = candidate - baseline
    absolute_difference = np.abs(difference)
    percentiles = (50, 90, 99)
    return {
        "std_ratio_candidate_over_baseline": ratio,
        "regression_slope": slope,
        "regression_intercept_cp": float(candidate.mean() - slope * baseline.mean()),
        "pearson_correlation": float(np.corrcoef(baseline, candidate)[0, 1]),
        "spearman_correlation": float(np.corrcoef(
            average_ranks(baseline), average_ranks(candidate)
        )[0, 1]),
        "regression_residual_std_cp": float(np.std(
            candidate_centered - slope * base_centered, ddof=0
        )),
        "difference_std_cp": float(np.std(difference, ddof=0)),
        "absolute_difference_percentiles_cp": dict(zip(
            map(str, percentiles), np.percentile(absolute_difference, percentiles).tolist()
        )),
        "absolute_difference_over_100_cp_fraction": float(np.mean(absolute_difference > 100)),
    }


def compare_intervals(
    baseline: np.ndarray, candidate: np.ndarray, values: np.ndarray,
    intervals: tuple[tuple[int, int | None], ...],
) -> list[dict]:
    """指定した閉区間ごとに、両PSTの統計と組の指標を求める。"""
    reports = []
    for lower, upper in intervals:
        mask = values >= lower
        if upper is not None:
            mask &= values <= upper
        base_scores, candidate_scores = baseline[mask], candidate[mask]
        try:
            reports.append({
                "lower_inclusive": lower,
                "upper_inclusive": upper,
                "sample_count": int(np.count_nonzero(mask)),
                "baseline_statistics": summarize(base_scores),
                "candidate_statistics": summarize(candidate_scores),
                **compare(base_scores, candidate_scores),
            })
        except ValueError as error:
            raise ValueError(f"interval [{lower}, {upper}]: {error}") from error
    return reports


def analyze(data: list[Path], sample: int, seed: int, pairs: list[list[str]]) -> dict:
    """全ファイルを引数順に連結し、すべての重みを同じ標本で評価する。"""
    if sample < 2:
        raise ValueError("sample must be at least 2")
    names = [name for name, _, _ in pairs]
    if len(set(names)) != len(names):
        raise ValueError("pair names must be unique")
    resolved_pairs = [(name, Path(base).resolve(), Path(candidate).resolve())
                      for name, base, candidate in pairs]
    paths = list(dict.fromkeys(path for _, base, candidate in resolved_pairs
                              for path in (base, candidate)))
    weights = {path: read_mnpt(path)[:2] for path in paths}
    identities = {path: {"path": str(path), "sha256": sha256_file(path).hex()}
                  for path in paths}

    print("Loading and validating MNSD files", file=sys.stderr, flush=True)
    dataset = Dataset(data)
    if sample > dataset.record_count:
        raise ValueError(f"sample {sample} exceeds {dataset.record_count} positions")
    indices = np.sort(np.random.default_rng(seed).choice(
        dataset.record_count, sample, replace=False
    ))
    scores = {path: np.empty(sample, dtype=np.int32) for path in paths}
    phases = np.empty(sample, dtype=np.int64)
    print(f"Evaluating {sample} positions with {len(paths)} PSTs", file=sys.stderr, flush=True)
    for start in range(0, sample, BATCH_SIZE):
        stop = min(start + BATCH_SIZE, sample)
        records = dataset.gather(indices[start:stop])
        # comparison.Weights.evaluate / check_probeと同じ手番側の整数評価経路。
        features = feature_indices(records["board"], records["stm"], records["lion"])
        numerators = phase_numerators(records["board"])
        phases[start:stop] = numerators
        for path, (middlegame, endgame) in weights.items():
            scores[path][start:stop] = integer_evaluate(
                middlegame, endgame, features, numerators
            )

    comparisons = []
    for name, base, candidate in resolved_pairs:
        base_scores, candidate_scores = scores[base], scores[candidate]
        metrics = compare(base_scores, candidate_scores)
        comparisons.append({
            "name": name,
            "baseline": identities[base],
            "candidate": identities[candidate],
            "sample_count": sample,
            **metrics,
            "verdict": scale_verdict(metrics["std_ratio_candidate_over_baseline"]),
            "by_phase_q": compare_intervals(
                base_scores, candidate_scores, phases, PHASE_INTERVALS
            ),
            "by_baseline_absolute_cp": compare_intervals(
                base_scores, candidate_scores, np.abs(base_scores), BASELINE_ABSOLUTE_INTERVALS
            ),
        })

    return {
        "sample_count": sample,
        "seed": seed,
        "population_count": dataset.record_count,
        "sampling": "sorted numpy.random.default_rng(seed).choice(total, sample, replace=False)",
        "sample_indices_sha256_le_i64": hashlib.sha256(indices.astype("<i8").tobytes()).hexdigest(),
        "evaluation_perspective": "side_to_move",
        "standard_deviation_ddof": 0,
        "percentile_method": "linear",
        "spearman_tie_method": "average",
        "regression_equation": "candidate = intercept + slope * baseline",
        "unchanged_scale_interval_inclusive": [0.9, 1.1],
        "input_files": [
            {"path": str(path), "sha256": sha256_file(path).hex(),
             "record_count": header.record_count}
            for path, header in zip(dataset.paths, dataset.headers)
        ],
        "psts": [dict(identities[path], **summarize(scores[path])) for path in paths],
        "pairs": comparisons,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", nargs="+", type=Path, required=True)
    parser.add_argument("--sample", type=int, default=100000)
    parser.add_argument("--seed", type=int, default=20261005)
    parser.add_argument("--pair", nargs=3, action="append", required=True,
                        metavar=("NAME", "BASELINE", "CANDIDATE"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        report = analyze(args.data, args.sample, args.seed, args.pair)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2, allow_nan=False)
                               + "\n", encoding="utf-8")
    except (OSError, ValueError) as error:
        parser.exit(1, f"error: {error}\n")


if __name__ == "__main__":
    main()
