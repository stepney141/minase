"""MNSD標本でPST/FMの手番依存と着手差を診断する。学習・探索は行わない。"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import subprocess

import numpy as np

from mnsd import map_records, read_header, write_mnsd
from taper import band_indices, phase_ratios, piece_counts


def statistics(values: list[float] | np.ndarray) -> dict:
    """空標本を明示し、符号付き平均と絶対値の裾を併記する。"""
    array = np.asarray(values, dtype=np.float64)
    if not array.size:
        return {"count": 0}
    return {
        "count": int(array.size), "mean": float(array.mean()),
        "std": float(array.std()), "mean_abs": float(np.abs(array).mean()),
        "p95_abs": float(np.quantile(np.abs(array), 0.95)),
        "max_abs": float(np.abs(array).max()),
    }


def probe(binary: Path, pst: Path, positions: Path, moves: bool) -> list[dict]:
    command = [str(binary.resolve()), "--pst", str(pst.resolve()),
               "--positions", str(positions.resolve())]
    if moves:
        command.append("--moves")
    return json.loads(subprocess.run(command, check=True, capture_output=True, text=True).stdout)


def sha256(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def static_summary(rows: list[dict]) -> dict:
    values = np.array([[row["eval_pst"], row["eval"]] for row in rows], dtype=np.int64)
    reverse = np.array([[row["eval_opposite_pst"], row["eval_opposite"]] for row in rows], dtype=np.int64)
    if not len(rows):
        return {"count": 0}
    result = {"count": len(rows), "fm_correction": statistics(values[:, 1] - values[:, 0])}
    for column, name in enumerate(("pst", "fm")):
        result[name] = {"eval": statistics(values[:, column]),
                        "turn_sum": statistics(values[:, column] + reverse[:, column])}
    result["correction_turn_sum"] = statistics(
        values[:, 1] + reverse[:, 1] - values[:, 0] - reverse[:, 0])
    return result


def move_summary(rows: list[dict]) -> dict:
    """着手後配置の固定視点差と手番交代を分離する。"""
    moves = [move for row in rows for move in row["moves"]]
    result = {"count": len(moves), "weighting": "one weight per legal move"}
    for suffix, name in (("_pst", "pst"), ("", "fm")):
        for move in moves:
            if move["delta" + suffix] != move["placement" + suffix] + move["turn" + suffix]:
                raise ValueError("move decomposition violates delta = placement + turn")
        result[name] = {key: statistics([move[key + suffix] for move in moves])
                        for key in ("delta", "placement", "turn")}
    result["correction"] = {
        key: statistics([move[key] - move[key + "_pst"] for move in moves])
        for key in ("delta", "placement", "turn")}
    result["position_weighted_correction"] = {
        key: {
            metric: statistics([
                statistics([move[key] - move[key + "_pst"] for move in row["moves"]])[metric]
                for row in rows if row["moves"]
            ]) for metric in ("mean", "mean_abs", "std", "max_abs")
        } for key in ("delta", "placement", "turn")
    }
    result["positions_with_moves"] = sum(bool(row["moves"]) for row in rows)
    return result


def grouped_moves(rows: list[dict]) -> dict:
    """非終局の静かな手、捕獲・成り手、終局手を排他的に分ける。"""
    result = move_summary(rows)
    result["by_kind"] = {}
    for name in ("quiet", "tactical", "terminal"):
        def belongs(move: dict) -> bool:
            if move["terminal"]:
                return name == "terminal"
            tactical = move["capture"] or move["promote"]
            return name == ("tactical" if tactical else "quiet")
        selected = [{"moves": [move for move in row["moves"] if belongs(move)]}
                    for row in rows]
        result["by_kind"][name] = move_summary(selected)
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--positions", required=True, type=Path)
    parser.add_argument("--probe", required=True, type=Path)
    parser.add_argument("--pst", required=True, type=Path)
    parser.add_argument("--output-dir", required=True, type=Path)
    parser.add_argument("--sample-size", required=True, type=int)
    parser.add_argument("--seed", required=True, type=int)
    parser.add_argument("--moves", action="store_true")
    args = parser.parse_args()
    source = map_records(args.positions)
    if args.sample_size < 1 or args.sample_size > len(source):
        parser.error("sample-size must be in 1..record_count")
    if args.seed < 0 or args.seed >= 2**64:
        parser.error("seed must be in 0..2**64-1")
    args.output_dir.mkdir(parents=True, exist_ok=False)
    indices = np.sort(np.random.default_rng(args.seed).choice(
        len(source), args.sample_size, replace=False))
    records = source[indices].copy()
    header = read_header(args.positions)
    sample_path = args.output_dir / "sample.bin"
    write_mnsd(sample_path, records, seed=header.seed, network_checksum=header.network_checksum,
               rule_set=header.rule_set, generation_commit=header.generation_commit,
               teacher_nodes=header.teacher_nodes)
    rows = probe(args.probe, args.pst, sample_path, args.moves)
    if len(rows) != len(records) or [row["index"] for row in rows] != list(range(len(records))):
        raise ValueError("probe record count mismatch")
    bands = band_indices(phase_ratios(records["board"]))
    report = {
        "inputs": {name: {"path": str(path.resolve()), "sha256": sha256(path)}
                   for name, path in (("positions", args.positions), ("probe", args.probe), ("pst", args.pst))},
        "sample_seed": args.seed, "source_record_count": len(source),
        "enumerate_moves": args.moves,
        "sample_indices": indices.tolist(),
        "definitions": {"turn_sum": "E_t(B) + E_opponent(B); not divided by 2",
                        "sampling": "uniform records without replacement; not independent games",
                        "flipped": "counterfactual perspective swap, not a legal null move",
                        "units": "cp"},
        "piece_count": statistics(piece_counts(records["board"])),
        "static": static_summary(rows),
        "bands": [],
    }
    for band in range(5):
        selected = np.flatnonzero(bands == band)
        report["bands"].append({"band": band, **static_summary(
            [rows[i] for i in selected])})
    if args.moves:
        report["moves"] = grouped_moves(rows)
        for band in range(5):
            report["bands"][band]["moves"] = grouped_moves(
                [rows[i] for i in np.flatnonzero(bands == band)])
    (args.output_dir / "probe.json").write_text(json.dumps(rows, indent=2) + "\n")
    (args.output_dir / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report["static"], indent=2))


if __name__ == "__main__":
    main()
