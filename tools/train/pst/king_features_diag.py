"""段階9フェーズ0: Rustが書き出した王の安全度特徴を訓練分割で診断する。

典拠は docs/plans/strength-stage9.md の「着手時の診断」。定数列との相関と
未発火列の加重平均は未定義なのでJSONではnullとする。識別性の指標は0になる。
"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path

import numpy as np
import torch

from features import feature_indices
from mnsd import COLUMN_COUNT, DEFINITION_ID, Dataset, KingFeatures
from taper import phase_ratios
from train_pst import (
    build_targets, estimate_generation_ks, make_model, model_logits, parse_ranges, read_mnpt,
)

def column_names() -> list[str]:
    """定義ID 1の列順を、人が検査できる形で返す。"""
    names = []
    for side in ("stm", "opponent"):
        for state in ("attacking_pawn_only", "no_pawns", "one_slider", "multiple_sliders"):
            for distance in range(3):
                names.append(f"shelter.{side}.{state}.distance{distance}")
    for side in ("stm", "opponent"):
        for distance in (1, 2):
            for attackers in range(3):
                for defenders in range(3):
                    if attackers or defenders:
                        names.append(f"zone.{side}.distance{distance}.a{attackers}.d{defenders}")
        for count in ("one", "two", "three_or_more"):
            names.append(f"zone.{side}.attackers.{count}")
    for side in ("stm", "opponent"):
        for state in ("occupied_uncontrolled", "occupied_controlled", "unoccupied_controlled"):
            names.append(f"defenders.{side}.{state}")
    return names


class Statistics:
    """バッチの十分統計だけを保持する。相関は中心化した積和を併合する。"""

    def __init__(self, columns=range(COLUMN_COUNT)) -> None:
        self.columns = [int(column) for column in columns]
        column_count = len(self.columns)
        self.count = 0
        self.histogram = np.zeros((column_count, 256), dtype=np.int64)
        self.weight = np.zeros(column_count)
        self.phase_mean = np.zeros(column_count)
        self.phase_ssd = np.zeros(column_count)
        self.mean = np.zeros(column_count + 1)
        self.cross = np.zeros((column_count + 1, column_count + 1))
        self.two_royals = np.zeros(3, dtype=np.int64)

    def add(self, values: np.ndarray, phi: np.ndarray, residual: np.ndarray,
            records: np.ndarray) -> None:
        """特徴、補間係数、残差、および王駒数を同じバッチから集計する。"""
        size = len(values)
        if not size:
            return
        x = values.astype(np.float64)
        for column in range(len(self.columns)):
            self.histogram[column] += np.bincount(values[:, column], minlength=256)
        weights = x * x
        weight = weights.sum(axis=0)
        phase_mean = np.divide(
            weights.T @ phi, weight, out=np.zeros_like(weight), where=weight > 0,
        )
        phase_ssd = (weights * (phi[:, None] - phase_mean) ** 2).sum(axis=0)
        combined_weight = self.weight + weight
        delta = phase_mean - self.phase_mean
        fraction = np.divide(weight, combined_weight, out=np.zeros_like(weight),
                             where=combined_weight > 0)
        self.phase_ssd += phase_ssd + delta * delta * self.weight * fraction
        self.phase_mean += delta * fraction
        self.weight = combined_weight

        joined = np.column_stack((x, residual))
        mean = joined.mean(axis=0)
        centered = joined - mean
        delta = mean - self.mean
        combined_count = self.count + size
        self.cross += centered.T @ centered + np.outer(delta, delta) * self.count * size / combined_count
        self.mean += delta * size / combined_count
        self.count = combined_count

        # MNSDは色ごとに64を加算し、未成は1+駒種、成駒は30+現在の駒種。
        board = records["board"].astype(np.int16)
        kind = (board % 64 - 1) % 29
        royal = (board != 0) & ((kind == 11) | (kind == 21))
        friendly = (board >= 65) == (records["stm"][:, None] == 1)
        stm_two = np.count_nonzero(royal & friendly, axis=1) == 2
        opponent_two = np.count_nonzero(royal & ~friendly, axis=1) == 2
        self.two_royals += [np.count_nonzero(stm_two | opponent_two),
                            np.count_nonzero(stm_two), np.count_nonzero(opponent_two)]

    def report(self) -> dict:
        """相関が定義できない列をnullで明示したJSON互換の統計を返す。"""
        if self.count == 0:
            raise ValueError("training split is empty")
        variances = np.diag(self.cross)
        denominator = np.sqrt(np.outer(variances, variances))
        correlations = np.divide(self.cross, denominator,
                                 out=np.full_like(self.cross, np.nan), where=denominator > 0)
        # 浮動小数点の丸めで相関の絶対値が1を微小に超える場合だけ補正する。
        correlations = np.clip(correlations, -1.0, 1.0)
        nullable = lambda value: float(value) if np.isfinite(value) else None
        return {
            "training_positions": self.count,
            "columns": [
                {"index": source_column, "name": column_names()[source_column],
                 "activation_rate": 1.0 - int(self.histogram[column, 0]) / self.count,
                 "histogram": {str(value): int(count) for value, count in enumerate(self.histogram[column]) if count},
                 "phase_weighted_mean": float(self.phase_mean[column]) if self.weight[column] else None,
                 "identifiability_ssd": float(self.phase_ssd[column]),
                 "residual_correlation": nullable(correlations[column, -1])}
                for column, source_column in enumerate(self.columns)
            ],
            "two_royals_rates": dict(zip(
                ("either", "stm", "opponent"), (self.two_royals / self.count).tolist(),
            )),
            "correlation_matrix": [[nullable(value) for value in row]
                                   for row in correlations[:-1, :-1]],
        }


def diagnose(dataset: Dataset, features: KingFeatures, pst: Path, output_k: float,
             lambda_value: float = 0.75, batch: int = 4096) -> dict:
    """学習器の教師K、教師値、モデル推論を再利用してCPU上で診断する。"""
    if not math.isfinite(output_k) or output_k <= 0:
        raise ValueError("output K must be finite and positive")
    if not math.isfinite(lambda_value) or not 0 <= lambda_value <= 1:
        raise ValueError("lambda must be in 0..1")
    if batch <= 0:
        raise ValueError("batch size must be positive")
    indices = dataset.training_indices
    teacher_ks, generation_counts = estimate_generation_ks(dataset, indices=indices)
    mg, eg, _, stored_k = read_mnpt(pst)
    device = torch.device("cpu")
    initial = torch.as_tensor(np.column_stack((mg, eg)).astype(np.float32) / 8.0)
    model = make_model(initial, device, "tapered")
    model.eval()
    stats = Statistics(features.columns)
    with torch.no_grad():
        for start in range(0, indices.size, batch):
            selected = indices[start:start + batch]
            records = dataset.gather(selected)
            phi = phase_ratios(records["board"])
            active = feature_indices(records["board"], records["stm"], records["lion"])
            p = torch.sigmoid(model_logits(
                model, torch.as_tensor(active), torch.as_tensor(phi.astype(np.float32)), output_k,
            )).numpy()
            targets = build_targets(records, teacher_ks, dataset.generations(selected), lambda_value)
            stats.add(features.gather(selected), phi, targets.astype(np.float64) - p, records)
    return {
        "definition_id": DEFINITION_ID, "column_count": len(features.columns),
        "data": [str(path) for path in dataset.paths], "pst": str(pst),
        "output_k": output_k, "pst_stored_k": stored_k, "lambda": lambda_value,
        "teacher_ks": teacher_ks.tolist(), "generation_training_counts": generation_counts,
        "undefined_correlation": "null (zero variance)", **stats.report(),
    }


def main() -> None:
    """入力の対応と必須の出力Kを解析し、診断JSONを保存する。"""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", nargs="+", type=Path, required=True)
    parser.add_argument("--king-features", nargs="+", type=Path, required=True)
    parser.add_argument("--extra-columns", help="MNKF columns as half-open ranges, e.g. 0:24")
    parser.add_argument("--pst", type=Path, default=Path("nets/pst.bin"))
    parser.add_argument("--lambda", dest="lambda_value", type=float, default=0.75)
    parser.add_argument("--output-k", type=float, required=True)
    parser.add_argument("--batch", type=int, default=4096)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    dataset = Dataset(args.data)
    columns = None if args.extra_columns is None else parse_ranges(args.extra_columns, COLUMN_COUNT)
    features = KingFeatures(dataset, args.king_features, columns)
    result = diagnose(dataset, features, args.pst, args.output_k, args.lambda_value, args.batch)
    args.output.write_text(json.dumps(result, indent=2, allow_nan=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
