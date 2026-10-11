"""方策の教師を持つ学習データの二進形式MNZTの読み込み。

形式はRust側の`crates/minase/src/nn/mnzt.rs`が正であり、本モジュールはその読み込みだけを写す。
1つのファイルは、176バイトのヘッダ、168バイトの固定長レコードを並べた局面の表、および
（ラベル、確率）の4バイトの組を並べた方策の列を、この順に連続して置く。
数値はすべてリトルエンディアンである。
局面の盤面の駒コードは評価関数の学習データ形式MNSD（`crates/minase/src/training/records.rs`の
`encode_piece`）と同じである。
局面の欄は成り権を保留している駒（規則P1、P2、P5）の升を持たないので、MNZTの局面を符号化した
入力平面の面95（`encoding.PROMOTION_DEFERRED_PLANE`）は常に0になる。

開く時点でヘッダ、ファイル長、および全レコードと全組の欄を検証し、壊れたファイルは`MnztError`で拒否する。
局面の表と方策の列は`numpy.memmap`で読むので、大きなファイルもメモリへ載せずに扱える。
"""

from __future__ import annotations

import hashlib
import struct
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from minase_zero.encoding import (
    _KIND_NAMES,
    _PROMOTED,
    POLICY_SIZE,
    SQUARE_COUNT,
    Piece,
    Position,
)

MAGIC = b"MNZT"
"""ファイル識別子。"""
FORMAT_VERSION = 1
"""本モジュールが読む形式の版。"""
HEADER_LEN = 176
"""ヘッダのバイト数。"""
RECORD_LEN = 168
"""局面の表の1レコードのバイト数。"""
POLICY_ENTRY_LEN = 4
"""方策の列の1組のバイト数。"""
PROBABILITY_SCALE = 65535.0
"""確率1に対応する量子化値。"""
NO_LION_SQUARE = 255
"""先獅子の升がないことを示す升コード。"""

TEACHER_KINDS = ("alphabeta_nodes", "mcts_simulations")
"""教師の種類コード0と1の名前。"""

RECORD_DTYPE = np.dtype(
    [
        ("board", "u1", (SQUARE_COUNT,)),
        ("side_to_move", "u1"),
        ("lion_square", "u1"),
        ("lion_by_kirin_promotion", "u1"),
        ("outcome", "u1"),
        ("game_number", "<u4"),
        ("ply", "<u2"),
        ("policy_count", "<u2"),
        ("value", "<f4"),
        ("policy_start", "<u8"),
    ]
)
"""局面の表の1レコード。`outcome`は手番側から見た結果（0負け、1引き分け、2勝ち）、
`value`は手番側から見た根の価値（勝率）である。"""

POLICY_DTYPE = np.dtype([("label", "<u2"), ("probability", "<u2")])
"""方策の列の1組。`probability`は確率に65535を掛けて丸めた値である。"""

assert RECORD_DTYPE.itemsize == RECORD_LEN
assert POLICY_DTYPE.itemsize == POLICY_ENTRY_LEN

_KIND_COUNT = len(_KIND_NAMES)
# 成った後にしか現れない駒種（minase-coreの PieceKind::can_exist_unpromoted が偽のもの）。
_PROMOTED_ONLY = frozenset(
    {
        "CrownPrince",
        "WhiteHorse",
        "Whale",
        "FlyingOx",
        "FreeBoar",
        "FlyingStag",
        "HornedFalcon",
        "SoaringEagle",
    }
)
_PROMOTED_KINDS = frozenset(_PROMOTED.values())

# 検証の1回に読むレコード数と組の数。メモリの使用量を抑えるために分割して検証する。
_VALIDATION_CHUNK = 1 << 20


class MnztError(ValueError):
    """MNZTファイルの検証に失敗した。"""


def _decode_piece_code(value: int) -> Piece | None:
    """駒コードを駒へ復号する。0は空升として`None`を返す。

    Raises:
        MnztError: 盤上の駒を表さないコードの場合。
    """
    if value == 0:
        return None
    if 1 <= value <= 2 * _KIND_COUNT:
        color, payload = 0, value - 1
    elif 65 <= value <= 64 + 2 * _KIND_COUNT:
        color, payload = 1, value - 65
    else:
        raise MnztError(f"invalid piece code {value}")
    promoted = payload >= _KIND_COUNT
    kind = _KIND_NAMES[payload % _KIND_COUNT]
    if promoted and kind not in _PROMOTED_KINDS:
        raise MnztError(f"invalid piece code {value}: {kind} is not a promoted kind")
    if not promoted and kind in _PROMOTED_ONLY:
        raise MnztError(f"invalid piece code {value}: {kind} cannot be unpromoted")
    return Piece(color, kind, promoted)


def _piece_table() -> tuple[tuple[Piece | None, ...], np.ndarray]:
    """駒コード0〜255から駒への表と、有効なコードを示す真偽値の表を作る。"""
    pieces: list[Piece | None] = []
    valid = np.zeros(256, dtype=bool)
    for code in range(256):
        try:
            pieces.append(_decode_piece_code(code))
            valid[code] = True
        except MnztError:
            pieces.append(None)
    return tuple(pieces), valid


_PIECES, _VALID_PIECE_CODE = _piece_table()


def record_position(record: np.void) -> Position:
    """局面の表の1レコードから、`encoding.encode`が受け付ける局面を作る。

    盤面の升の番号（dense index、`段 * 12 + 筋`）と先獅子の升コードは、
    `encoding.Position`の升番号と同じ規約である。レコードは`MnztFile`が検証済みとする。
    MNZTのレコードは成り権保留の升を持たないので、`promotion_deferred`は常に空である。
    """
    lion = int(record["lion_square"])
    return Position(
        board=tuple(_PIECES[code] for code in record["board"].tolist()),
        side_to_move=int(record["side_to_move"]),
        lion_capture=None if lion == NO_LION_SQUARE else lion,
        promotion_deferred=(),
    )


@dataclass(frozen=True)
class Header:
    """検証済みのMNZTヘッダ。"""

    rule_set: str
    """規則セットの表示文字列。"""
    generation_commit: str
    """生成元コミットの完全ハッシュ。"""
    network_checksum: bytes
    """生成に使ったネットの検査和（32バイト）。ネットを使わない生成では全零。"""
    teacher_kind: str
    """方策の教師を作った探索の種類（`TEACHER_KINDS`のどちらか）。"""
    teacher_value: int
    """αβ探索のノード数の上限、またはMCTSの探索回数。"""
    seed: int
    """データ生成の基本シード。"""
    record_count: int
    """局面の表のレコード数。"""
    policy_count: int
    """方策の列の組の総数。"""

    @property
    def policy_offset(self) -> int:
        """方策の列の開始バイト位置。"""
        return HEADER_LEN + self.record_count * RECORD_LEN

    @property
    def file_length(self) -> int:
        """ヘッダの記録数と組の総数から決まるファイル長。"""
        return self.policy_offset + self.policy_count * POLICY_ENTRY_LEN


def decode_header(raw: bytes) -> Header:
    """ヘッダの176バイトを検証して復号する。ファイル長の照合は行わない。

    Raises:
        MnztError: 識別子、版、長さ、予約欄、教師の種類、文字列欄、または位置が不正な場合。
    """
    if len(raw) != HEADER_LEN:
        raise MnztError(f"header must be {HEADER_LEN} bytes, got {len(raw)}")
    magic, version, header_length, record_length = struct.unpack_from("<4sIII", raw, 0)
    if magic != MAGIC:
        raise MnztError(f"invalid file magic {magic!r}")
    if version != FORMAT_VERSION:
        raise MnztError(f"unsupported format version {version}")
    if header_length != HEADER_LEN:
        raise MnztError(f"unsupported header length {header_length}")
    if record_length != RECORD_LEN:
        raise MnztError(f"unsupported record length {record_length}")
    rule_field = raw[16:48]
    rule_end = rule_field.find(b"\0")
    if rule_end < 0:
        rule_end = len(rule_field)
    if any(rule_field[rule_end:]):
        raise MnztError("rule set name is not NUL padded")
    try:
        rule_set = rule_field[:rule_end].decode("utf-8")
    except UnicodeDecodeError as error:
        raise MnztError("rule set name is not UTF-8") from error
    commit_field = raw[48:88]
    if not all(chr(byte) in "0123456789abcdefABCDEF" for byte in commit_field):
        raise MnztError("generation commit is not a 40-digit hexadecimal hash")
    teacher_code = raw[120]
    if teacher_code >= len(TEACHER_KINDS):
        raise MnztError(f"invalid teacher kind code {teacher_code}")
    if any(raw[121:128]):
        raise MnztError("header reserved bytes must all be zero")
    teacher_value, seed, record_count, policy_count, table_offset, policy_offset = (
        struct.unpack_from("<6Q", raw, 128)
    )
    header = Header(
        rule_set=rule_set,
        generation_commit=commit_field.decode("ascii"),
        network_checksum=bytes(raw[88:120]),
        teacher_kind=TEACHER_KINDS[teacher_code],
        teacher_value=teacher_value,
        seed=seed,
        record_count=record_count,
        policy_count=policy_count,
    )
    if table_offset != HEADER_LEN:
        raise MnztError(f"invalid position-table offset: expected {HEADER_LEN}, got {table_offset}")
    if policy_offset != header.policy_offset:
        raise MnztError(
            f"invalid policy-array offset: expected {header.policy_offset}, got {policy_offset}"
        )
    return header


def sha256_file(path: Path) -> str:
    """ファイルのSHA-256を16進文字列で返す。"""
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        while chunk := stream.read(1 << 22):
            digest.update(chunk)
    return digest.hexdigest()


class MnztFile:
    """検証済みのMNZTファイル。局面の表`records`と方策の列`policy`を`memmap`で持つ。

    `pickle`では経路とヘッダだけを渡し、復元先で再検証せずに`memmap`を開き直す。
    これにより`DataLoader`のワーカーへ渡しても、ファイルの内容をコピーしない。
    """

    def __init__(self, path: Path) -> None:
        """ヘッダ、ファイル長、および全レコードと全組を検証して開く。

        Raises:
            MnztError: ファイルが壊れている場合。
        """
        self.path = Path(path)
        with self.path.open("rb") as stream:
            raw = stream.read(HEADER_LEN)
        try:
            self.header = decode_header(raw)
        except MnztError as error:
            raise MnztError(f"{self.path}: {error}") from error
        actual_length = self.path.stat().st_size
        if actual_length != self.header.file_length:
            raise MnztError(
                f"{self.path}: file length mismatch: expected {self.header.file_length}, "
                f"got {actual_length}"
            )
        self._map()
        try:
            self._validate()
        except MnztError as error:
            raise MnztError(f"{self.path}: {error}") from error

    def _map(self) -> None:
        """局面の表と方策の列を読み取り専用の`memmap`で開く。空の配列は`memmap`にできないので通常の配列にする。"""
        header = self.header
        if header.record_count == 0:
            self.records = np.zeros(0, dtype=RECORD_DTYPE)
        else:
            self.records = np.memmap(
                self.path, dtype=RECORD_DTYPE, mode="r", offset=HEADER_LEN,
                shape=(header.record_count,),
            )
        if header.policy_count == 0:
            self.policy = np.zeros(0, dtype=POLICY_DTYPE)
        else:
            self.policy = np.memmap(
                self.path, dtype=POLICY_DTYPE, mode="r", offset=header.policy_offset,
                shape=(header.policy_count,),
            )

    def _validate(self) -> None:
        """全レコードの欄と全組のラベルを、Rust側の`Record::decode`と`PolicyEntry::decode`と同じ条件で検査する。"""
        records = self.records
        for begin in range(0, len(records), _VALIDATION_CHUNK):
            chunk = np.asarray(records[begin:begin + _VALIDATION_CHUNK])
            bad = ~_VALID_PIECE_CODE[chunk["board"]].all(axis=1)
            _reject(bad, begin, "invalid piece code")
            _reject(chunk["side_to_move"] > 1, begin, "invalid side to move")
            lion = chunk["lion_square"]
            _reject((lion >= SQUARE_COUNT) & (lion != NO_LION_SQUARE), begin, "invalid lion square")
            kirin = chunk["lion_by_kirin_promotion"]
            _reject(kirin > 1, begin, "invalid lion promotion flag")
            _reject((kirin == 1) & (lion == NO_LION_SQUARE), begin, "lion promotion without square")
            _reject(chunk["outcome"] > 2, begin, "invalid outcome code")
            value = chunk["value"]
            _reject(~((value >= 0.0) & (value <= 1.0)), begin, "root value outside [0, 1]")
            end = chunk["policy_start"].astype(np.uint64) + chunk["policy_count"]
            overflow = end < chunk["policy_start"]
            _reject(overflow | (end > self.header.policy_count), begin, "policy range out of bounds")
        for begin in range(0, len(self.policy), _VALIDATION_CHUNK):
            labels = np.asarray(self.policy["label"][begin:begin + _VALIDATION_CHUNK])
            bad = labels >= POLICY_SIZE
            if bad.any():
                index = begin + int(np.flatnonzero(bad)[0])
                raise MnztError(f"policy entry {index}: label {int(labels[index - begin])} out of range")

    def policy_of(self, index: int) -> tuple[np.ndarray, np.ndarray]:
        """レコード`index`の方策の（ラベル`int64`、確率`float32`）の配列を返す。"""
        record = self.records[index]
        start = int(record["policy_start"])
        entries = np.asarray(self.policy[start:start + int(record["policy_count"])])
        return (
            entries["label"].astype(np.int64),
            entries["probability"].astype(np.float32) / np.float32(PROBABILITY_SCALE),
        )

    def __getstate__(self) -> dict:
        """`memmap`を除いた状態を返す。"""
        return {"path": self.path, "header": self.header}

    def __setstate__(self, state: dict) -> None:
        """経路とヘッダから`memmap`を開き直す。"""
        self.path = state["path"]
        self.header = state["header"]
        self._map()


def _reject(bad: np.ndarray, begin: int, message: str) -> None:
    """`bad`に真の要素があれば、最初のレコード番号を添えて`MnztError`を送出する。"""
    if bad.any():
        raise MnztError(f"record {begin + int(np.flatnonzero(bad)[0])}: {message}")


class MnztCollection:
    """複数のMNZTファイルを連結した局面の列。

    大域的なレコード番号は、ファイルの指定順にレコードを並べた通し番号である。
    対局は（ファイル番号、対局番号）の組で識別する。
    """

    def __init__(self, files: list[MnztFile]) -> None:
        if not files:
            raise ValueError("at least one MNZT file is required")
        self.files = files
        counts = np.array([file.header.record_count for file in files], dtype=np.int64)
        self.offsets = np.concatenate([[0], np.cumsum(counts)])
        """ファイルごとの最初のレコードの大域番号と、末尾に総数を並べた配列。"""

    @classmethod
    def open(cls, paths: list[Path]) -> MnztCollection:
        """各ファイルを検証して開く。"""
        return cls([MnztFile(Path(path)) for path in paths])

    def __len__(self) -> int:
        """総レコード数を返す。"""
        return int(self.offsets[-1])

    def locate(self, indices: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
        """大域番号の配列を（ファイル番号、ファイル内の番号）の配列へ変換する。"""
        indices = np.asarray(indices, dtype=np.int64)
        file_indices = np.searchsorted(self.offsets, indices, side="right") - 1
        return file_indices, indices - self.offsets[file_indices]

    def game_keys(self) -> np.ndarray:
        """各レコードの対局の識別子`ファイル番号 << 32 | 対局番号`を大域番号順に返す。"""
        keys = [
            (np.int64(file_index) << 32) | np.asarray(file.records["game_number"], dtype=np.int64)
            for file_index, file in enumerate(self.files)
        ]
        return np.concatenate(keys)
