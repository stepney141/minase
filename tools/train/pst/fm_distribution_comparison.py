"""局面帯×PST 500 cp刻みの共通層でFM補正を対局側の重みに標準化する。"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

import numpy as np

from fm_strength_diagnostics import sha256, statistics
from mnsd import map_records
from taper import band_indices, phase_ratios


def load(directory: Path) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    records = map_records(directory / "sample.bin")
    rows = json.loads((directory / "probe.json").read_text())
    if [row["index"] for row in rows] != list(range(len(records))):
        raise ValueError("probe indices must match sample order")
    pst = np.array([row["eval_pst"] for row in rows], dtype=np.int64)
    correction = np.array([row["eval"] - row["eval_pst"] for row in rows], dtype=np.int64)
    return band_indices(phase_ratios(records["board"])), pst, correction


def validate_evaluators(saved: Path, match: Path) -> dict:
    """同一重みと同一probeによる診断だけを比較する。"""
    reports = [json.loads((directory / "report.json").read_text())
               for directory in (saved, match)]
    checksums = {}
    for name in ("pst", "probe"):
        left, right = [report["inputs"][name]["sha256"] for report in reports]
        if left != right:
            raise ValueError(f"{name} checksum differs between saved and match diagnostics")
        checksums[name] = left
    return checksums


def distribution(correction: np.ndarray) -> dict:
    result = statistics(correction)
    result.update({"positive_count": int(np.sum(correction > 0)),
                   "zero_count": int(np.sum(correction == 0)),
                   "negative_count": int(np.sum(correction < 0)),
                   "positive_fraction": float(np.mean(correction > 0)),
                   "quantiles": {str(q): float(np.quantile(correction, q))
                                 for q in (0, 0.05, 0.25, 0.5, 0.75, 0.95, 1)}})
    return result


def compare(saved: tuple, match: tuple, center_only: bool) -> dict:
    strata = []
    counts = {}
    grouped = {}
    for name, (bands, pst, correction) in (("saved", saved), ("match", match)):
        selected = ((pst >= -1000) & (pst < 1000)) if center_only else np.ones(len(pst), dtype=bool)
        counts[name] = int(selected.sum())
        grouped[name] = {}
        for band, bucket in sorted(set(zip(bands[selected].tolist(), (pst[selected] // 500).tolist()))):
            mask = selected & (bands == band) & (pst // 500 == bucket)
            grouped[name][band, bucket] = correction[mask]
    common = sorted(set(grouped["saved"]) & set(grouped["match"]))
    if not common:
        raise ValueError("no common strata")
    for band, bucket in common:
        left, right = grouped["saved"][band, bucket], grouped["match"][band, bucket]
        strata.append({"band": band, "pst_lower_inclusive": bucket * 500,
                       "pst_upper_exclusive": (bucket + 1) * 500,
                       "saved_count": len(left), "match_count": len(right),
                       "saved_mean": float(left.mean()), "match_mean": float(right.mean()),
                       "difference": float(right.mean() - left.mean())})
    covered = {name: sum(len(grouped[name][key]) for key in common) for name in grouped}
    standardized = sum(row["saved_mean"] * row["match_count"] for row in strata) / covered["match"]
    observed = sum(row["match_mean"] * row["match_count"] for row in strata) / covered["match"]
    return {"pst_window": "[-1000,1000)" if center_only else "all",
            "source_counts": counts, "common_counts": covered,
            "coverage": {name: covered[name] / counts[name] for name in counts},
            "common_stratum_count": len(common),
            "saved_standardized_mean": standardized, "match_common_mean": observed,
            "difference": observed - standardized, "strata": strata}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--saved-dir", type=Path, required=True)
    parser.add_argument("--match-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    evaluators = validate_evaluators(args.saved_dir, args.match_dir)
    saved, match = load(args.saved_dir), load(args.match_dir)
    report = {
        "evaluator_checksums": evaluators,
        "inputs": {name: {file: {"path": str((directory / file).resolve()),
                                 "sha256": sha256(directory / file)}
                          for file in ("sample.bin", "probe.json")}
                   for name, directory in (("saved", args.saved_dir), ("match", args.match_dir))},
        "definitions": {"correction": "FM evaluation minus PST evaluation, side-to-move cp",
                        "strata": "5 piece-count phase bands x floor(PST / 500)",
                        "weighting": "match record frequency within common strata; no independent-game inference",
                        "limitation": "controls phase band and coarse PST only; no teacher accuracy claim"},
        "saved": distribution(saved[2]), "match": distribution(match[2]),
        "comparisons": [compare(saved, match, False), compare(saved, match, True)],
    }
    with args.output.open("x") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")
    print(json.dumps({"saved": report["saved"], "match": report["match"],
                      "comparisons": [{key: value for key, value in item.items() if key != "strata"}
                                      for item in report["comparisons"]]}, indent=2))


if __name__ == "__main__":
    main()
