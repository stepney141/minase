"""教師値の構成と勝率尺度Kの推定を行う。"""

from __future__ import annotations

import math

import numpy as np
from numpy.typing import NDArray

from minase_train.data.mnsd import Dataset


def binary_cross_entropy_sum(scores: np.ndarray, results: NDArray[np.uint8], k: float) -> float:
    """探索値から最終結果を予測する二値交差エントロピー合計を返す。"""
    logits = scores.astype(np.float64) / k
    targets = results.astype(np.float64) / 2.0
    return float(np.sum(np.logaddexp(0.0, logits) - targets * logits, dtype=np.float64))


def estimate_k(scores: np.ndarray, results: NDArray[np.uint8]) -> float:
    """区間50から4000で損失を最小化するKを黄金分割探索で求める。"""
    lower = 50.0
    upper = 4000.0
    ratio = (math.sqrt(5.0) - 1.0) / 2.0
    left = upper - ratio * (upper - lower)
    right = lower + ratio * (upper - lower)
    left_loss = binary_cross_entropy_sum(scores, results, left)
    right_loss = binary_cross_entropy_sum(scores, results, right)
    for _ in range(64):
        if left_loss <= right_loss:
            upper = right
            right = left
            right_loss = left_loss
            left = upper - ratio * (upper - lower)
            left_loss = binary_cross_entropy_sum(scores, results, left)
        else:
            lower = left
            left = right
            left_loss = right_loss
            right = lower + ratio * (upper - lower)
            right_loss = binary_cross_entropy_sum(scores, results, right)
    return (lower + upper) / 2.0


def _selected_indices(dataset: Dataset, indices: NDArray[np.int64]) -> NDArray[np.int64]:
    """呼出側が明示した局面集合を検証する。訓練・検証の分割は変更しない。"""
    indices = np.asarray(indices)
    if indices.ndim != 1 or indices.dtype.kind not in "iu" or indices.size == 0:
        raise ValueError("indices must be a nonempty one-dimensional integer array")
    if np.any(indices < 0) or np.any(indices >= dataset.record_count):
        raise ValueError("indices are outside the dataset")
    return indices.astype(np.int64, copy=False)


def _selected_scores_results(
    dataset: Dataset, indices: NDArray[np.int64]
) -> tuple[NDArray[np.float64], NDArray[np.uint8]]:
    """明示した局面集合から変換後の教師探索値と結果を集める。"""
    scores, results = [], []
    for start in range(0, indices.size, 65536):
        chunk = indices[start:start + 65536]
        records = dataset.gather(chunk)
        scores.append(dataset.teacher_scores(chunk))
        results.append(records["result"].copy())
    return np.concatenate(scores), np.concatenate(results)


def estimate_generation_ks(
    dataset: Dataset, *, indices: NDArray[np.int64]
) -> tuple[NDArray[np.float64], list[int]]:
    """教師の分類ごとに指定した訓練局面からKを推定する。λ=0はNaN。"""
    indices = _selected_indices(dataset, indices)
    values = np.full(dataset.generation_count, np.nan, dtype=np.float64)
    counts = []
    classes = dataset.generations(indices)
    for generation, mix in enumerate(dataset.teacher_lambdas):
        selected = indices[classes == generation]
        counts.append(int(selected.size))
        if mix == 0:
            continue
        if not selected.size:
            raise ValueError(f"teacher class {generation} has no selected training records")
        values[generation] = estimate_k(*_selected_scores_results(dataset, selected))
    return values, counts


def build_targets(
    records: np.ndarray,
    teacher_ks: NDArray[np.float64],
    generations: NDArray[np.int64],
    teacher_lambdas: NDArray[np.float64],
    *, scores: NDArray[np.float64] | None = None,
) -> NDArray[np.float32]:
    """分類ごとのλで混ぜ、λ=0のKと探索値は参照しない。"""
    teacher_ks = np.asarray(teacher_ks, dtype=np.float64)
    teacher_lambdas = np.broadcast_to(np.asarray(teacher_lambdas, dtype=np.float64), teacher_ks.shape)
    generations = np.asarray(generations, dtype=np.int64)
    if teacher_ks.ndim != 1 or teacher_lambdas.shape != teacher_ks.shape:
        raise ValueError("one lambda and K are required per teacher class")
    if np.any(~np.isfinite(teacher_lambdas)) or np.any((teacher_lambdas < 0) | (teacher_lambdas > 1)):
        raise ValueError("teacher lambdas must be finite and in 0..1")
    if generations.shape != (len(records),) or np.any(generations < 0) or np.any(generations >= teacher_ks.size):
        raise ValueError("invalid teacher class indices")
    mix = teacher_lambdas[generations]
    active = mix != 0
    scales = teacher_ks[generations[active]]
    if np.any(scales <= 0) or np.any(~np.isfinite(scales)):
        raise ValueError("active teacher K values must be finite and positive")
    scores = records["score"].astype(np.float64) if scores is None else np.asarray(scores, dtype=np.float64)
    if scores.shape != (len(records),) or np.any(~np.isfinite(scores[active])):
        raise ValueError("teacher scores must match records and be finite for active classes")
    targets = records["result"].astype(np.float64) / 2
    targets[active] = (mix[active] / (1 + np.exp(-scores[active] / scales))
                       + (1 - mix[active]) * targets[active])
    return targets.astype(np.float32)


def estimate_mixed_k(dataset: Dataset, *, indices: NDArray[np.int64]) -> float | None:
    """明示した全世代の局面集合から参考用の混合Kを求める。"""
    indices = _selected_indices(dataset, indices)
    classes = dataset.generations(indices)
    indices = indices[dataset.teacher_lambdas[classes] != 0]
    if not indices.size:
        return None
    return estimate_k(*_selected_scores_results(dataset, indices))
