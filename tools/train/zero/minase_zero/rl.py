"""自己対局のシードの区間を登録し、過去の全サイクルから投了の閾値を較正する。

台帳は既存のJSON Linesファイルとし、混合前の閉区間[seed + 1, seed + games]を
重なりとuint64の折り返しがない場合だけ登録する。全行を検査してから同じディレクトリの
一時ファイルへ書いて同期し、置き換え後に親ディレクトリも同期する。

投了の較正はAobaZeroのコミット5eb9441のsrc/server/datakeep.cppのEMAKeepと
server.cfgのResignEMADeno=5,000、ResignEMAInit=0.10、ResignMRate=0.05に従う。
64候補の誤り率を初期値0.10から対局ごとに指数移動平均で更新する。
集計はサイクルの順、各配列は対局番号の順に読み、勝者の最小値、引き分けでは両者の
小さいほうを使う。状態は保存せず、同じ入力から毎回同じ閾値を計算する。
方式の理由はdocs/adr/calibrate-the-zero-self-play-resignation-threshold-as-aobazero-does-and-record-the-gumbel-choice-value.mdにある。
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


def _resign_threshold(estimates: list[float]) -> float:
    """誤り率が0.05を超える最初の候補の直下を閾値にする。"""
    for index, estimate in enumerate(estimates):
        if estimate > 0.05:
            return 0.005 * index
    return 0.32


def resign_calibration(summary: list[Path]) -> dict:
    """過去の集計を指定順に適用し、閾値、64個の推定、取り込んだ対局数を返す。"""
    estimates = [0.10] * 64
    games = 0
    decay = 4999 / 5000
    fields = {"game", "result", "min_value_black", "min_value_white"}
    for path in summary:
        counts = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(counts, dict) or "resign_calibration_games" not in counts:
            raise ValueError(f"{path}: missing resign_calibration_games")
        records = counts["resign_calibration_games"]
        if not isinstance(records, list):
            raise ValueError(f"{path}: resign_calibration_games must be a list")
        previous = 0
        for record in records:
            if not isinstance(record, dict) or set(record) != fields:
                raise ValueError(f"{path}: calibration game must have game, result, min_value_black and min_value_white")
            number = record["game"]
            if type(number) is not int or number <= previous:
                raise ValueError(f"{path}: calibration game numbers must be positive integers in strictly increasing order")
            previous = number
            result = record["result"]
            if result not in ("black", "white", "draw"):
                raise ValueError(f"{path}: calibration result must be black, white or draw")
            for field in ("min_value_black", "min_value_white"):
                value = record[field]
                if type(value) not in (int, float) or not 0 <= value <= 1 or not math.isfinite(value):
                    raise ValueError(f"{path}: {field} must be finite and in [0, 1]")
            minimum = (min(record["min_value_black"], record["min_value_white"]) if result == "draw"
                       else record[f"min_value_{result}"])
            for index in range(64):
                estimates[index] = decay * estimates[index] + (1 - decay) * (minimum <= 0.005 * (index + 1))
            games += 1
    return {"threshold": _resign_threshold(estimates), "estimates": estimates, "games": games}
