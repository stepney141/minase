"""自己対局学習データ形式MNSDを検証し、NumPy配列として読む。"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
import struct
from typing import Sequence

import numpy as np
from numpy.typing import NDArray


HEADER_LENGTH = 136
RECORD_LENGTH = 160
NO_LION_SQUARE = 255

RECORD_DTYPE = np.dtype(
    [
        ("board", "u1", (144,)),
        ("stm", "u1"),
        ("lion", "u1"),
        ("kirin", "u1"),
        ("score", "<i2"),
        ("result", "u1"),
        ("game", "<u4"),
        ("ply", "<u2"),
        ("reserved", "u1", (4,)),
    ]
)


@dataclass(frozen=True)
class Header:
    """検証済みMNSDヘッダの来歴情報を保持する。"""

    rule_set: str
    generation_commit: str
    network_checksum: bytes
    teacher_nodes: int
    seed: int
    record_count: int


def _decode_nul_padded(field: bytes, name: str) -> str:
    """NUL埋めされたUTF-8固定長欄を検証して文字列へ戻す。"""
    end = field.find(b"\0")
    if end < 0:
        end = len(field)
    if any(field[end:]):
        raise ValueError(f"{name} is not NUL padded")
    try:
        return field[:end].decode("utf-8")
    except UnicodeDecodeError as error:
        raise ValueError(f"{name} is not UTF-8") from error


def read_header(path: Path) -> Header:
    """MNSDヘッダとファイル長を検証して返す。"""
    file_length = path.stat().st_size
    with path.open("rb") as stream:
        raw = stream.read(HEADER_LENGTH)
    if len(raw) != HEADER_LENGTH:
        raise ValueError(
            f"{path}: header length must be {HEADER_LENGTH}, got {len(raw)}"
        )
    magic, version, record_length = struct.unpack_from("<4sII", raw)
    if magic != b"MNSD":
        raise ValueError(f"{path}: invalid MNSD magic {magic!r}")
    if version != 1:
        raise ValueError(f"{path}: unsupported MNSD version {version}")
    if record_length != RECORD_LENGTH:
        raise ValueError(
            f"{path}: record length must be {RECORD_LENGTH}, got {record_length}"
        )
    rule_set = _decode_nul_padded(raw[12:44], "rule-set name")
    try:
        generation_commit = raw[44:84].decode("ascii")
    except UnicodeDecodeError as error:
        raise ValueError(f"{path}: generation commit is not ASCII") from error
    if len(generation_commit) != 40 or any(
        character not in "0123456789abcdefABCDEF" for character in generation_commit
    ):
        raise ValueError(f"{path}: generation commit is not a full hexadecimal hash")
    teacher_nodes = struct.unpack_from("<I", raw, 116)[0]
    seed = struct.unpack_from("<Q", raw, 120)[0]
    record_count = struct.unpack_from("<Q", raw, 128)[0]
    expected_length = HEADER_LENGTH + RECORD_LENGTH * record_count
    if file_length != expected_length:
        raise ValueError(
            f"{path}: file length must be {expected_length}, got {file_length}"
        )
    return Header(
        rule_set=rule_set,
        generation_commit=generation_commit,
        network_checksum=raw[84:116],
        teacher_nodes=teacher_nodes,
        seed=seed,
        record_count=record_count,
    )


def _valid_piece_bytes() -> NDArray[np.bool_]:
    """MNSD盤面欄で有効な256種類のバイトを表にする。"""
    valid = np.zeros(256, dtype=np.bool_)
    valid[0] = True
    valid[1:59] = True
    valid[65:123] = True
    promotable_results = np.zeros(29, dtype=np.bool_)
    promotable_results[
        [4, 5, 6, 7, 8, 9, 10, 12, 17, 20, 21, 22, 23, 24, 25, 26, 27, 28]
    ] = True
    for color_base in (0, 64):
        for kind in range(29):
            if not promotable_results[kind]:
                valid[color_base + 1 + 29 + kind] = False
    return valid


VALID_PIECE_BYTES = _valid_piece_bytes()


def _validate_records(path: Path, records: np.ndarray) -> None:
    """MNSDレコードの定義域と予約欄を一括検証する。"""
    if not np.all(VALID_PIECE_BYTES[records["board"]]):
        raise ValueError(f"{path}: board contains an invalid piece byte")
    if not np.all(records["stm"] <= 1):
        raise ValueError(f"{path}: side-to-move code is outside 0..1")
    lions = records["lion"]
    if not np.all((lions < 144) | (lions == NO_LION_SQUARE)):
        raise ValueError(f"{path}: lion square is outside 0..143 or 255")
    kirin = records["kirin"]
    if not np.all(kirin <= 1):
        raise ValueError(f"{path}: kirin-promotion flag is outside 0..1")
    if np.any((lions == NO_LION_SQUARE) & (kirin != 0)):
        raise ValueError(f"{path}: kirin-promotion flag has no lion square")
    if not np.all(records["result"] <= 2):
        raise ValueError(f"{path}: result code is outside 0..2")
    if np.any(records["reserved"]):
        raise ValueError(f"{path}: reserved record bytes are nonzero")


def map_records(path: str | Path) -> np.memmap:
    """単一MNSDファイルを検証し、レコード領域をメモリマップする。"""
    resolved = Path(path)
    header = read_header(resolved)
    records = np.memmap(
        resolved,
        mode="r",
        dtype=RECORD_DTYPE,
        offset=HEADER_LENGTH,
        shape=(header.record_count,),
    )
    _validate_records(resolved, records)
    return records


def hash64(seed: int, games: NDArray[np.uint32]) -> NDArray[np.uint64]:
    """ファイルのシードと対局番号から安定した64ビット値を作る。"""
    values = np.asarray(games, dtype=np.uint64)
    with np.errstate(over="ignore"):
        values = np.uint64(seed) ^ (values * np.uint64(0x9E3779B97F4A7C15))
        values = (values ^ (values >> np.uint64(30))) * np.uint64(
            0xBF58476D1CE4E5B9
        )
        values = (values ^ (values >> np.uint64(27))) * np.uint64(
            0x94D049BB133111EB
        )
    return values ^ (values >> np.uint64(31))


class Dataset:
    """複数MNSDをメモリマップのまま保持し、大域番号で参照する。"""

    def __init__(self, paths: Sequence[str | Path]) -> None:
        if not paths:
            raise ValueError("at least one MNSD path is required")

        resolved_paths = tuple(Path(path).resolve() for path in paths)
        if len(set(resolved_paths)) != len(resolved_paths):
            raise ValueError("the same MNSD file was specified more than once")

        headers = tuple(read_header(path) for path in resolved_paths)
        seeds = [header.seed for header in headers]
        if len(set(seeds)) != len(seeds):
            raise ValueError("MNSD file seeds must be unique")

        counts = np.fromiter(
            (header.record_count for header in headers), dtype=np.uint64
        )
        total = sum(header.record_count for header in headers)
        if total > np.iinfo(np.int64).max:
            raise OverflowError("combined record count exceeds i64")

        checksums: list[bytes] = []
        checksum_generations: dict[bytes, int] = {}
        file_generations = np.empty(len(headers), dtype=np.int64)
        for file_index, header in enumerate(headers):
            generation = checksum_generations.get(header.network_checksum)
            if generation is None:
                generation = len(checksums)
                checksum_generations[header.network_checksum] = generation
                checksums.append(header.network_checksum)
            file_generations[file_index] = generation

        offsets = np.empty(len(headers) + 1, dtype=np.int64)
        offsets[0] = 0
        offsets[1:] = np.cumsum(counts, dtype=np.uint64)
        records = tuple(map_records(path) for path in resolved_paths)

        validation_masks: list[NDArray[np.bool_]] = []
        training_indices_by_file: list[NDArray[np.int64]] = []
        validation_indices_by_file: list[NDArray[np.int64]] = []
        for file_index, (header, mapped) in enumerate(zip(headers, records)):
            validation = hash64(header.seed, mapped["game"]) % np.uint64(20) == 0
            validation_masks.append(validation)
            offset = offsets[file_index]
            training_indices_by_file.append(
                np.flatnonzero(~validation).astype(np.int64) + offset
            )
            validation_indices_by_file.append(
                np.flatnonzero(validation).astype(np.int64) + offset
            )

        self.paths = resolved_paths
        self.headers = headers
        self.records = records
        self.offsets = offsets
        self.file_generations = file_generations
        self.generation_checksums = tuple(checksums)
        self.validation_masks = tuple(validation_masks)
        self.training_indices_by_file = tuple(training_indices_by_file)
        self.validation_indices_by_file = tuple(validation_indices_by_file)
        self.training_indices = np.concatenate(training_indices_by_file)
        self.validation_indices = np.concatenate(validation_indices_by_file)

    @property
    def record_count(self) -> int:
        """全ファイルのレコード数を返す。"""
        return int(self.offsets[-1])

    @property
    def generation_count(self) -> int:
        """ネット検査和で識別した世代数を返す。"""
        return len(self.generation_checksums)

    def generation_training_indices(self, generation: int) -> NDArray[np.int64]:
        """指定世代に属する訓練レコードの大域番号を返す。"""
        if not 0 <= generation < self.generation_count:
            raise IndexError("generation is outside the dataset")
        chunks = [
            indices
            for file_generation, indices in zip(
                self.file_generations, self.training_indices_by_file
            )
            if file_generation == generation
        ]
        return np.concatenate(chunks)

    def generations(self, indices: NDArray[np.int64]) -> NDArray[np.int64]:
        """大域レコード番号に対応する世代番号を返す。"""
        normalized = self._normalize_indices(indices)
        files = np.searchsorted(self.offsets[1:], normalized, side="right")
        return self.file_generations[files]

    def gather(self, indices: NDArray[np.int64]) -> np.ndarray:
        """任意順の大域レコード番号を同じ順序の構造化配列へ集める。"""
        normalized = self._normalize_indices(indices)
        gathered = np.empty(normalized.shape[0], dtype=RECORD_DTYPE)
        if normalized.size == 0:
            return gathered

        files = np.searchsorted(self.offsets[1:], normalized, side="right")
        for file_index, mapped in enumerate(self.records):
            positions = np.flatnonzero(files == file_index)
            if positions.size == 0:
                continue
            local = normalized[positions] - self.offsets[file_index]
            local_order = np.argsort(local, kind="stable")
            gathered[positions[local_order]] = mapped[local[local_order]]
        return gathered

    def _normalize_indices(self, indices: NDArray[np.int64]) -> NDArray[np.int64]:
        """大域レコード番号を検証してint64配列に揃える。"""
        normalized = np.asarray(indices, dtype=np.int64)
        if normalized.ndim != 1:
            raise ValueError("record indices must be one-dimensional")
        if np.any(normalized < 0) or np.any(normalized >= self.record_count):
            raise IndexError("record index is outside the dataset")
        return normalized
