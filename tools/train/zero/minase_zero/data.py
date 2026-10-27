"""MCTSの教師データを選別し、疎な方策を保った学習窓を入力順に読み込む。"""

from collections.abc import Sequence
from dataclasses import dataclass, fields
from pathlib import Path

import minase_mnsd_reader
import numpy as np
from numpy.typing import NDArray

from .encoding import BOARD_SQUARES, policy_labels


def hash64(seed: int, games: NDArray[np.integer]) -> NDArray[np.uint64]:
    """Rustのshuffle::hash64と同じu64の折り返し演算で対局番号を混合する。"""
    if not 0 <= seed <= 0xFFFF_FFFF_FFFF_FFFF:
        raise ValueError("seed must fit in uint64")
    if games.dtype.kind not in "iu" or np.any((games < 0) | (games > 0xFFFF_FFFF)):
        raise ValueError("game numbers must be uint32 integers")
    # 配列のuint64演算は、Rustのwrapping_mulと同じく2の64乗を法とする。
    value = np.uint64(seed) ^ (games.astype(np.uint64) * np.uint64(0x9E37_79B9_7F4A_7C15))
    value = (value ^ (value >> np.uint64(30))) * np.uint64(0xBF58_476D_1CE4_E5B9)
    value = (value ^ (value >> np.uint64(27))) * np.uint64(0x94D0_49BB_1331_11EB)
    return value ^ (value >> np.uint64(31))


def is_validation(seed: int, games: NDArray[np.integer]) -> NDArray[np.bool_]:
    """ヘッダのシードと対局番号のハッシュが20の倍数なら検証集合へ割り当てる。"""
    return hash64(seed, games) % np.uint64(20) == 0


@dataclass(frozen=True)
class Header:
    """拡張モジュールが検査したヘッダと、ファイル全体で共通の由来。"""

    origin: int | None
    teacher_kind: int
    rule_set: str
    engine_default: bool
    generation_commit: str
    network_checksum: bytes
    teacher_search_budget: int
    seed: int
    game_count: int
    entry_count: int


@dataclass(frozen=True)
class FileInfo:
    """正規化したパス、バイト数、ヘッダ、局面数、および全対局番号を保持する。

    game_numbersには選別後の局面が0件の対局も含み、学習窓での重複検査に使う。
    """

    path: Path
    size: int
    header: Header
    position_count: int
    training_count: int
    validation_count: int
    game_numbers: tuple[int, ...]


@dataclass(frozen=True)
class Dataset:
    """局面順の配列と、候補を平坦に並べた疎な方策。

    boardsはuint8の(n,144)、sidesとlionsはuint8、valuesは量子化したままのint32、
    resultsは手番側の勝ち1・引き分け0.5・負け0を表すfloat32、validationはboolである。
    gamesはuint32、pliesはuint16、file_indicesは入力順の0始まりのuint32である。
    policy_offsetsはuint64の長さn+1、policy_labelsとpolicy_probabilitiesはuint16である。
    1ファイルだけを読む場合もfile_indicesを0として同じ構造で返す。
    """

    boards: NDArray[np.uint8]
    sides: NDArray[np.uint8]
    lions: NDArray[np.uint8]
    values: NDArray[np.int32]
    results: NDArray[np.float32]
    validation: NDArray[np.bool_]
    games: NDArray[np.uint32]
    plies: NDArray[np.uint16]
    file_indices: NDArray[np.uint32]
    policy_offsets: NDArray[np.uint64]
    policy_labels: NDArray[np.uint16]
    policy_probabilities: NDArray[np.uint16]

    def __len__(self) -> int:
        """選別後の局面数を返す。"""
        return len(self.games)


def read_file(path: str | Path) -> tuple[Dataset, FileInfo]:
    """MCTS・乱数開始の自己対局・engine-defaultの1ファイルを読み込む。

    成り権の保留、候補の欠落、確率和の不一致もValueErrorとする。
    規則名の別表記は、解釈した規則がengine-defaultと一致すれば受け付ける。
    0局のファイルは由来を問わず、空の局面配列と区切り[0]を返す。
    """
    path = Path(path).resolve(strict=True)
    raw = minase_mnsd_reader.read(str(path))
    header = Header(**{field.name: raw[field.name] for field in fields(Header)})
    if header.teacher_kind != 1:
        raise ValueError(f"{path}: teacher must be MCTS")
    if header.game_count != 0 and header.origin != 0:
        raise ValueError(f"{path}: origin must be random selfplay")
    if not header.engine_default:
        raise ValueError(f"{path}: rules must equal engine-default")

    games = np.frombuffer(raw["games"], dtype="<u4")
    boards = np.frombuffer(raw["boards"], dtype=np.uint8).reshape(len(games), BOARD_SQUARES)
    sides = np.frombuffer(raw["sides"], dtype=np.uint8)
    lions = np.frombuffer(raw["lions"], dtype=np.uint8)
    values = np.frombuffer(raw["values"], dtype="<i4")
    plies = np.frombuffer(raw["plies"], dtype="<u2")
    outcomes = np.frombuffer(raw["outcomes"], dtype=np.uint8)
    offsets = np.frombuffer(raw["policy_offsets"], dtype="<u8")
    probabilities = np.frombuffer(raw["policy_probabilities"], dtype="<u2")
    if np.any(boards & 128):
        raise ValueError(f"{path}: deferred promotion cannot be encoded in 95 planes")
    counts = np.diff(offsets)
    if np.any(counts == 0):
        raise ValueError(f"{path}: every selected position must have policy candidates")
    # 累積和の差なら候補数が多い局面でもu16の加算であふれない。
    cumulative = np.empty(len(probabilities) + 1, dtype=np.uint64)
    cumulative[0] = 0
    np.cumsum(probabilities, dtype=np.uint64, out=cumulative[1:])
    if np.any(np.diff(cumulative[offsets]) != 65_535):
        raise ValueError(f"{path}: policy probabilities must sum to 65535 per position")
    candidate_sides = np.repeat(sides, counts.astype(np.intp))
    labels = policy_labels(
        np.frombuffer(raw["policy_from"], dtype=np.uint8),
        np.frombuffer(raw["policy_mid"], dtype=np.uint8),
        np.frombuffer(raw["policy_to"], dtype=np.uint8),
        np.frombuffer(raw["policy_promote"], dtype=np.uint8),
        candidate_sides,
    ).astype(np.uint16)
    results = np.where(outcomes == 2, 0.5, outcomes == sides).astype(np.float32)
    validation = is_validation(header.seed, games)
    dataset = Dataset(
        boards=boards, sides=sides, lions=lions, values=values, results=results,
        validation=validation, games=games, plies=plies,
        file_indices=np.zeros(len(games), dtype=np.uint32),
        policy_offsets=offsets, policy_labels=labels, policy_probabilities=probabilities,
    )
    validation_count = int(np.count_nonzero(validation))
    info = FileInfo(
        path=path, size=path.stat().st_size, header=header, position_count=len(games),
        training_count=len(games) - validation_count, validation_count=validation_count,
        game_numbers=tuple(np.frombuffer(raw["game_numbers"], dtype="<u4").tolist()),
    )
    return dataset, info


def read_window(paths: Sequence[str | Path]) -> tuple[Dataset, list[FileInfo]]:
    """指定順に1つ以上のファイルを連結し、局面の列とファイル情報の列を返す。

    正規化後のパス、または基本シードと対局番号の組が重複すればValueErrorとする。
    同じ基本シードの塊でも対局番号が重ならなければ受け付ける。
    """
    if not paths:
        raise ValueError("at least one MNSD file is required")
    normalized = [Path(path).resolve(strict=True) for path in paths]
    if len(set(normalized)) != len(normalized):
        raise ValueError("duplicate normalized MNSD path in training window")
    datasets = []
    infos = []
    seen_games: set[tuple[int, int]] = set()
    offset_parts = []
    candidate_count = 0
    for index, path in enumerate(normalized):
        dataset, info = read_file(path)
        for game in info.game_numbers:
            key = (info.header.seed, game)
            if key in seen_games:
                raise ValueError(f"{path}: duplicate game (seed={key[0]}, game={key[1]})")
            seen_games.add(key)
        dataset.file_indices.fill(index)
        offset_parts.append(dataset.policy_offsets[:-1] + np.uint64(candidate_count))
        candidate_count += len(dataset.policy_labels)
        datasets.append(dataset)
        infos.append(info)
    columns = {
        field.name: np.concatenate([getattr(dataset, field.name) for dataset in datasets])
        for field in fields(Dataset) if field.name != "policy_offsets"
    }
    columns["policy_offsets"] = np.concatenate(
        [*offset_parts, np.array([candidate_count], dtype=np.uint64)]
    )
    return Dataset(**columns), infos
