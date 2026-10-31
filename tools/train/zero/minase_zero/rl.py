"""自己対局のシードの区間を登録し、直前のサイクルから投了を較正する。

台帳は既存のJSON Linesファイルとし、混合前の閉区間[seed + 1, seed + games]を
重なりとuint64の折り返しがない場合だけ登録する。全行を検査してから同じディレクトリの
一時ファイルへ書いて同期し、置き換え後に親ディレクトリも同期する。

投了の較正では、生成器のsummary.jsonの件数を合算し、Clopper–Pearson法による
片側95%上限を求める。標本が100対局以上あり、上限が0.05以下の場合だけ投了を有効にする。
"""

from datetime import UTC, datetime
import json
import math
import os
from pathlib import Path
import re
import tempfile


def _seed_interval(name: str, seed: int, games: int) -> tuple[int, int]:
    if not isinstance(name, str) or not name or any(char.isspace() for char in name):
        raise ValueError("name must be nonempty and contain no whitespace")
    if type(seed) is not int or not 0 <= seed < 2**64:
        raise ValueError("seed must be an integer in [0, 2^64 - 1]")
    if type(games) is not int or games < 1:
        raise ValueError("games must be a positive integer")
    if seed + games > 2**64 - 1:
        raise ValueError("seed + games exceeds uint64; the game seeds would wrap")
    return seed + 1, seed + games


def register_seed_range(ledger: Path, name: str, seed: int, games: int) -> dict:
    """既存の台帳を検査し、重ならない区間を確定させて登録内容を返す。"""
    low, high = _seed_interval(name, seed, games)
    contents = ledger.read_bytes()
    entries = []
    names = set()
    fields = {"name", "seed", "games", "low", "high", "registered_at"}
    for number, line in enumerate(contents.decode("utf-8").splitlines(), start=1):
        try:
            entry = json.loads(line)
            if not isinstance(entry, dict) or set(entry) != fields:
                raise ValueError("entry must have name, seed, games, low, high and registered_at")
            expected = _seed_interval(entry["name"], entry["seed"], entry["games"])
            if (type(entry["low"]) is not int or type(entry["high"]) is not int
                    or (entry["low"], entry["high"]) != expected):
                raise ValueError("low and high must match seed + 1 and seed + games")
            timestamp = entry["registered_at"]
            if not isinstance(timestamp, str) or re.fullmatch(
                r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z", timestamp,
            ) is None:
                raise ValueError("registered_at must be UTC ISO 8601 to seconds ending in Z")
            datetime.fromisoformat(timestamp)
            if entry["name"] in names:
                raise ValueError(f"duplicate name: {entry['name']}")
            names.add(entry["name"])
            entries.append(entry)
        except ValueError as error:
            raise ValueError(f"{ledger}:{number}: {error}") from error

    if name in names:
        raise ValueError(f"duplicate name: {name}")
    for entry in entries:
        if low <= entry["high"] and entry["low"] <= high:
            raise ValueError(
                f"[{low}, {high}] overlaps {entry['name']} [{entry['low']}, {entry['high']}]"
            )

    registration = {
        "name": name, "seed": seed, "games": games, "low": low, "high": high,
        "registered_at": datetime.now(UTC).isoformat(timespec="seconds").replace("+00:00", "Z"),
    }
    line = json.dumps(registration, ensure_ascii=False, allow_nan=False) + "\n"
    descriptor, temporary_name = tempfile.mkstemp(
        prefix=f".{ledger.name}.", suffix=".tmp", dir=ledger.parent,
    )
    temporary = Path(temporary_name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(contents)
            if contents and not contents.endswith(b"\n"):
                output.write(b"\n")
            output.write(line.encode("utf-8"))
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, ledger)
        directory = os.open(ledger.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        temporary.unlink(missing_ok=True)
    return registration


def _log_binomial_cdf(n: int, x: int, p: float) -> float:
    """二項分布のP(X ≤ x)を、隣り合う確率の比と対数空間の加算で求める。"""
    log_failure = math.log1p(-p)
    log_odds = math.log(p) - log_failure
    log_term = n * log_failure
    log_total = log_term
    for count in range(1, x + 1):
        log_term += math.log(n - count + 1) - math.log(count) + log_odds
        larger = max(log_total, log_term)
        log_total = larger + math.log1p(math.exp(-abs(log_total - log_term)))
    return log_total


def _upper_bound(n: int, x: int) -> float | None:
    if n == 0:
        return None
    if x == n:
        return 1.0
    low, high = 0.0, 1.0
    target = math.log(0.05)
    while high - low > high * 1e-12:
        middle = (low + high) / 2
        if _log_binomial_cdf(n, x, middle) > target:
            low = middle
        else:
            high = middle
    return high


def resign_calibration(summary: list[Path]) -> dict:
    """生成器の集計を合算し、誤投了率の上限と次のサイクルの投了の可否を返す。"""
    if not summary:
        raise ValueError("at least one summary is required")
    n, x = 0, 0
    for path in summary:
        counts = json.loads(path.read_text(encoding="utf-8"))
        fields = ("resign_triggered_playthrough_games", "false_resignations")
        if not isinstance(counts, dict) or any(field not in counts for field in fields):
            raise ValueError(f"{path}: missing resign_triggered_playthrough_games or false_resignations")
        trials, errors = (counts[field] for field in fields)
        if (type(trials) is not int or type(errors) is not int
                or not 0 <= errors <= trials):
            raise ValueError(f"{path}: counts must be integers with 0 <= false_resignations <= resign_triggered_playthrough_games")
        n += trials
        x += errors
    upper = _upper_bound(n, x)
    return {
        "n": n, "false_resignations": x, "upper_bound": upper,
        "enable_resignation": n >= 100 and upper is not None and upper <= 0.05,
    }
