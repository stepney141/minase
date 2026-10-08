"""MNPT v3の整数参照評価をRustのpst_probeと照合する。"""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path
from typing import Callable, Sequence

import numpy as np

from minase_train.checksum import sha256_file
from minase_train.data.features import feature_indices
from minase_train.data.mnpt import read_mnpt_v3, validate_fixed_base, validate_search_constants
from minase_train.data.mnsd import map_records
from minase_train.data.taper import phase_numerators
from minase_train.diagnostics.comparison import Weights, check_probe
from minase_train.fm.train import float_evaluate, integer_evaluate
from minase_train.pst.evaluate import integer_evaluate as pst_evaluate

Probe = Callable[[Path, Path, bool, bool], list[dict]]


class FMWeights:
    """FM候補のPSTと整数化した埋め込みを保持する。"""

    def __init__(self, path: Path) -> None:
        (self.middlegame, self.endgame, self.piece_values, self.k,
         self.u, self.signs, self.exponent) = read_mnpt_v3(path)

    def evaluate(self, records: np.ndarray) -> np.ndarray:
        features = feature_indices(records["board"], records["stm"], records["lion"])
        return integer_evaluate(self.middlegame, self.endgame, self.u, self.signs,
                                self.exponent, features, phase_numerators(records["board"]))

    def floating(self, records: np.ndarray, weights: dict) -> np.ndarray:
        features = feature_indices(records["board"], records["stm"], records["lion"])
        return float_evaluate(self.middlegame, self.endgame, self.k, weights["V"], weights["a"],
                              features, phase_numerators(records["board"]))


def rust_probe(command: Sequence[str]) -> Probe:
    """明示したコマンドへfm-evalと同じ--pst/--positions引数を渡す。"""
    if not command:
        raise ValueError("probe command must not be empty")

    def probe(mnpt: Path, mnsd: Path, promotions: bool, moves: bool) -> list[dict]:
        arguments = [*command, "--pst", str(mnpt), "--positions", str(mnsd)]
        if promotions:
            arguments.append("--promotions")
        if moves:
            arguments.append("--moves")
        return json.loads(subprocess.run(arguments, check=True, capture_output=True, text=True).stdout)

    return probe


def compare(base: Path, candidate: Path, positions: Path, probe: Probe, *, pst_changed: bool = False) -> dict:
    """全局面のFM込み評価とPST評価を照合し、欠落や並べ替えも拒否する。"""
    if pst_changed:
        validate_search_constants(base, candidate)
    else:
        validate_fixed_base(base, candidate)
    records = map_records(positions)
    if not len(records):
        raise ValueError("probe positions must be nonempty")
    model = FMWeights(candidate)
    rows = probe(candidate, positions, False, False)
    check_probe(model, records, rows, "FM candidate")
    if pst_changed:
        features = feature_indices(records["board"], records["stm"], records["lion"])
        expected_pst = pst_evaluate(model.middlegame, model.endgame, features,
                                   phase_numerators(records["board"]))
    else:
        expected_pst = Weights(base).evaluate(records)
    if not np.array_equal(expected_pst, [row["eval_pst"] for row in rows]):
        raise ValueError("candidate Rust eval_pst disagrees with the candidate PST")
    return {"samples": len(records), "candidate_pst": len(records),
            "base_sha256": sha256_file(base).hex(),
            "candidate_sha256": sha256_file(candidate).hex(),
            "positions_sha256": sha256_file(positions).hex()}


def main(arguments: Sequence[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pst-changed", action="store_true", help="同時学習した候補のPSTと照合する")
    parser.add_argument("--base", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--positions", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--probe-command", nargs="+", required=True,
                        help="例: target/release/minase dev pst-probe、またはpst_probeのパス")
    args = parser.parse_args(arguments)
    if args.output.exists():
        raise ValueError("diagnostic output already exists")
    report = compare(args.base, args.candidate, args.positions, rust_probe(args.probe_command),
                     pst_changed=args.pst_changed)
    report["probe_command"] = args.probe_command
    report["probe_sha256"] = sha256_file(Path(args.probe_command[0])).hex()
    args.output.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")


if __name__ == "__main__":
    main()
