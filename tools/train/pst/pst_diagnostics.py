"""固定した検証標本と駒の除去で、基準PSTと候補PSTを比較する。"""

from __future__ import annotations

from pathlib import Path

import numpy as np

from features import INITIAL_BOARD, feature_indices
from mnsd import Dataset
from train_pst import integer_evaluate, read_mnpt


def diagnose(
    dataset: Dataset,
    base_path: Path,
    candidate_path: Path,
    output_dir: Path,
    sample_size: int,
    seed: int,
) -> dict:
    """世代別の教師誤差と静的な駒価値を返し、標本の大域番号を保存する。

    output_dirは呼出側が作成した新規ディレクトリとする。
    相関が定義できない標本でも平均絶対誤差は報告する。
    """
    if sample_size <= 0:
        raise ValueError("diagnostic sample size must be positive")
    validation = dataset.validation_indices
    generations = dataset.generations(validation)
    pools = [
        validation[generations == generation]
        for generation in range(dataset.generation_count)
    ]
    for generation, pool in enumerate(pools):
        if pool.size == 0:
            raise ValueError(f"generation {generation} has no validation records")
    weights = {
        "base": read_mnpt(base_path)[0],
        "candidate": read_mnpt(candidate_path)[0],
    }
    report = {"sample_size": sample_size, "seed": seed, "generations": []}
    rng = np.random.default_rng(seed)
    for generation, pool in enumerate(pools):
        indices = rng.choice(pool, size=min(sample_size, pool.size), replace=False)
        filename = f"diagnostic-indices-generation{generation}.npy"
        with (output_dir / filename).open("xb") as stream:
            np.save(stream, indices, allow_pickle=False)
        records = dataset.gather(indices)
        features = feature_indices(records["board"], records["stm"], records["lion"])
        teacher = records["score"].astype(np.float64)
        result = {
            "generation": generation,
            "network_checksum": dataset.generation_checksums[generation].hex(),
            "validation_records": int(pool.size),
            "samples": int(indices.size),
            "indices_file": filename,
        }
        for name, table in weights.items():
            predicted = integer_evaluate(table, features).astype(np.float64)
            reason = None
            if indices.size < 2:
                reason = "fewer than 2 samples"
            elif np.ptp(teacher) == 0:
                reason = "constant teacher scores"
            elif np.ptp(predicted) == 0:
                reason = "constant predicted scores"
            result[name] = {
                "mae_cp": float(np.abs(predicted - teacher).mean()),
                "correlation": (
                    float(np.corrcoef(predicted, teacher)[0, 1])
                    if reason is None else None
                ),
                "correlation_reason": reason,
            }
        report["generations"].append(result)

    squares = np.flatnonzero((INITIAL_BOARD > 0) & (INITIAL_BOARD < 65))
    boards = np.repeat(INITIAL_BOARD[None, :], len(squares) + 1, axis=0)
    boards[np.arange(1, len(squares) + 1), squares] = 0
    features = feature_indices(
        boards,
        np.zeros(len(boards), dtype=np.uint8),
        np.full(len(boards), 255, dtype=np.uint8),
    )
    report["material"] = {}
    for name, table in weights.items():
        scores = integer_evaluate(table, features)
        report["material"][name] = {
            "initial_cp": int(scores[0]),
            "removals": [
                {
                    "square": int(square),
                    "piece_byte": int(INITIAL_BOARD[square]),
                    "delta_cp": int(delta),
                }
                for square, delta in zip(squares, scores[1:] - scores[0])
            ],
        }
    return report
