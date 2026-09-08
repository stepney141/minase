"""固定した帯別標本、駒の除去、および成りで、基準PSTと候補PSTを比較する。"""

from __future__ import annotations

import json
from pathlib import Path
import subprocess
from typing import Callable

import numpy as np
from numpy.typing import NDArray

from features import (
    BOARD_SQUARE_COUNT,
    COLOR_BY_BYTE,
    INITIAL_BOARD,
    PIECE_STATE_BY_BYTE,
    PIECE_STATE_COUNT,
    feature_indices,
)
from train_fm import (
    read_mnpt_v3, validate_fixed_base, float_evaluate as fm_float_evaluate,
    integer_evaluate as fm_integer_evaluate,
)
from mnsd import NO_LION_SQUARE, RECORD_DTYPE, Dataset, write_mnsd
from taper import (
    BAND_COUNT,
    BATCH,
    band_counts,
    band_indices,
    band_label,
    band_samples,
    phase_numerators,
    phase_ratios,
    piece_counts,
)
from train_pst import (
    QUANTIZATION_ERROR_LIMIT,
    REACHABLE_NON_ROYAL_STATES,
    ROYAL_STATES,
    build_targets,
    estimate_generation_ks,
    float_evaluate,
    integer_evaluate,
    read_mnpt,
)

# (MNPTのパス, MNSDのパス, 成り手を列挙するか) を受け、Rustの評価結果をレコード順に返す。
Probe = Callable[[Path, Path, bool], list[dict]]


class Weights:
    """MNPTから読んだ両端点、駒価値、およびKを保持する。"""

    def __init__(self, path: Path) -> None:
        self.middlegame, self.endgame, self.piece_values, self.k = read_mnpt(path)

    def evaluate(self, records: np.ndarray) -> NDArray[np.int32]:
        features = feature_indices(records["board"], records["stm"], records["lion"])
        return integer_evaluate(
            self.middlegame, self.endgame, features, phase_numerators(records["board"])
        )


class FMWeights:
    """第1フェーズの候補をPythonの整数参照評価で評価する。"""

    def __init__(self, path: Path) -> None:
        (self.middlegame, self.endgame, self.piece_values, self.k,
         self.u, self.signs, self.exponent) = read_mnpt_v3(path)

    def evaluate(self, records: np.ndarray) -> NDArray[np.int32]:
        features = feature_indices(records["board"], records["stm"], records["lion"])
        return fm_integer_evaluate(self.middlegame, self.endgame, self.u, self.signs,
                                   self.exponent, features, phase_numerators(records["board"]))

    def floating(self, records: np.ndarray, weights: dict) -> NDArray[np.float64]:
        """学習器の量子化前評価を、同じ特徴と固定PSTで計算する。"""
        features = feature_indices(records["board"], records["stm"], records["lion"])
        return fm_float_evaluate(self.middlegame, self.endgame, self.k, weights["V"], weights["a"],
                                 features, phase_numerators(records["board"]))


def rust_probe(binary: Path) -> Probe:
    """`pst_probe`バイナリを呼ぶ探査関数を返す。"""

    def probe(mnpt: Path, mnsd: Path, promotions: bool) -> list[dict]:
        command = [str(binary), "--pst", str(mnpt), "--positions", str(mnsd)]
        if promotions:
            command.append("--promotions")
        output = subprocess.run(command, check=True, capture_output=True, text=True).stdout
        return json.loads(output)

    return probe


def derived_piece_values(weights: NDArray[np.int16]) -> list[int]:
    """1端点の全升平均から、盤上に現れ得る非王駒の駒価値を導出する(0.5は0から遠ざける)。"""
    table = weights.astype(np.int64)
    values = []
    for state in range(PIECE_STATE_COUNT):
        own = int(table[state * 144 : (state + 1) * 144].sum())
        enemy = int(table[(PIECE_STATE_COUNT + state) * 144 : (PIECE_STATE_COUNT + state + 1) * 144].sum())
        numerator = own - enemy
        magnitude = (abs(numerator) + 1_152) // 2_304
        values.append(magnitude if numerator >= 0 else -magnitude)
    return values


def _band_losses(
    dataset: Dataset,
    models: dict[str, Weights | FMWeights],
    teacher_ks: NDArray[np.float64],
    lambda_value: float,
) -> dict[str, NDArray[np.float64]]:
    """全検証局面について、世代と帯ごとの検証損失をモデル別に返す。"""
    shape = (dataset.generation_count, BAND_COUNT)
    counts = np.zeros(shape, dtype=np.int64)
    sums = {name: np.zeros(shape) for name in models}
    validation = dataset.validation_indices
    for start in range(0, validation.size, BATCH):
        chunk = validation[start : start + BATCH]
        records = dataset.gather(chunk)
        generations = dataset.generations(chunk)
        bands = band_indices(phase_ratios(records["board"]))
        targets = build_targets(records, teacher_ks, generations, lambda_value).astype(np.float64)
        np.add.at(counts, (generations, bands), 1)
        for name, model in models.items():
            logits = model.evaluate(records).astype(np.float64) / model.k
            losses = np.logaddexp(0.0, logits) - targets * logits
            np.add.at(sums[name], (generations, bands), losses)
    result = {}
    for name in models:
        result[name] = np.divide(sums[name], counts, out=np.full(shape, np.nan), where=counts != 0)
    return result


def _error_summary(predicted: NDArray, scaled: NDArray, raw: NDArray) -> dict:
    """標本の平均絶対誤差(換算と生)と相関を返し、定義できない相関には理由を付ける。"""
    reason = None
    if predicted.size < 2:
        reason = "fewer than 2 samples"
    elif np.ptp(scaled) == 0:
        reason = "constant teacher scores"
    elif np.ptp(predicted) == 0:
        reason = "constant predicted scores"
    return {
        "mae_scaled_cp": float(np.abs(predicted - scaled).mean()),
        "mae_raw_cp": float(np.abs(predicted - raw).mean()),
        "correlation": None if reason else float(np.corrcoef(predicted, scaled)[0, 1]),
        "correlation_reason": reason,
    }


def _removal_report(records: np.ndarray, models: dict[str, Weights | FMWeights]) -> list[dict]:
    """各局面の非王駒を1枚ずつ除いた評価変化をモデル別に返す。"""
    reports = []
    for record in records:
        board = record["board"]
        squares = [
            int(square)
            for square in np.flatnonzero(board != 0)
            if int(PIECE_STATE_BY_BYTE[board[square]]) not in ROYAL_STATES
        ]
        boards = np.repeat(board[None, :], len(squares) + 1, axis=0)
        boards[np.arange(1, len(squares) + 1), squares] = 0
        variants = np.zeros(len(boards), dtype=RECORD_DTYPE)
        variants["board"] = boards
        variants["stm"] = record["stm"]
        variants["lion"] = record["lion"]
        variants["kirin"] = record["kirin"]
        counts = piece_counts(boards)
        entry = {
            "piece_count": int(counts[0]),
            "phase_numerator": int(phase_numerators(board[None, :])[0]),
            "evaluations": {},
            "removals": [],
        }
        scores = {name: model.evaluate(variants) for name, model in models.items()}
        for name in models:
            entry["evaluations"][name] = int(scores[name][0])
        for offset, square in enumerate(squares, start=1):
            entry["removals"].append({
                "square": square,
                "piece_byte": int(board[square]),
                "relative_color": int(COLOR_BY_BYTE[board[square]] != record["stm"]),
                "state": int(PIECE_STATE_BY_BYTE[board[square]]),
                "delta_cp": {name: int(scores[name][offset] - scores[name][0]) for name in models},
            })
        reports.append(entry)
    return reports


def diagnose(
    dataset: Dataset,
    base_path: Path,
    candidate_path: Path,
    candidate_float_path: Path,
    output_dir: Path,
    sample_size: int,
    seed: int,
    lambda_value: float,
    probe: Probe,
    *,
    model_kind: str,
) -> dict:
    """帯別の教師誤差、量子化誤差、駒の除去、および成りの診断を返す。

    output_dirは呼出側が作成した新規ディレクトリとする。
    候補の探索用駒価値は基準と一致していなければならない。
    """
    if model_kind not in ("single", "tapered", "fm"):
        raise ValueError("unknown diagnostic model kind")
    is_fm = model_kind == "fm"
    if is_fm:
        validate_fixed_base(base_path, candidate_path)
    models = {"base": Weights(base_path),
              "candidate": FMWeights(candidate_path) if is_fm else Weights(candidate_path)}
    if not np.array_equal(models["base"].piece_values, models["candidate"].piece_values):
        raise ValueError("candidate piece values differ from the base")
    with np.load(candidate_float_path, allow_pickle=False) as stored:
        float_weights = {name: stored[name] for name in stored.files}
    if is_fm:
        rank = models["candidate"].u.shape[1]
        if (set(float_weights) != {"V", "a", "mask"}
                or float_weights["V"].shape != models["candidate"].u.shape
                or float_weights["a"].shape != (rank,)
                or float_weights["mask"].shape != (len(models["candidate"].u),)
                or float_weights["mask"].dtype != np.bool_
                or not np.isfinite(float_weights["V"]).all()
                or not np.isfinite(float_weights["a"]).all()
                or np.any(float_weights["V"][~float_weights["mask"]] != 0)):
            raise ValueError("invalid floating FM weights or observation mask")
    teacher_ks, _ = estimate_generation_ks(dataset)
    samples = band_samples(dataset, sample_size, seed)
    losses = _band_losses(dataset, models, teacher_ks, lambda_value)
    training_counts = band_counts(dataset, dataset.training_indices)
    validation_counts = band_counts(dataset, dataset.validation_indices)

    report = {
        "sample_size": sample_size,
        "seed": seed,
        "lambda": lambda_value,
        "k": {name: model.k for name, model in models.items()},
        "teacher_ks": teacher_ks.tolist(),
        "generation_checksums": [c.hex() for c in dataset.generation_checksums],
        "piece_values": models["base"].piece_values.tolist(),
        "bands": [],
    }
    sample_chunks: list[np.ndarray] = []
    representative_candidates: dict[int, int] = {}
    for sample in samples:
        generation, band, indices = sample["generation"], sample["band"], sample["indices"]
        filename = f"diagnostic-indices-generation{generation}-band{band}.npy"
        with (output_dir / filename).open("xb") as stream:
            np.save(stream, indices, allow_pickle=False)
        entry = {
            "generation": generation,
            "band": band,
            "range": band_label(band),
            "training_records": int(training_counts[generation, band]),
            "validation_records": int(validation_counts[generation, band]),
            "samples": int(indices.size),
            "indices_file": filename,
            "validation_loss": {
                name: (None if validation_counts[generation, band] == 0 else float(losses[name][generation, band]))
                for name in models
            },
        }
        if indices.size == 0:
            entry["reason"] = "empty band"
            for name in models:
                entry[name] = None
        else:
            records = dataset.gather(indices)
            sample_chunks.append(indices)
            representative_candidates[band] = min(
                representative_candidates.get(band, int(indices[0])), int(indices[0])
            )
            raw = records["score"].astype(np.float64)
            scaled = raw * (models["candidate"].k / teacher_ks[generation])
            for name, model in models.items():
                entry[name] = _error_summary(model.evaluate(records).astype(np.float64), scaled, raw)
        report["bands"].append(entry)

    if not sample_chunks:
        raise ValueError("every band is empty")
    union = np.sort(np.concatenate(sample_chunks))
    union_records = dataset.gather(union)
    features = feature_indices(union_records["board"], union_records["stm"], union_records["lion"])
    if is_fm:
        floating = models["candidate"].floating(union_records, float_weights)
    else:
        floating = float_evaluate(
            float_weights["middlegame"], float_weights["endgame"], features, phase_ratios(union_records["board"])
        )
    integer = models["candidate"].evaluate(union_records)
    errors = np.abs(floating - integer.astype(np.float32))
    report["quantization"] = {
        "samples": int(union.size),
        "mean_absolute_error_cp": float(errors.mean()),
        "max_absolute_error_cp": float(errors.max()),
        "limit_cp": QUANTIZATION_ERROR_LIMIT,
    }
    if not np.isfinite(errors.mean()) or errors.mean() > QUANTIZATION_ERROR_LIMIT:
        raise ValueError(f"quantization mean absolute error {errors.mean()} exceeds {QUANTIZATION_ERROR_LIMIT} cp")

    # Rustの評価がPythonの整数参照評価と全標本で一致することを確かめる。
    union_path = output_dir / "diagnostic-samples.bin"
    write_mnsd(union_path, union_records, seed=0, network_checksum=base_path.read_bytes()[48:80])
    report["rust_agreement"] = {}
    probe_paths = [("base", base_path)] if is_fm else [("base", base_path), ("candidate", candidate_path)]
    if is_fm:
        report["rust_agreement"]["candidate"] = {"status": "未実施（第2フェーズ）"}
        report["move_deltas"] = {"status": "未実施（第2フェーズ）"}
        report["candidate_evaluator"] = "Python MNPT v3 integer reference"
    for name, path in probe_paths:
        probed = probe(path, union_path, False)
        rust = np.array([item["eval"] for item in probed], dtype=np.int64)
        python = models[name].evaluate(union_records).astype(np.int64)
        if rust.shape != python.shape or not np.array_equal(rust, python):
            raise ValueError(f"Rust evaluation disagrees with the Python reference for {name}")
        report["rust_agreement"][name] = int(rust.size)

    initial = np.zeros(1, dtype=RECORD_DTYPE)
    initial["board"] = INITIAL_BOARD
    initial["lion"] = NO_LION_SQUARE
    representative_indices = [representative_candidates[band] for band in sorted(representative_candidates)]
    representatives = np.concatenate([initial, dataset.gather(np.array(representative_indices, dtype=np.int64))])
    representative_path = output_dir / "representatives.bin"
    write_mnsd(representative_path, representatives, seed=0, network_checksum=base_path.read_bytes()[48:80])
    removal = _removal_report(representatives, models)
    promotions = {name: probe(path, representative_path, True) for name, path in probe_paths}
    labels = ["initial"] + [f"band{band}" for band in sorted(representative_candidates)]
    report["representatives"] = []
    for position, label in enumerate(labels):
        entry = {"label": label, "index": None if position == 0 else representative_indices[position - 1]}
        entry.update(removal[position])
        entry["promotions"] = {}
        for name, _ in probe_paths:
            probed = promotions[name][position]
            if probed["eval"] != entry["evaluations"][name]:
                raise ValueError(f"Rust evaluation disagrees with the Python reference for {name}")
            entry["promotions"][name] = probed["promotions"] or None
        entry["promotion_reason"] = None if promotions["base"][position]["promotions"] else "no legal promotion"
        if is_fm:
            entry["promotions"]["candidate"] = {"status": "未実施（第2フェーズ）"}
            entry["fm_correction_cp"] = entry["evaluations"]["candidate"] - entry["evaluations"]["base"]
            for removal_entry in entry["removals"]:
                delta = removal_entry["delta_cp"]
                removal_entry["fm_delta_cp"] = delta["candidate"] - delta["base"]
        report["representatives"].append(entry)
    report["derived_piece_values"] = {
        "states": list(REACHABLE_NON_ROYAL_STATES),
        "fixed": [int(models["base"].piece_values[s]) for s in REACHABLE_NON_ROYAL_STATES],
    }
    for name, model in models.items():
        for endpoint, weights in (("middlegame", model.middlegame), ("endgame", model.endgame)):
            derived = derived_piece_values(weights)
            report["derived_piece_values"][f"{name}_{endpoint}"] = [derived[s] for s in REACHABLE_NON_ROYAL_STATES]
    return report
