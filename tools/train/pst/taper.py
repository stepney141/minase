"""盤上総駒数による補間係数、局面帯、および端点の識別性の集計。"""

from __future__ import annotations

import numpy as np
from numpy.typing import NDArray

from features import (
    BOARD_SQUARE_COUNT, FEATURE_COUNT, MIRRORED_FEATURE_COUNT, PADDING_INDEX,
    canonical_feature_indices, feature_indices,
)
from mnsd import Dataset

# 設計書「盤上総駒数で補間する」: q = min(90, max(0, N - 2)), φ = q / 90。
PHASE_DIVISOR = 90
PHASE_OFFSET = 2
BAND_COUNT = 5
# 設計書「局面帯別に診断する」: 偏差平方和が100未満の特徴は識別できない。
IDENTIFIABILITY_SSD_LIMIT = 100.0
# 識別できない特徴の出現割合がこの値を超えると待機中へ移る。
UNIDENTIFIABLE_SHARE_LIMIT = 0.05
BATCH = 1 << 18


def piece_counts(board: NDArray[np.uint8]) -> NDArray[np.int64]:
    """局面バッチの盤上総駒数を返す。先獅子特徴は含めない。"""
    board = np.asarray(board, dtype=np.uint8)
    if board.ndim != 2 or board.shape[1] != BOARD_SQUARE_COUNT:
        raise ValueError("board must have shape (B, 144)")
    return np.count_nonzero(board != 0, axis=1).astype(np.int64)


def phase_numerators(board: NDArray[np.uint8]) -> NDArray[np.int64]:
    """整数評価の分子に使う q = min(90, max(0, N - 2)) を返す。"""
    return np.clip(piece_counts(board) - PHASE_OFFSET, 0, PHASE_DIVISOR)


def phase_ratios(board: NDArray[np.uint8]) -> NDArray[np.float64]:
    """序中盤側の補間係数 φ = q / 90 を返す。"""
    return phase_numerators(board).astype(np.float64) / PHASE_DIVISOR


def band_indices(phi: NDArray[np.float64]) -> NDArray[np.int64]:
    """φを5等分帯へ割り当てる。左端を含み右端を含まず、最後の帯だけ1を含む。"""
    phi = np.asarray(phi, dtype=np.float64)
    if np.any(phi < 0.0) or np.any(phi > 1.0):
        raise ValueError("phase ratio must be in 0..1")
    return np.minimum(np.floor(phi * BAND_COUNT).astype(np.int64), BAND_COUNT - 1)


def band_label(band: int) -> str:
    """帯の区間を文字列で返す。"""
    lower = band / BAND_COUNT
    upper = (band + 1) / BAND_COUNT
    closing = "]" if band == BAND_COUNT - 1 else ")"
    return f"[{lower:.1f}, {upper:.1f}{closing}"


def feature_identifiability(
    dataset: Dataset, indices: NDArray[np.int64], *, mirrored: bool, batch: int = BATCH
) -> dict[str, NDArray]:
    """特徴ごとの出現回数、出現時のφの平均、および偏差平方和を集める。"""
    feature_count = MIRRORED_FEATURE_COUNT if mirrored else FEATURE_COUNT
    counts = np.zeros(feature_count, dtype=np.int64)
    phi_sums = np.zeros(feature_count, dtype=np.float64)
    phi_square_sums = np.zeros(feature_count, dtype=np.float64)
    for start in range(0, indices.size, batch):
        records = dataset.gather(indices[start : start + batch])
        features = feature_indices(records["board"], records["stm"], records["lion"])
        phi = phase_ratios(records["board"])
        active = features != PADDING_INDEX
        if mirrored:
            features = canonical_feature_indices(features)
        rows = np.broadcast_to(phi[:, None], features.shape)[active]
        flat = features[active]
        counts += np.bincount(flat, minlength=feature_count)
        phi_sums += np.bincount(flat, weights=rows, minlength=feature_count)
        phi_square_sums += np.bincount(flat, weights=rows * rows, minlength=feature_count)
    observed = counts > 0
    means = np.divide(phi_sums, counts, out=np.full(feature_count, np.nan), where=observed)
    ssd = np.where(observed, phi_square_sums - phi_sums * means, 0.0)
    return {"count": counts, "mean_phi": means, "ssd": np.maximum(ssd, 0.0)}


def identifiability_verdict(stats: dict[str, NDArray]) -> dict:
    """識別できない特徴の件数と出現割合を判定する。"""
    counts = stats["count"]
    unidentifiable = (counts > 0) & (stats["ssd"] < IDENTIFIABILITY_SSD_LIMIT)
    total = int(counts.sum())
    share = float(counts[unidentifiable].sum() / total) if total else float("nan")
    return {
        "ssd_limit": IDENTIFIABILITY_SSD_LIMIT,
        "share_limit": UNIDENTIFIABLE_SHARE_LIMIT,
        "observed_features": int(np.count_nonzero(counts > 0)),
        "unobserved_features": int(np.count_nonzero(counts == 0)),
        "unidentifiable_features": int(np.count_nonzero(unidentifiable)),
        "unidentifiable_occurrences": int(counts[unidentifiable].sum()),
        "total_occurrences": total,
        "unidentifiable_share": share,
        "proceed": bool(share <= UNIDENTIFIABLE_SHARE_LIMIT),
    }


def band_counts(dataset: Dataset, indices: NDArray[np.int64], batch: int = BATCH) -> NDArray[np.int64]:
    """世代と帯ごとの局面数を(世代数, 5)の表として返す。"""
    table = np.zeros((dataset.generation_count, BAND_COUNT), dtype=np.int64)
    for start in range(0, indices.size, batch):
        chunk = indices[start : start + batch]
        records = dataset.gather(chunk)
        bands = band_indices(phase_ratios(records["board"]))
        generations = dataset.generations(chunk)
        np.add.at(table, (generations, bands), 1)
    return table


def piece_count_histogram(dataset: Dataset, indices: NDArray[np.int64], batch: int = BATCH) -> NDArray[np.int64]:
    """総駒数(0..144)ごとの局面数を返す。"""
    histogram = np.zeros(BOARD_SQUARE_COUNT + 1, dtype=np.int64)
    for start in range(0, indices.size, batch):
        records = dataset.gather(indices[start : start + batch])
        histogram += np.bincount(piece_counts(records["board"]), minlength=BOARD_SQUARE_COUNT + 1)
    return histogram


def validation_bands(dataset: Dataset, batch: int = BATCH) -> NDArray[np.int64]:
    """検証集合の各レコードの帯番号を、`dataset.validation_indices`と同じ順序で返す。"""
    validation = dataset.validation_indices
    bands = np.empty(validation.size, dtype=np.int64)
    for start in range(0, validation.size, batch):
        records = dataset.gather(validation[start : start + batch])
        bands[start : start + batch] = band_indices(phase_ratios(records["board"]))
    return bands


def band_samples(dataset: Dataset, sample_size: int, seed: int) -> list[dict]:
    """世代と帯ごとに検証集合から最大sample_size局面をシードで抽出する。

    抽出は世代、帯の順に1つの乱数列で行うので、同じデータとシードなら同じ標本になる。
    空の帯は`indices`を空配列とし、呼出側が理由を記録する。
    """
    if sample_size <= 0:
        raise ValueError("diagnostic sample size must be positive")
    validation = dataset.validation_indices
    generations = dataset.generations(validation)
    bands = validation_bands(dataset)
    rng = np.random.default_rng(seed)
    samples = []
    for generation in range(dataset.generation_count):
        for band in range(BAND_COUNT):
            pool = validation[(generations == generation) & (bands == band)]
            chosen = rng.choice(pool, size=min(sample_size, pool.size), replace=False) if pool.size else pool
            samples.append({
                "generation": generation, "band": band, "pool_size": int(pool.size),
                "indices": np.sort(chosen.astype(np.int64)),
            })
    return samples
