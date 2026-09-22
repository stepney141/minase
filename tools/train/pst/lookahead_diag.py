"""先読み教師の差の分布、教師K、露出標本、118列との残差相関を記録する。"""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path

import numpy as np
import torch

from features import feature_indices
from lookahead import validate_lookahead, window_statistics
from mnsd import Dataset, KingFeatures, sha256_file
from taper import phase_ratios
from train_pst import build_targets, estimate_generation_ks, make_model, model_logits, read_mnpt


def distribution(values: np.ndarray) -> dict:
    values = np.asarray(values, dtype=np.float64)
    if not values.size:
        return {"count": 0, "mean": None, "std": None,
                "quantiles": None, "reason": "empty population"}
    return {"count": int(values.size), "mean": float(values.mean()), "std": float(values.std()),
            "quantiles": dict(zip(("5", "25", "50", "75", "95"),
                                  np.percentile(values, [5, 25, 50, 75, 95]).tolist()))}


def difference_report(dataset: Dataset, plies: int) -> dict:
    """全訓練局面を等重みとし、分位点は線形補間、標準偏差は母標準偏差。"""
    differences, pieces, counts, first, fallback = [], [], [], [], []
    for file, rows in enumerate(dataset.records):
        selected = dataset.training_indices_by_file[file] - dataset.offsets[file]
        differences.append(dataset.lookahead_scores[file][selected] - rows["score"][selected].astype(np.float64))
        pieces.append(np.concatenate([
            np.count_nonzero(rows["board"][selected[start:start + 65536]], axis=1)
            for start in range(0, len(selected), 65536)
        ]) if len(selected) else np.empty(0, dtype=np.int64))
        stats = window_statistics(rows["game"], rows["ply"], plies)
        counts.append(stats["record_count"][selected])
        first.append(stats["first_record_plies"][selected][~stats["fallback"][selected]])
        fallback.append(stats["fallback"][selected])
    delta, pieces = np.concatenate(differences), np.concatenate(pieces)
    def summarize(values):
        return {**distribution(values), "sharp_drop_fraction": float(np.mean(values <= -300)) if len(values) else None}
    return {"difference_cp": summarize(delta),
            "piece_bands": {name: summarize(delta[mask]) for name, mask in (
                ("78以上", pieces >= 78), ("62〜77", (pieces >= 62) & (pieces <= 77)),
                ("47〜61", (pieces >= 47) & (pieces <= 61)), ("46以下", pieces <= 46))},
            "windows": {"record_count": distribution(np.concatenate(counts)),
                        "first_record_plies": distribution(np.concatenate(first)),
                        "first_record_population": "nonempty windows only",
                        "fallback_fraction": float(np.mean(np.concatenate(fallback)))}}


def sample_indices(dataset: Dataset, path: Path) -> tuple[np.ndarray, np.ndarray]:
    sample = json.loads(path.read_text(encoding="utf-8"))
    if sample["format"] != "depth-sensitivity-sample" or sample["version"] != 1:
        raise ValueError("unsupported exposed sample format/version")
    files = {source: index for index, source in enumerate(dataset.paths)}
    for item in sample["files"]:
        source = Path(item["file"]).resolve()
        if source not in files or item["sha256"] != sha256_file(source).hex():
            raise ValueError("exposed sample MNSD path or checksum mismatch")
    indices, groups = [], []
    for row in sample["positions"]:
        source = Path(row["file"]).resolve()
        if source not in files:
            raise ValueError("exposed sample file is not in data")
        file = files[source]
        index = row["index"]
        if type(index) is not int or not 0 <= index < dataset.headers[file].record_count:
            raise ValueError("exposed sample index outside file")
        if row["group"] not in ("exposed", "control"):
            raise ValueError("unknown exposed sample group")
        indices.append(int(dataset.offsets[file]) + index)
        groups.append(row["group"])
    if len(set(indices)) != len(indices):
        raise ValueError("duplicate exposed sample position")
    indices = np.asarray(indices, dtype=np.int64)
    if not np.all(np.isin(indices, dataset.training_indices)):
        raise ValueError("exposed sample contains positions outside training split")
    return indices, np.asarray(groups)


class ResidualCorrelation:
    """列ごとの中心化した積和を併合し、全局面の特徴を保持しない。"""

    def __init__(self):
        self.count = 0
        self.mean_x = np.zeros(118)
        self.mean_y = 0.0
        self.xx = np.zeros(118)
        self.yy = 0.0
        self.xy = np.zeros(118)

    def add(self, x, y):
        x = x.astype(np.float64)
        size = len(y)
        mx, my = x.mean(axis=0), y.mean()
        cx, cy = x - mx, y - my
        dx, dy = mx - self.mean_x, my - self.mean_y
        combined = self.count + size
        factor = self.count * size / combined
        self.xx += (cx * cx).sum(axis=0) + dx * dx * factor
        self.yy += float(cy @ cy) + dy * dy * factor
        self.xy += cx.T @ cy + dx * dy * factor
        self.mean_x += dx * size / combined
        self.mean_y += dy * size / combined
        self.count = combined

    def values(self):
        denominator = np.sqrt(self.xx * self.yy)
        return [float(np.clip(value / scale, -1, 1)) if scale > 0 else None
                for value, scale in zip(self.xy, denominator)]


def compare_teachers(current: Dataset, future: Dataset, features: KingFeatures,
                     pst: Path, output_k: float, sample: Path, batch: int) -> dict:
    current_k, counts = estimate_generation_ks(current, indices=current.training_indices)
    future_k, _ = estimate_generation_ks(future, indices=future.training_indices)
    selected, groups = sample_indices(current, sample)
    def targets(dataset, ks, indices, records):
        return build_targets(records, ks, dataset.generations(indices), 0.75,
                             scores=dataset.teacher_scores(indices))
    rows = current.gather(selected)
    change = targets(future, future_k, selected, rows).astype(np.float64) - targets(current, current_k, selected, rows)
    mg, eg, _, stored_k = read_mnpt(pst)
    model = make_model(torch.as_tensor(np.column_stack((mg, eg)).astype(np.float32) / 8),
                       torch.device("cpu"), "tapered")
    model.eval()
    stats = [ResidualCorrelation(), ResidualCorrelation()]
    indices = current.training_indices
    with torch.no_grad():
        for start in range(0, len(indices), batch):
            chunk = indices[start:start + batch]
            rows = current.gather(chunk)
            active = feature_indices(rows["board"], rows["stm"], rows["lion"])
            phi = phase_ratios(rows["board"]).astype(np.float32)
            prediction = torch.sigmoid(model_logits(model, torch.as_tensor(active), torch.as_tensor(phi), output_k)).numpy()
            values = features.gather(chunk)
            for stat, dataset, ks in zip(stats, (current, future), (current_k, future_k)):
                residual = targets(dataset, ks, chunk, rows).astype(np.float64) - prediction
                stat.add(values, residual)
    return {"teacher_k_comparison": [
                {"teacher_class": metadata, "lookahead_teacher_class": future.class_metadata()[i],
                 "training_positions": counts[i], "current_k": float(current_k[i]), "lookahead_k": float(future_k[i])}
                for i, metadata in enumerate(current.class_metadata())],
            "exposed_sample": {"path": str(sample), "sha256": sha256_file(sample).hex(),
                               "groups": {name: distribution(change[groups == name]) for name in ("exposed", "control")}},
            "residual_correlations": {"definition": "mixed teacher probability minus PST probability",
                                      "undefined_reason": "null means zero variance",
                                      "training_positions": len(indices),
                                      "columns": [{"index": i, "current": a, "lookahead": b}
                                                  for i, (a, b) in enumerate(zip(stats[0].values(), stats[1].values()))]},
            "pst_stored_k": stored_k}


def diagnose(data, king_features, pst: Path, output_k: float, gammas, plies: int,
             exposed_sample: Path, batch: int = 4096) -> dict:
    if not math.isfinite(output_k) or output_k <= 0 or batch <= 0:
        raise ValueError("output K and batch must be positive and finite")
    if not gammas or len(set(gammas)) != len(gammas) or 0.9 not in gammas:
        raise ValueError("gammas must be distinct and include 0.9 for teacher comparisons")
    for gamma in gammas:
        validate_lookahead(gamma, plies)
    current = Dataset(data)
    if not len(current.training_indices):
        raise ValueError("training split is empty")
    if np.any(current.teacher_lambdas != 0.75):
        raise ValueError("lookahead diagnostics require the basic teachers with lambda 0.75")
    features = KingFeatures(current, king_features)
    if features.column_count != 118 or features.definition_id != 2:
        raise ValueError("lookahead diagnostics require MNKF definition 2 with 118 columns")
    report = {"data": [str(p) for p in current.paths], "pst": str(pst), "output_k": output_k,
              "lambda": 0.75, "plies": plies, "comparison_gamma": 0.9,
              "training_positions": len(current.training_indices), "gammas": {}}
    for gamma in gammas:
        future = Dataset(data, lookahead={"gamma": gamma, "plies": plies})
        report["gammas"][str(gamma)] = difference_report(future, plies)
        if gamma == 0.9:
            report.update(compare_teachers(current, future, features, pst, output_k, exposed_sample, batch))
        del future
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", nargs="+", type=Path, required=True)
    parser.add_argument("--king-features", nargs="+", type=Path, required=True)
    parser.add_argument("--pst", type=Path, required=True)
    parser.add_argument("--output-k", type=float, required=True)
    parser.add_argument("--gammas", default="0.9,0.7,0.95")
    parser.add_argument("--plies", type=int, default=40)
    parser.add_argument("--exposed-sample", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = diagnose(args.data, args.king_features, args.pst, args.output_k,
                          [float(value) for value in args.gammas.split(",")], args.plies, args.exposed_sample)
        args.output.write_text(json.dumps(result, indent=2, ensure_ascii=False, allow_nan=False) + "\n", encoding="utf-8")
    except (ValueError, OSError) as error:
        parser.exit(1, f"error: {error}\n")


if __name__ == "__main__":
    main()
