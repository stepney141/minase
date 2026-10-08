"""MNUE版4の全局面の整数参照評価をRustの探査コマンドと照合する。"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import shutil
import subprocess
from typing import Callable, Sequence

from minase_train.checksum import sha256_file
from minase_train.data.mnsd import map_records
from minase_train.nnue.format import read_mnue
from minase_train.nnue.reference import REFERENCE_BATCH, integer_evaluate


Probe = Callable[[Path, Path], list[dict]]


def rust_probe(command: Sequence[str]) -> Probe:
    """明示されたコマンドにネットと局面ファイルの引数を付けてJSONを受け取る。"""
    if not command:
        raise ValueError("probe command must not be empty")

    def probe(net: Path, positions: Path) -> list[dict]:
        arguments = [*command, "--net", str(net), "--positions", str(positions)]
        return json.loads(subprocess.run(arguments, check=True, capture_output=True, text=True).stdout)

    return probe


def compare(net: Path, positions: Path, probe: Probe) -> dict:
    """件数、順序、整数評価を全件照合し、最初の不一致の添字と両値を報告する。"""
    network = read_mnue(net)
    records = map_records(positions)
    if not len(records):
        raise ValueError(f"{positions}: probe positions must be nonempty")
    rows = probe(net, positions)
    if not isinstance(rows, list):
        raise ValueError(f"{positions}: probe must return a JSON array")
    for start in range(0, len(records), REFERENCE_BATCH):
        chunk = records[start:start + REFERENCE_BATCH]
        values = integer_evaluate(network, chunk["board"], chunk["stm"], chunk["lion"])
        for index, expected in enumerate(values, start):
            if index >= len(rows):
                raise ValueError(f"{positions}: mismatch at index {index}: Python={int(expected)}, Rust=missing")
            row = rows[index]
            if (not isinstance(row, dict) or type(row.get("index")) is not int or row["index"] != index
                    or type(row.get("eval")) is not int or row["eval"] != int(expected)):
                raise ValueError(f"{positions}: mismatch at index {index}: Python={int(expected)}, Rust={row!r}")
    if len(rows) != len(records):
        raise ValueError(f"{positions}: mismatch at index {len(records)}: Python=missing, Rust={rows[len(records)]!r}")
    return {"samples": len(records), "net_sha256": sha256_file(net).hex(),
            "positions_sha256": sha256_file(positions).hex()}


def main(arguments: Sequence[str] | None = None) -> None:
    """照合結果と実行コマンドおよびバイナリの検査和をJSONへ保存する。"""
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("net", "positions", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--probe-command", nargs="+", required=True)
    args = parser.parse_args(arguments)
    if args.output.exists():
        raise ValueError(f"{args.output}: diagnostic output already exists")
    executable = shutil.which(args.probe_command[0])
    if executable is None:
        raise ValueError(f"{args.probe_command[0]}: probe executable not found")
    report = compare(args.net, args.positions, rust_probe(args.probe_command))
    report["probe_command"] = [*args.probe_command, "--net", str(args.net), "--positions", str(args.positions)]
    report["probe_sha256"] = sha256_file(Path(executable)).hex()
    args.output.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n")


if __name__ == "__main__":
    main()
