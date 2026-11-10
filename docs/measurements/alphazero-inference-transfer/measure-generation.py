#!/usr/bin/env python3
"""変種ごとの自己対局生成を交互に測り、中央値と測定の幅から採否を集計する。

warmは最大バッチを指定して16局を生成し、warm.jsonlへ名前、elapsed_seconds、
exit_codeを追記する。runは偶数周を一覧の順、奇数周を逆順に実行し、runs.jsonlへ
生成器が測った時間と訓練局面数を残す。記録後に対局ファイルを削除し、JSONを残す。
既存の生成先は再使用せず、追加測定の周は--round-startで指定する。
summarizeは名前をキーとするvariantsとcomparisonsを出力する。
variantsのmedian、min、maxは毎秒の訓練局面数、countは回数、widthは幅である。
comparisonsにはparent、increment、threshold、decision、slower_beyond_widthを置く。
引数はすべて必須で、子プロセスの環境変数は呼び出し元からそのまま引き継ぐ。
"""

import argparse
from datetime import datetime, timezone
import json
import math
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import time


class Parser(argparse.ArgumentParser):
    def error(self, message: str) -> None:
        raise ValueError(message)


def _reject_constant(value: str) -> None:
    raise ValueError(f"non-finite JSON number: {value}")


def _unique_object(pairs: list[tuple[str, object]]) -> dict:
    value = {}
    for name, item in pairs:
        if name in value:
            raise ValueError(f"duplicate JSON field: {name}")
        value[name] = item
    return value


def _json(text: str, source: object) -> object:
    try:
        return json.loads(text, parse_constant=_reject_constant, object_pairs_hook=_unique_object)
    except ValueError as error:
        raise ValueError(f"{source}: {error}") from error


def _object(value: object, source: object) -> dict:
    if not isinstance(value, dict):
        raise ValueError(f"{source}: must be a JSON object")
    return value


def _field(value: dict, name: str, source: object) -> object:
    if name not in value:
        raise ValueError(f"{source}: missing field {name}")
    return value[name]


def _name(value: object, source: object) -> str:
    if (not isinstance(value, str) or not value.strip() or value in (".", "..")
            or any(character in value for character in ("/", "\\", "\0"))):
        raise ValueError(f"{source}: name must be a nonempty single path component")
    return value


def _integer(value: object, source: object) -> int:
    if type(value) is not int or value < 0:
        raise ValueError(f"{source}: must be a nonnegative integer")
    return value


def _number(value: object, source: object) -> float:
    if type(value) not in (int, float):
        raise ValueError(f"{source}: must be a finite nonnegative number")
    try:
        number = float(value)
    except OverflowError as error:
        raise ValueError(f"{source}: number is too large") from error
    if not math.isfinite(number) or number < 0:
        raise ValueError(f"{source}: must be a finite nonnegative number")
    return number


def _hex(value: object, digits: int, source: object) -> str:
    if not isinstance(value, str) or re.fullmatch(rf"[0-9a-fA-F]{{{digits}}}", value) is None:
        raise ValueError(f"{source}: must be {digits} hexadecimal digits")
    return value


def load_variants(path: Path) -> list[dict]:
    """一覧の欄、識別子、ファイル、および比較相手を実行前に検査する。"""
    variants = _json(path.read_text(encoding="utf-8"), path)
    if not isinstance(variants, list) or not variants:
        raise ValueError(f"{path}: variants must be a nonempty JSON array")
    names = set()
    fields = {"name", "minase", "network", "commit", "parent"}
    for index, value in enumerate(variants):
        source = f"{path}[{index}]"
        variant = _object(value, source)
        if set(variant) != fields:
            missing = sorted(fields - set(variant))
            extra = sorted(set(variant) - fields)
            raise ValueError(f"{source}: incorrect fields; missing={missing}, extra={extra}")
        name = _name(variant["name"], f"{source}.name")
        if name in names:
            raise ValueError(f"{source}: duplicate name {name}")
        names.add(name)
        _hex(variant["commit"], 40, f"{source}.commit")
        if variant["parent"] is not None:
            _name(variant["parent"], f"{source}.parent")
        for field in ("minase", "network"):
            filename = variant[field]
            if not isinstance(filename, str) or not filename or "\0" in filename:
                raise ValueError(f"{source}.{field}: must be an absolute file path string")
            file = Path(filename)
            if not file.is_absolute():
                raise ValueError(f"{source}.{field}: must be an absolute file path")
            if not file.is_file():
                raise ValueError(f"{source}.{field}: file does not exist: {file}")
            if field == "minase" and not os.access(file, os.X_OK):
                raise ValueError(f"{source}.minase: file is not executable: {file}")
    for variant in variants:
        if variant["parent"] is not None and variant["parent"] not in names:
            raise ValueError(f"{path}: {variant['name']} has unknown parent {variant['parent']}")
    return variants


def _records(path: Path):
    with path.open(encoding="utf-8") as stream:
        for number, line in enumerate(stream, 1):
            source = f"{path}:{number}"
            yield _object(_json(line, source), source), source


def _append(path: Path, record: dict) -> None:
    line = json.dumps(record, ensure_ascii=False, allow_nan=False) + "\n"
    with path.open("a", encoding="utf-8") as stream:
        stream.write(line)


def _log_path(output: Path) -> Path:
    return output.parent / f"{output.name}.log"


def _check_output(output: Path) -> None:
    for path in (output, _log_path(output)):
        if path.exists() or path.is_symlink():
            raise ValueError(f"output already exists: {path}; use a new round or output directory")


def _generate(args: argparse.Namespace, variant: dict, output: Path,
              games: int, chunk_games: int) -> tuple[int, str]:
    output.mkdir(parents=True)
    command = [
        variant["minase"], "data", "zero-selfplay", "--output-dir", str(output),
        "--nn-model", variant["network"], "--nn-backend", "tensorrt",
        "--generation-commit", variant["commit"], "--seed", str(args.seed),
        "--start-position", "shuffled-setup", "--games", str(games),
        "--target-positions", "1000000000", "--chunk-games", str(chunk_games),
        "--simulations", str(args.simulations), "--playthrough-percent", "100",
        "--threads", str(args.threads_per_gpu * args.devices), "--devices", str(args.devices),
        "--games-per-thread", str(args.games_per_thread),
    ]
    with _log_path(output).open("x", encoding="utf-8") as log:
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=log,
                                text=True, encoding="utf-8", check=False)
        log.write(result.stdout)
    return result.returncode, result.stdout


def _check_exit(code: int, output: Path) -> None:
    if code != 0:
        raise RuntimeError(f"generator exited with code {code}; log: {_log_path(output)}")


def warm(args: argparse.Namespace, variants: list[dict]) -> None:
    """各変種で16局を生成し、成功・失敗のどちらも壁時計の経過秒とともに残す。"""
    for variant in variants:
        _check_output(args.out / "warm" / variant["name"])
    for variant in variants:
        output = args.out / "warm" / variant["name"]
        started = time.perf_counter()
        code, _ = _generate(args, variant, output, 16, 16)
        elapsed = time.perf_counter() - started
        _append(args.out / "warm.jsonl", {
            "name": variant["name"], "elapsed_seconds": elapsed, "exit_code": code,
        })
        _check_exit(code, output)


def _report(output: Path, stdout: str) -> dict:
    path = output / "summary.json"
    summary = _object(_json(path.read_text(encoding="utf-8"), path), path)
    counts = {
        name: _integer(_field(summary, name, path), f"{path}.{name}")
        for name in ("positions", "resigned_positions", "plies", "full_searches",
                     "fast_searches", "games", "discarded_max_ply")
    }
    source = f"{_log_path(output)} (stdout)"
    fields = {}
    for line in stdout.splitlines():
        name, separator, value = line.partition(":")
        if separator and name in ("network_sha256", "elapsed_seconds"):
            if name in fields:
                raise ValueError(f"{source}: duplicate field {name}")
            fields[name] = value.strip()
    checksum = _hex(_field(fields, "network_sha256", source), 64, f"{source}.network_sha256")
    elapsed = _number(_json(_field(fields, "elapsed_seconds", source), f"{source}.elapsed_seconds"),
                      f"{source}.elapsed_seconds")
    if elapsed == 0:
        raise ValueError(f"{source}.elapsed_seconds: must be positive")
    training_positions = counts.pop("positions") + counts.pop("resigned_positions")
    speed = _number(_number(training_positions, f"{path}.training_positions") / elapsed,
                    f"{source}.positions_per_second")
    return {"network_sha256": checksum, "training_positions": training_positions, **counts,
            "elapsed_seconds": elapsed, "positions_per_second": speed}


def run(args: argparse.Namespace, variants: list[dict]) -> None:
    """周と変種の重複を開始前に拒み、成功した生成だけを記録して対局を削除する。"""
    runs = args.out / "runs.jsonl"
    existing = set()
    if runs.exists():
        for row, source in _records(runs):
            key = (_integer(_field(row, "round", source), f"{source}.round"),
                   _name(_field(row, "name", source), f"{source}.name"))
            if key in existing:
                raise ValueError(f"{source}: duplicate measurement: round={key[0]}, name={key[1]}")
            existing.add(key)
    rounds = range(args.round_start, args.round_start + args.rounds)
    for number in rounds:
        for variant in variants:
            if (number, variant["name"]) in existing:
                raise ValueError(f"{runs}: duplicate measurement: round={number}, name={variant['name']}")
    for number in rounds:
        for variant in variants:
            _check_output(args.out / f"round-{number:02d}" / variant["name"])
    for number in rounds:
        ordered = variants if number % 2 == 0 else reversed(variants)
        for position, variant in enumerate(ordered):
            output = args.out / f"round-{number:02d}" / variant["name"]
            started_at = datetime.now(timezone.utc).isoformat()
            code, stdout = _generate(args, variant, output, args.games, args.chunk_games)
            _check_exit(code, output)
            record = {"round": number, "position": position, "name": variant["name"],
                      "commit": variant["commit"], **_report(output, stdout), "started_at": started_at}
            _append(runs, record)
            for path in output.glob("*.mnsd"):
                path.unlink()


def summarize(variants: list[dict], paths: list[Path]) -> dict:
    """生成の速さの中央値と幅を求め、比較相手に対する増分を判定する。"""
    samples = {variant["name"]: [] for variant in variants}
    for path in paths:
        for row, source in _records(path):
            name = _name(_field(row, "name", source), f"{source}.name")
            if name not in samples:
                raise ValueError(f"{source}: unknown variant {name}")
            samples[name].append(_number(_field(row, "positions_per_second", source),
                                         f"{source}.positions_per_second"))
    stats = {}
    for name, values in samples.items():
        if not values:
            raise ValueError(f"{name}: no measurement rows")
        median = statistics.median(values)
        if median == 0:
            raise ValueError(f"{name}: median positions_per_second must be positive")
        minimum, maximum = min(values), max(values)
        stats[name] = {"median": median, "min": minimum, "max": maximum, "count": len(values),
                       "width": (maximum - minimum) / median}
    comparisons = {}
    for variant in variants:
        name, parent = variant["name"], variant["parent"]
        if parent is None:
            continue
        candidate, reference = stats[name], stats[parent]
        if candidate["count"] != reference["count"]:
            raise ValueError(f"{name} and {parent}: measurement counts differ "
                             f"({candidate['count']} != {reference['count']})")
        increment = candidate["median"] / reference["median"] - 1
        threshold = max(candidate["width"], reference["width"])
        decision = "adopt" if increment > threshold else "undecided" if increment > 0 else "reject"
        comparisons[name] = {"parent": parent, "increment": increment, "threshold": threshold,
                             "decision": decision,
                             "slower_beyond_width": increment < -reference["width"]}
    return {"variants": stats, "comparisons": comparisons}


def _parse(argv: list[str]) -> argparse.Namespace:
    parser = Parser(description=__doc__, allow_abbrev=False)
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("warm", "run", "summarize"):
        subparser = commands.add_parser(command, allow_abbrev=False)
        subparser.add_argument("--variants", type=Path, required=True)
        if command == "summarize":
            subparser.add_argument("--runs", type=Path, nargs="+", required=True)
            continue
        subparser.add_argument("--out", type=Path, required=True)
        for name in ("devices", "threads-per-gpu", "games-per-thread", "simulations", "seed"):
            subparser.add_argument(f"--{name}", type=int, required=True)
        if command == "run":
            for name in ("round-start", "rounds", "games", "chunk-games"):
                subparser.add_argument(f"--{name}", type=int, required=True)
    args = parser.parse_args(argv)
    if args.command != "summarize":
        positive = ["devices", "threads_per_gpu", "games_per_thread", "simulations"]
        if args.command == "run":
            positive += ["rounds", "games", "chunk_games"]
            if args.round_start < 0:
                raise ValueError("--round-start must be nonnegative")
        for name in positive:
            if getattr(args, name) <= 0:
                raise ValueError(f"--{name.replace('_', '-')} must be positive")
        if not 0 <= args.seed <= 0xFFFF_FFFF_FFFF_FFFF:
            raise ValueError("--seed must fit in uint64")
    return args


def main(argv: list[str]) -> int:
    """入力と実行の失敗を標準エラーへ示し、終了コード1を返す。"""
    try:
        args = _parse(argv)
        variants = load_variants(args.variants)
        if args.command == "warm":
            warm(args, variants)
        elif args.command == "run":
            run(args, variants)
        else:
            print(json.dumps(summarize(variants, args.runs), ensure_ascii=False,
                             indent=2, allow_nan=False))
    except (OSError, ValueError, RuntimeError) as error:
        print(f"measure-generation: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
