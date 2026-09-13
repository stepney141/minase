"""既存データの駒数分布、端点の識別性、および基準PSTの帯別診断を保存する。"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
from pathlib import Path

import numpy as np

from features import PIECE_STATE_COUNT, feature_indices
from mnsd import Dataset
from taper import (
    BAND_COUNT,
    PHASE_DIVISOR,
    PHASE_OFFSET,
    BATCH,
    band_counts,
    band_indices,
    band_label,
    feature_identifiability,
    identifiability_verdict,
    phase_numerators,
    phase_ratios,
    piece_count_histogram,
)
from train_pst import MODEL_KINDS, build_targets, estimate_generation_ks, integer_evaluate, read_mnpt


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def band_diagnostics(dataset: Dataset, middlegame: np.ndarray, endgame: np.ndarray, k: float, teacher_ks: np.ndarray,
                     lambda_value: float) -> list[dict]:
    """全検証局面について、世代と帯ごとの検証損失と教師誤差を返す。"""
    generation_count = dataset.generation_count
    shape = (generation_count, BAND_COUNT)
    counts = np.zeros(shape, dtype=np.int64)
    loss_sums = np.zeros(shape)
    scaled_error_sums = np.zeros(shape)
    raw_error_sums = np.zeros(shape)
    predicted_chunks: list[list[np.ndarray]] = [[[] for _ in range(BAND_COUNT)] for _ in range(generation_count)]
    teacher_chunks: list[list[np.ndarray]] = [[[] for _ in range(BAND_COUNT)] for _ in range(generation_count)]
    validation = dataset.validation_indices
    for start in range(0, validation.size, BATCH):
        chunk = validation[start : start + BATCH]
        records = dataset.gather(chunk)
        generations = dataset.generations(chunk)
        bands = band_indices(phase_ratios(records["board"]))
        features = feature_indices(records["board"], records["stm"], records["lion"])
        predicted = integer_evaluate(middlegame, endgame, features, phase_numerators(records["board"])).astype(np.float64)
        targets = build_targets(records, teacher_ks, generations, lambda_value).astype(np.float64)
        logits = predicted / k
        losses = np.logaddexp(0.0, logits) - targets * logits
        teacher = records["score"].astype(np.float64)
        scaled_teacher = teacher * (k / teacher_ks[generations])
        np.add.at(counts, (generations, bands), 1)
        np.add.at(loss_sums, (generations, bands), losses)
        np.add.at(scaled_error_sums, (generations, bands), np.abs(predicted - scaled_teacher))
        np.add.at(raw_error_sums, (generations, bands), np.abs(predicted - teacher))
        for generation in range(generation_count):
            for band in range(BAND_COUNT):
                selected = (generations == generation) & (bands == band)
                if np.any(selected):
                    predicted_chunks[generation][band].append(predicted[selected])
                    teacher_chunks[generation][band].append(scaled_teacher[selected])
    rows = []
    for generation in range(generation_count):
        for band in range(BAND_COUNT):
            count = int(counts[generation, band])
            row = {"generation": generation, "band": band, "range": band_label(band),
                   "validation_records": count}
            if count == 0:
                row.update({"validation_loss": None, "mae_scaled_cp": None, "mae_raw_cp": None,
                            "correlation": None, "reason": "empty band"})
            else:
                predicted = np.concatenate(predicted_chunks[generation][band])
                teacher = np.concatenate(teacher_chunks[generation][band])
                reason = None
                if count < 2:
                    reason = "fewer than 2 samples"
                elif np.ptp(teacher) == 0:
                    reason = "constant teacher scores"
                elif np.ptp(predicted) == 0:
                    reason = "constant predicted scores"
                row.update({
                    "validation_loss": float(loss_sums[generation, band] / count),
                    "mae_scaled_cp": float(scaled_error_sums[generation, band] / count),
                    "mae_raw_cp": float(raw_error_sums[generation, band] / count),
                    "correlation": None if reason else float(np.corrcoef(predicted, teacher)[0, 1]),
                    "reason": reason,
                })
            rows.append(row)
    return rows


def state_summary(stats: dict, squares: int) -> list[dict]:
    """駒状態と陣営ごとに出現回数、φの平均、および識別できない特徴数をまとめる。"""
    rows = []
    for relative_color in range(2):
        for state in range(PIECE_STATE_COUNT):
            start = (relative_color * PIECE_STATE_COUNT + state) * squares
            counts = stats["count"][start : start + squares]
            ssd = stats["ssd"][start : start + squares]
            observed = counts > 0
            total = int(counts.sum())
            rows.append({
                "relative_color": relative_color, "state": state, "occurrences": total,
                "observed_squares": int(observed.sum()),
                "mean_phi": float(np.average(stats["mean_phi"][start : start + squares][observed], weights=counts[observed])) if total else None,
                "min_ssd": float(ssd[observed].min()) if total else None,
                "unidentifiable_squares": int(np.count_nonzero(observed & (ssd < 100.0))),
                "unidentifiable_occurrences": int(counts[observed & (ssd < 100.0)].sum()),
            })
    lion = stats["count"][2 * PIECE_STATE_COUNT * squares :]
    lion_ssd = stats["ssd"][2 * PIECE_STATE_COUNT * squares :]
    observed = lion > 0
    rows.append({
        "relative_color": None, "state": "lion_square", "occurrences": int(lion.sum()),
        "observed_squares": int(observed.sum()),
        "mean_phi": float(np.average(stats["mean_phi"][2 * PIECE_STATE_COUNT * squares :][observed], weights=lion[observed])) if lion.sum() else None,
        "min_ssd": float(lion_ssd[observed].min()) if lion.sum() else None,
        "unidentifiable_squares": int(np.count_nonzero(observed & (lion_ssd < 100.0))),
        "unidentifiable_occurrences": int(lion[observed & (lion_ssd < 100.0)].sum()),
    })
    return rows


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", required=True, nargs="+")
    parser.add_argument("--model", choices=MODEL_KINDS, required=True)
    parser.add_argument("--pst", required=True, help="基準PST(MNPT)")
    parser.add_argument("--lambda", dest="lambda_value", type=float, default=0.75)
    parser.add_argument("--output-dir", required=True)
    arguments = parser.parse_args()
    output = Path(arguments.output_dir)
    output.mkdir(parents=True)
    dataset = Dataset(arguments.data)
    middlegame, endgame, _, k = read_mnpt(arguments.pst)
    teacher_ks, teacher_counts = estimate_generation_ks(dataset, indices=dataset.training_indices)
    training = dataset.training_indices
    validation = dataset.validation_indices
    mirrored = arguments.model == "mirrored"
    stats = feature_identifiability(dataset, training, mirrored=mirrored)
    training_histogram = piece_count_histogram(dataset, training)
    numerators = np.clip(np.arange(training_histogram.size) - PHASE_OFFSET, 0, PHASE_DIVISOR)
    mean_phi = float(np.dot(training_histogram, numerators) / (training.size * PHASE_DIVISOR))
    with (output / "features.csv").open("x", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(["feature", "count", "mean_phi", "ssd"])
        for feature in range(stats["count"].size):
            writer.writerow([feature, int(stats["count"][feature]),
                             "" if stats["count"][feature] == 0 else f"{stats['mean_phi'][feature]:.6f}",
                             f"{stats['ssd'][feature]:.3f}"])
    report = {
        "model": arguments.model,
        "training_mean_phi": mean_phi,
        "data": [{"path": str(Path(p).resolve()), "sha256": digest(Path(p)),
                  "seed": h.seed, "records": h.record_count, "generation": int(g)}
                 for p, h, g in zip(arguments.data, dataset.headers, dataset.file_generations)],
        "pst": {"path": str(Path(arguments.pst).resolve()), "sha256": digest(Path(arguments.pst)),
                "body_sha256": Path(arguments.pst).read_bytes()[48:80].hex(), "k": k},
        "validation_split": "hash64(seed, game) % 20 == 0 (mnsd.py)",
        "records": {"training": int(training.size), "validation": int(validation.size)},
        "teacher_ks": teacher_ks.tolist(), "teacher_training_records": teacher_counts,
        "piece_count_histogram": {
            "training": training_histogram.tolist(),
            "validation": piece_count_histogram(dataset, validation).tolist(),
        },
        "band_counts": {
            "labels": [band_label(band) for band in range(BAND_COUNT)],
            "training": band_counts(dataset, training).tolist(),
            "validation": band_counts(dataset, validation).tolist(),
        },
        "identifiability": identifiability_verdict(stats),
        "state_summary": state_summary(stats, 72 if mirrored else 144),
        "base_band_diagnostics": band_diagnostics(dataset, middlegame, endgame, k, teacher_ks, arguments.lambda_value),
    }
    with (output / "report.json").open("x") as stream:
        json.dump(report, stream, indent=2, ensure_ascii=False, allow_nan=False)
    print(json.dumps(report["identifiability"], indent=2))
    print(json.dumps(report["band_counts"], indent=2))


if __name__ == "__main__":
    main()
