"""MNUE版4の整数評価、分布、着手差分、および駒除去差分を診断する。"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
from typing import Callable, Sequence

import numpy as np

from minase_train.checksum import sha256_file
from minase_train.data.features import FEATURE_COUNT, PIECE_STATE_BY_BYTE, ROYAL_STATES, feature_indices
from minase_train.data.mnpt import initial_weights, read_mnpt_v3
from minase_train.data.mnsd import Dataset, map_records
from minase_train.data.taper import PHASE_DIVISOR, phase_numerators
from minase_train.fm.train import integer_evaluate as fm_evaluate
from minase_train.nnue.format import read_mnue
from minase_train.nnue.model import float_evaluate
from minase_train.nnue.reference import REFERENCE_BATCH, integer_evaluate
from minase_train.pst.evaluate import integer_evaluate as pst_evaluate


PHASE_INTERVALS = ((0, 29), (30, 59), (60, 90))
BASELINE_ABSOLUTE_INTERVALS = ((0, 99), (100, 299), (300, 999), (1000, None))
Probe = Callable[[Path, Path], list[dict]]


def rust_probe(command: Sequence[str], *, weight_option: str = "--net", moves: bool = False) -> Probe:
    """明示されたコマンドにネットと局面ファイルの引数を付けてJSONを受け取る。"""
    if not command:
        raise ValueError("probe command must not be empty")

    def probe(net: Path, positions: Path) -> list[dict]:
        arguments = [*command, weight_option, str(net), "--positions", str(positions)]
        if moves:
            arguments.append("--moves")
        return json.loads(subprocess.run(arguments, check=True, capture_output=True, text=True).stdout)

    return probe


def compare(net: Path, positions: Path, probe: Probe) -> dict:
    """件数、順序、整数評価を全件照合し、最初の不一致の添字と両値を報告する。"""
    network = read_mnue(net)
    records = map_records(positions)
    if not len(records):
        raise ValueError(f"{positions}: probe positions must be nonempty")
    rows = probe(net, positions)
    if not isinstance(rows, list):
        raise ValueError(f"{positions}: probe must return a JSON array")
    for start in range(0, len(records), REFERENCE_BATCH):
        chunk = records[start:start + REFERENCE_BATCH]
        values = integer_evaluate(network, chunk["board"], chunk["stm"], chunk["lion"])
        for index, expected in enumerate(values, start):
            if index >= len(rows):
                raise ValueError(f"{positions}: mismatch at index {index}: Python={int(expected)}, Rust=missing")
            row = rows[index]
            if (not isinstance(row, dict) or type(row.get("index")) is not int or row["index"] != index
                    or type(row.get("eval")) is not int or row["eval"] != int(expected)):
                raise ValueError(f"{positions}: mismatch at index {index}: Python={int(expected)}, Rust={row!r}")
    if len(rows) != len(records):
        raise ValueError(f"{positions}: mismatch at index {len(records)}: Python=missing, Rust={rows[len(records)]!r}")
    return {"samples": len(records), "net_sha256": sha256_file(net).hex(),
            "positions_sha256": sha256_file(positions).hex()}


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


def compare_scores(baseline: np.ndarray, candidate: np.ndarray) -> dict:
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
    """指定した閉区間ごとに、両評価の統計と組の指標を求める。"""
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
                **compare_scores(base_scores, candidate_scores),
            })
        except ValueError as error:
            raise ValueError(f"interval [{lower}, {upper}]: {error}") from error
    return reports


def file_identity(path: Path) -> dict:
    return {"path": str(path.resolve()), "sha256": sha256_file(path).hex()}


def sample_identity(dataset: Dataset, indices: np.ndarray, seed: int) -> dict:
    return {
        "sample_count": len(indices), "seed": seed,
        "sample_indices_sha256_le_i64": hashlib.sha256(indices.astype("<i8").tobytes()).hexdigest(),
        "input_files": [dict(file_identity(path), record_count=header.record_count)
                        for path, header in zip(dataset.paths, dataset.headers)],
    }


def distribution(data: list[Path], sample: int, seed: int, pairs: list[list[str]]) -> dict:
    """全局面から同じ一様非復元標本を取り、MNPT版3とMNUE版4を比較する。"""
    if sample < 2:
        raise ValueError("sample must be at least 2")
    names = [name for name, _, _ in pairs]
    if len(set(names)) != len(names):
        raise ValueError("pair names must be unique")
    dataset = Dataset(data)
    if sample > dataset.record_count:
        raise ValueError(f"sample {sample} exceeds {dataset.record_count} positions")
    indices = np.sort(np.random.default_rng(seed).choice(dataset.record_count, sample, replace=False))
    comparisons = []
    for name, base, candidate in pairs:
        base, candidate = Path(base), Path(candidate)
        mg, eg, _, _, embeddings, signs, exponent = read_mnpt_v3(base)
        net = read_mnue(candidate)
        base_scores = np.empty(sample, dtype=np.int32)
        candidate_scores = np.empty(sample, dtype=np.int32)
        phases = np.empty(sample, dtype=np.int64)
        for start in range(0, sample, REFERENCE_BATCH):
            chunk = slice(start, start + REFERENCE_BATCH)
            records = dataset.gather(indices[chunk])
            features = feature_indices(records["board"], records["stm"], records["lion"])
            phases[chunk] = phase_numerators(records["board"])
            base_scores[chunk] = fm_evaluate(mg, eg, embeddings, signs, exponent, features, phases[chunk])
            candidate_scores[chunk] = integer_evaluate(net, records["board"], records["stm"], records["lion"])
        metrics = compare_scores(base_scores, candidate_scores)
        comparisons.append({
            "name": name, "baseline": file_identity(base), "candidate": file_identity(candidate),
            "sample_count": sample, "baseline_statistics": summarize(base_scores),
            "candidate_statistics": summarize(candidate_scores), **metrics,
            "verdict": scale_verdict(metrics["std_ratio_candidate_over_baseline"]),
            "by_phase_q": compare_intervals(base_scores, candidate_scores, phases, PHASE_INTERVALS),
            "by_baseline_absolute_cp": compare_intervals(
                base_scores, candidate_scores, np.abs(base_scores), BASELINE_ABSOLUTE_INTERVALS),
        })
    return {
        **sample_identity(dataset, indices, seed), "population_count": dataset.record_count,
        "sampling": "sorted numpy.random.default_rng(seed).choice(total, sample, replace=False)",
        "evaluation_perspective": "side_to_move", "standard_deviation_ddof": 0,
        "percentile_method": "linear", "spearman_tie_method": "average",
        "regression_equation": "candidate = intercept + slope * baseline",
        "unchanged_scale_interval_inclusive": [0.9, 1.1], "pairs": comparisons,
    }


def delta_statistics(values: np.ndarray) -> dict:
    """母標準偏差を使い、標本が空なら定義できない統計をnullで表す。"""
    values = np.asarray(values, dtype=np.float64)
    return {"count": int(values.size),
            "mean_cp": float(values.mean()) if values.size else None,
            "std_cp": float(values.std()) if values.size else None,
            "mean_absolute_cp": float(np.abs(values).mean()) if values.size else None,
            "max_absolute_cp": float(np.abs(values).max()) if values.size else None}


def move_rows(rows: list[dict], count: int, *, baseline: bool) -> dict:
    """局面添字と着手文字列を検査し、重複や局面の欠落を拒否する。"""
    if not isinstance(rows, list):
        raise ValueError("move probe must return a JSON array")
    indexed = {}
    for row in rows:
        if (not isinstance(row, dict) or type(row.get("index")) is not int
                or not 0 <= row["index"] < count or row["index"] in indexed
                or not isinstance(row.get("moves"), list)):
            raise ValueError(f"invalid or duplicate move probe position: {row!r}")
        moves = {}
        for move in row["moves"]:
            fields = ("delta", "delta_pst") if baseline else ("delta",)
            if (not isinstance(move, dict) or not isinstance(move.get("move"), str)
                    or not move["move"] or move["move"] in moves
                    or any(type(move.get(field)) is not int for field in fields)):
                raise ValueError(f"invalid or duplicate move at index {row['index']}: {move!r}")
            moves[move["move"]] = move
        indexed[row["index"]] = moves
    if len(indexed) != count:
        raise ValueError(f"move probe returned {len(indexed)} positions; expected {count}")
    return indexed


def moves(net: Path, base: Path, positions: Path, probe: Probe, base_probe: Probe) -> dict:
    """同じ局面・着手を対応付け、手番を変換せず探査のdeltaを集計する。"""
    count = len(map_records(positions))
    if not count:
        raise ValueError("move probe positions must be nonempty")
    candidate = move_rows(probe(net, positions), count, baseline=False)
    baseline = move_rows(base_probe(base, positions), count, baseline=True)
    reports = []
    candidate_only = baseline_only = 0
    for index in range(count):
        cand, ref = candidate[index], baseline[index]
        common = sorted(cand.keys() & ref.keys())
        only_cand, only_base = len(cand.keys() - ref.keys()), len(ref.keys() - cand.keys())
        candidate_only += only_cand
        baseline_only += only_base
        cand_values = np.array([cand[move]["delta"] for move in common], dtype=np.float64)
        base_values = np.array([ref[move]["delta"] for move in common], dtype=np.float64)
        pst_values = np.array([ref[move]["delta_pst"] for move in common], dtype=np.float64)
        reports.append({
            "index": index, "matched_count": len(common),
            "candidate_only_count": only_cand, "baseline_only_count": only_base,
            "candidate": delta_statistics([row["delta"] for row in cand.values()]),
            "baseline": delta_statistics([row["delta"] for row in ref.values()]),
            "candidate_minus_baseline": delta_statistics(cand_values - base_values),
            "candidate_minus_baseline_pst": delta_statistics(cand_values - pst_values),
        })
    report = {"net": file_identity(net), "base": file_identity(base),
              "positions": file_identity(positions), "position_count": count,
              "evaluation_perspective": "side_to_move_before_move", "standard_deviation_ddof": 0,
              "candidate_only_count": candidate_only, "baseline_only_count": baseline_only,
              "by_position": reports}
    if candidate_only or baseline_only:
        report["error"] = (f"unmatched moves: candidate_only={candidate_only}, "
                           f"baseline_only={baseline_only}")
    return report


def float_removal_deltas(weights: dict, k: float, records: np.ndarray) -> np.ndarray:
    """両累算器から各駒の特徴を引き、補間率の変化を含めて倍精度で計算する。"""
    linear = np.zeros((FEATURE_COUNT + 1, 2), dtype=np.float64)
    linear[:FEATURE_COUNT] = np.stack((weights["middlegame"], weights["endgame"]), axis=1)
    table = np.zeros((FEATURE_COUNT + 1, weights["weights"].shape[1]), dtype=np.float64)
    table[:FEATURE_COUNT] = weights["weights"]
    features = [feature_indices(records["board"], side, records["lion"])
                for side in (records["stm"], 1 - records["stm"])]
    rows = linear[features[0]]
    sums = rows.sum(axis=1)
    counts = np.count_nonzero(records["board"], axis=1)
    q = np.clip(counts - 2, 0, PHASE_DIVISOR) / PHASE_DIVISOR
    after_q = np.clip(counts - 3, 0, PHASE_DIVISOR) / PHASE_DIVISOR
    before = q * sums[:, 0] + (1 - q) * sums[:, 1]
    remaining = sums[:, None, :] - rows[:, :144]
    delta = (after_q[:, None] * remaining[:, :, 0]
             + (1 - after_q[:, None]) * remaining[:, :, 1] - before[:, None])
    for perspective, indices in enumerate(features):
        rows = table[indices]
        hidden = rows.sum(axis=1) + weights["biases"]
        before = (np.clip(hidden, 0, 1)**2 * weights["output_weights"][perspective]).sum(axis=-1)
        after = (np.clip(hidden[:, None, :] - rows[:, :144], 0, 1)**2
                 * weights["output_weights"][perspective]).sum(axis=-1)
        delta += k * (after - before[:, None])
    return delta


def reversal_statistics(deltas: np.ndarray, reference: np.ndarray) -> dict:
    """参照差分が非0の除去だけを母数にし、参照と逆符号の整数差分を数える。"""
    eligible = reference != 0
    wrong = deltas[eligible & (deltas * np.sign(reference) < 0)]
    count = int(np.count_nonzero(eligible))
    return {"count": int(wrong.size), "total": count,
            "fraction": float(wrong.size / count) if count else None,
            "mean_absolute_cp": float(np.abs(wrong).mean()) if wrong.size else None,
            "max_absolute_cp": float(np.abs(wrong).max()) if wrong.size else None}


def removal(float_path: Path, net_path: Path, data: list[Path], sample: int, seed: int) -> dict:
    """検証集合の駒除去について独立再計算との誤差と逆向きを報告する。"""
    dataset = Dataset(data)
    if sample <= 0 or sample > len(dataset.validation_indices):
        raise ValueError(f"sample must be in 1..{len(dataset.validation_indices)} validation positions")
    indices = np.sort(np.random.default_rng(seed).choice(dataset.validation_indices, sample, replace=False))
    net = read_mnue(net_path)
    shapes = {"middlegame": (FEATURE_COUNT,), "endgame": (FEATURE_COUNT,),
              "weights": (FEATURE_COUNT, net.width), "biases": (net.width,),
              "output_weights": (2, net.width), "output_bias": ()}
    with np.load(float_path, allow_pickle=False) as archive:
        weights = {}
        for name, shape in shapes.items():
            if name not in archive:
                raise ValueError(f"{float_path}: missing {name}")
            value = archive[name]
            if value.shape != shape or value.dtype.kind != "f" or not np.isfinite(value).all():
                raise ValueError(f"{float_path}: invalid {name}, expected finite floating weights of shape {shape}")
            weights[name] = value.astype(np.float64)
    v0 = initial_weights()
    batch_errors, integer_errors, integer_deltas, references, states = [], [], [], [], []
    for index in indices:
        records = dataset.gather(np.array([index]))
        board_states = PIECE_STATE_BY_BYTE[records["board"][0]]
        squares = np.flatnonzero((records["board"][0] != 0) & ~np.isin(board_states, ROYAL_STATES))
        if not squares.size:
            continue
        batched = float_removal_deltas(weights, net.k, records)[0, squares]
        after = np.repeat(records, len(squares))
        after["board"][np.arange(len(squares)), squares] = 0
        floating, integer, reference = [], [], []
        for positions in (records, after):
            arguments = (positions["board"], positions["stm"], positions["lion"])
            floating.append(float_evaluate(weights, net.k, *arguments))
            integer.append(integer_evaluate(net, *arguments).astype(np.int64))
            reference.append(pst_evaluate(v0, v0, feature_indices(*arguments),
                                          phase_numerators(positions["board"])).astype(np.int64))
        float_delta = floating[1] - floating[0]
        integer_delta = integer[1] - integer[0]
        batch_errors.extend(batched - float_delta)
        integer_errors.extend(integer_delta - float_delta)
        integer_deltas.extend(integer_delta)
        references.extend(reference[1] - reference[0])
        states.extend(board_states[squares])
    if not states:
        raise ValueError("sample contains no non-royal removals")
    batch_errors, integer_errors = np.asarray(batch_errors), np.asarray(integer_errors)
    if not np.isfinite(batch_errors).all() or not np.isfinite(integer_errors).all():
        raise ValueError("non-finite removal difference")
    integer_deltas, references, states = map(np.asarray, (integer_deltas, references, states))
    maximum = float(np.abs(batch_errors).max())
    report = {
        **sample_identity(dataset, indices, seed), "split": "validation",
        "population_count": len(dataset.validation_indices),
        "float": file_identity(float_path), "net": file_identity(net_path),
        "removal_count": len(states), "reference": "v0 initial_weights at both endpoints",
        "float_batch_vs_recomputed": {"max_absolute_cp": maximum, "limit_cp": 1e-6,
                                      "passed": maximum <= 1e-6},
        "float_vs_integer_recomputed": {"mean_absolute_cp": float(np.abs(integer_errors).mean()),
                                        "max_absolute_cp": float(np.abs(integer_errors).max())},
        "reversals": reversal_statistics(integer_deltas, references),
        "by_piece_state": {str(int(state)): reversal_statistics(
            integer_deltas[states == state], references[states == state]) for state in np.unique(states)},
    }
    if maximum > 1e-6:
        report["error"] = f"float batched removal differs from recomputation by {maximum} cp (limit 1e-6)"
    return report


def probe_identity(command: Sequence[str], weight_option: str, weights: Path,
                   positions: Path, *, moves: bool) -> dict:
    executable = shutil.which(command[0])
    if executable is None:
        raise ValueError(f"{command[0]}: probe executable not found")
    arguments = [*command, weight_option, str(weights), "--positions", str(positions)]
    if moves:
        arguments.append("--moves")
    return {"probe_command": arguments, "probe_sha256": sha256_file(Path(executable)).hex()}


def main(arguments: Sequence[str] | None = None) -> None:
    """診断を選択し、不合格の診断も報告を保存してから例外にする。"""
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("compare", "distribution", "moves", "removal"):
        command = commands.add_parser(name)
        command.add_argument("--output", type=Path, required=True)
        if name in ("compare", "moves", "removal"):
            command.add_argument("--net", type=Path, required=True)
        if name in ("compare", "moves"):
            command.add_argument("--positions", type=Path, required=True)
            command.add_argument("--probe-command", nargs="+", required=True)
        if name in ("distribution", "removal"):
            command.add_argument("--data", nargs="+", type=Path, required=True)
            command.add_argument("--sample", type=int, required=True)
            command.add_argument("--seed", type=int, required=True)
        if name == "distribution":
            command.add_argument("--pair", nargs=3, action="append", required=True,
                                 metavar=("NAME", "BASELINE", "CANDIDATE"))
        if name == "moves":
            command.add_argument("--base", type=Path, required=True)
            command.add_argument("--base-probe-command", nargs="+", required=True)
        if name == "removal":
            command.add_argument("--float", dest="float_path", type=Path, required=True)
    args = parser.parse_args(arguments)
    if args.output.exists():
        raise ValueError(f"{args.output}: diagnostic output already exists")
    if args.command == "compare":
        identity = probe_identity(args.probe_command, "--net", args.net, args.positions, moves=False)
        report = compare(args.net, args.positions, rust_probe(args.probe_command))
        report.update(identity)
    elif args.command == "distribution":
        report = distribution(args.data, args.sample, args.seed, args.pair)
    elif args.command == "moves":
        identity = probe_identity(args.probe_command, "--net", args.net, args.positions, moves=True)
        base_identity = probe_identity(args.base_probe_command, "--pst", args.base, args.positions, moves=True)
        report = moves(args.net, args.base, args.positions,
                       rust_probe(args.probe_command, moves=True),
                       rust_probe(args.base_probe_command, weight_option="--pst", moves=True))
        report.update(identity)
        report.update({"base_" + key: value for key, value in base_identity.items()})
    else:
        report = removal(args.float_path, args.net, args.data, args.sample, args.seed)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2, ensure_ascii=False, allow_nan=False) + "\n",
                           encoding="utf-8")
    if "error" in report:
        raise ValueError(report["error"])


if __name__ == "__main__":
    main()
