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
    FEATURE_COUNT,
    COLOR_BY_BYTE,
    INITIAL_BOARD,
    PIECE_STATE_BY_BYTE,
    PIECE_STATE_COUNT,
    feature_indices,
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
    PAWN_STATE,
    REACHABLE_NON_ROYAL_STATES,
    ROYAL_STATES,
    build_targets,
    estimate_generation_ks,
    float_evaluate,
    integer_evaluate,
    read_mnpt,
    write_mnpt,
)

# (MNPTのパス, MNSDのパス, 成り手を列挙するか) を受け、Rustの評価結果をレコード順に返す。
Probe = Callable[[Path, Path, bool], list[dict]]


class Weights:
    """MNPTから読んだ両端点、駒価値、およびKを保持する。"""

    def __init__(self, path: Path) -> None:
        self.middlegame, self.endgame, self.piece_values, self.k = read_mnpt(path, feature_count=None)

    def evaluate(self, records: np.ndarray, extra: np.ndarray | None = None) -> NDArray[np.int32]:
        features = feature_indices(records["board"], records["stm"], records["lion"])
        return integer_evaluate(
            self.middlegame, self.endgame, features, phase_numerators(records["board"]), extra
        )

    def evaluate_pst(self, records: np.ndarray) -> NDArray[np.int32]:
        features = feature_indices(records["board"], records["stm"], records["lion"])
        return integer_evaluate(self.middlegame[:FEATURE_COUNT], self.endgame[:FEATURE_COUNT],
                                features, phase_numerators(records["board"]))

    def probe_features(self, rows: list[dict]) -> np.ndarray | None:
        columns = len(self.middlegame) - FEATURE_COUNT
        if not columns:
            if any("king_features" in row and len(row["king_features"]) != 0 for row in rows):
                raise ValueError("probe king_features column count does not match weights")
            return None
        values = np.asarray([row["king_features"] for row in rows], dtype=np.int64)
        if values.shape != (len(rows), columns):
            raise ValueError("probe king_features column count does not match weights")
        return values


def check_probe(model: Weights, records: np.ndarray, rows: list[dict], name: str) -> np.ndarray:
    if len(rows) != len(records) or any("skipped" in row for row in rows):
        raise ValueError(f"Rust probe omitted records for {name}")
    if [row["index"] for row in rows] != list(range(len(records))):
        raise ValueError(f"Rust probe record order differs for {name}")
    scores = model.evaluate(records, model.probe_features(rows))
    if not np.array_equal(scores, [row["eval"] for row in rows]):
        raise ValueError(f"Rust evaluation disagrees with the Python reference for {name}")
    return scores


def rust_probe(binary: Path) -> Probe:
    """`pst_probe`バイナリを呼ぶ探査関数を返す。"""

    def probe(mnpt: Path, mnsd: Path, promotions: bool) -> list[dict]:
        command = [str(binary), "--pst", str(mnpt), "--positions", str(mnsd), "--skip-invalid"]
        if promotions:
            command.append("--promotions")
        output = subprocess.run(command, check=True, capture_output=True, text=True).stdout
        return json.loads(output)

    return probe


def derived_piece_values(
    middlegame: NDArray[np.int16], endgame: NDArray[np.int16], mean_phi: float
) -> list[int]:
    """訓練局面の平均φで両端点を合成し、47状態の探索用駒価値を導く。"""
    if not np.isfinite(mean_phi) or not 0 <= mean_phi <= 1:
        raise ValueError("mean phase ratio must be finite and in 0..1")
    table = mean_phi * middlegame.astype(np.float64) + (1 - mean_phi) * endgame.astype(np.float64)
    values = []
    for state in range(PIECE_STATE_COUNT):
        own = table[state * 144 : (state + 1) * 144].sum()
        enemy = table[(PIECE_STATE_COUNT + state) * 144 : (PIECE_STATE_COUNT + state + 1) * 144].sum()
        numerator = own - enemy
        magnitude = int(np.floor(abs(numerator) / 2_304 + 0.5))
        values.append(magnitude if numerator >= 0 else -magnitude)
    royal = max(values[state] for state in REACHABLE_NON_ROYAL_STATES) + values[PAWN_STATE]
    for state in ROYAL_STATES:
        values[state] = royal
    return values


def _band_losses(
    dataset: Dataset,
    models: dict[str, Weights],
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
            logits = model.evaluate(records, dataset.gather_extra(chunk)).astype(np.float64) / model.k
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


def _removal_report(records: np.ndarray, models: dict[str, Weights]) -> list[dict]:
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
        scores = {name: model.evaluate_pst(variants) for name, model in models.items()}
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


def _total_removal_report(records: np.ndarray, reports: list[dict], models: dict[str, Weights],
                          paths: dict[str, Path], before: dict[str, np.ndarray],
                          probe: Probe, output_dir: Path) -> dict:
    """除去局面をRustで再構築し、全評価を照合して差分と除外理由を残す。"""
    variants, locations = [], []
    for position, entry in enumerate(reports):
        entry["total_evaluations"] = {name: int(scores[position]) for name, scores in before.items()}
        for removal in entry["removals"]:
            record = records[position].copy()
            record["board"][removal["square"]] = 0
            variants.append(record)
            locations.append((position, removal))
            removal["total_delta_cp"] = {}
            removal["skipped"] = {}
    if not variants:
        return {name: {"checked": 0, "skipped": 0} for name in models}
    variants = np.array(variants, dtype=RECORD_DTYPE)
    path = output_dir / "removals.bin"
    write_mnsd(path, variants, seed=0, network_checksum=bytes(32))
    agreement = {}
    for name, model in models.items():
        rows = probe(paths[name], path, False)
        if len(rows) != len(variants) or [r["index"] for r in rows] != list(range(len(variants))):
            raise ValueError("removal probe lost record indices")
        checked = skipped = 0
        for index, row in enumerate(rows):
            position, removal = locations[index]
            if "skipped" in row:
                removal["total_delta_cp"][name] = None
                removal["skipped"][name] = row["skipped"]
                skipped += 1
                continue
            score = model.evaluate(variants[index:index + 1], model.probe_features([row]))[0]
            if int(score) != row["eval"]:
                raise ValueError(f"Rust removal evaluation disagrees for {name}")
            removal["total_delta_cp"][name] = int(score) - int(before[name][position])
            checked += 1
        agreement[name] = {"checked": checked, "skipped": skipped}
    return agreement


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
) -> dict:
    """帯別の教師誤差、量子化誤差、駒の除去、および成りの診断を返す。

    output_dirは呼出側が作成した新規ディレクトリとする。
    候補の探索用駒価値は基準と一致していなければならない。
    """
    models = {"base": Weights(base_path), "candidate": Weights(candidate_path)}
    if not np.array_equal(models["base"].piece_values, models["candidate"].piece_values):
        raise ValueError("candidate piece values differ from the base")
    base, candidate = models["base"], models["candidate"]
    if len(base.middlegame) > len(candidate.middlegame):
        raise ValueError("base has more features than candidate")
    if len(base.middlegame) < len(candidate.middlegame):
        # 設計書の明示的な変換: 新しい列だけを0にし、候補バイナリで基準を評価する。
        padding = len(candidate.middlegame) - len(base.middlegame)
        base.middlegame = np.pad(base.middlegame, (0, padding))
        base.endgame = np.pad(base.endgame, (0, padding))
        base_path = output_dir / "diagnostic-base.bin"
        write_mnpt(base_path, base.middlegame, base.endgame, base.piece_values, base.k,
                   feature_count=len(base.middlegame))
    float_weights = np.load(candidate_float_path)
    if any(float_weights[key].shape != candidate.middlegame.shape for key in ("middlegame", "endgame")):
        raise ValueError("float weights do not match candidate feature count")
    teacher_ks, _ = estimate_generation_ks(dataset, indices=dataset.training_indices)
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
                entry[name] = _error_summary(model.evaluate(records, dataset.gather_extra(indices)).astype(np.float64), scaled, raw)
        report["bands"].append(entry)

    if not sample_chunks:
        raise ValueError("every band is empty")
    union = np.sort(np.concatenate(sample_chunks))
    union_records = dataset.gather(union)
    features = feature_indices(union_records["board"], union_records["stm"], union_records["lion"])
    union_path = output_dir / "diagnostic-samples.bin"
    write_mnsd(union_path, union_records, seed=0, network_checksum=base_path.read_bytes()[48:80])
    report["rust_agreement"] = {}
    for name, path in (("base", base_path), ("candidate", candidate_path)):
        probed = probe(path, union_path, False)
        checked = check_probe(models[name], union_records, probed, name)
        report["rust_agreement"][name] = len(checked)
        if name == "candidate":
            integer = checked
            extra = models[name].probe_features(probed)
            stored_extra = dataset.gather_extra(union)
            if extra is not None and not np.array_equal(extra, stored_extra):
                raise ValueError("MNKF selected columns disagree with candidate probe")
    floating = float_evaluate(
        float_weights["middlegame"], float_weights["endgame"], features,
        phase_ratios(union_records["board"]), extra,
    )
    errors = np.abs(floating - integer.astype(np.float32))
    report["quantization"] = {
        "samples": int(union.size),
        "mean_absolute_error_cp": float(errors.mean()),
        "max_absolute_error_cp": float(errors.max()),
        "limit_cp": QUANTIZATION_ERROR_LIMIT,
    }
    if not np.isfinite(errors.mean()) or errors.mean() > QUANTIZATION_ERROR_LIMIT:
        raise ValueError(f"quantization mean absolute error {errors.mean()} exceeds {QUANTIZATION_ERROR_LIMIT} cp")

    initial = np.zeros(1, dtype=RECORD_DTYPE)
    initial["board"] = INITIAL_BOARD
    initial["lion"] = NO_LION_SQUARE
    representative_indices = [representative_candidates[band] for band in sorted(representative_candidates)]
    representatives = np.concatenate([initial, dataset.gather(np.array(representative_indices, dtype=np.int64))])
    representative_path = output_dir / "representatives.bin"
    write_mnsd(representative_path, representatives, seed=0, network_checksum=base_path.read_bytes()[48:80])
    removal = _removal_report(representatives, models)
    report["pst_removal_sign_reversals"] = sum(
        item["delta_cp"]["base"] * item["delta_cp"]["candidate"] < 0
        for entry in removal for item in entry["removals"]
    )
    if report["pst_removal_sign_reversals"]:
        raise ValueError("PST removal delta reverses the baseline sign")
    promotions = {name: probe(path, representative_path, True) for name, path in (("base", base_path), ("candidate", candidate_path))}
    total_scores = {
        name: check_probe(model, representatives, promotions[name], name)
        for name, model in models.items()
    }
    report["rust_removal_agreement"] = _total_removal_report(
        representatives, removal, models, {"base": base_path, "candidate": candidate_path},
        total_scores, probe, output_dir,
    )
    labels = ["initial"] + [f"band{band}" for band in sorted(representative_candidates)]
    report["representatives"] = []
    report["rust_promotion_agreement"] = {name: 0 for name in models}
    for position, label in enumerate(labels):
        entry = {"label": label, "index": None if position == 0 else representative_indices[position - 1]}
        entry.update(removal[position])
        entry["promotions"] = {}
        for name in models:
            probed = promotions[name][position]
            moves = probed["promotions"]
            if moves:
                after = np.zeros(len(moves), dtype=RECORD_DTYPE)
                for field in ("board", "stm", "lion"):
                    after[field] = [move["after"][field] for move in moves]
                after_rows = [move["after"] for move in moves]
                after_scores = models[name].evaluate(after, models[name].probe_features(after_rows)).astype(np.int64)
                if any("eval" in row for row in after_rows) and not np.array_equal(
                    after_scores, [row["eval"] for row in after_rows]
                ):
                    raise ValueError(f"Rust promotion after evaluation disagrees for {name}")
                python_delta = -after_scores - total_scores[name][position]
                rust_delta = np.array([move["delta"] for move in moves], dtype=np.int64)
                if not np.array_equal(rust_delta, python_delta):
                    raise ValueError(f"Rust promotion delta disagrees with the Python reference for {name}")
                report["rust_promotion_agreement"][name] += len(moves)
            entry["promotions"][name] = probed["promotions"] or None
        entry["promotion_reason"] = None if promotions["base"][position]["promotions"] else "no legal promotion"
        report["representatives"].append(entry)
    report["derived_piece_values"] = {
        "states": list(REACHABLE_NON_ROYAL_STATES),
        "fixed": [int(models["base"].piece_values[s]) for s in REACHABLE_NON_ROYAL_STATES],
    }
    for name, model in models.items():
        for endpoint, phi in (("middlegame", 1.0), ("endgame", 0.0)):
            derived = derived_piece_values(model.middlegame, model.endgame, phi)
            report["derived_piece_values"][f"{name}_{endpoint}"] = [derived[s] for s in REACHABLE_NON_ROYAL_STATES]
    return report
