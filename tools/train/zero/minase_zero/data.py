"""リプレイバッファから訓練局面を抽出し、各ランクが使う局面を入力順に保持する。

訓練局面は価値と方策の候補の両方を持ち、除外されていない項目とする。
指した手を確率1とする方策も候補に含める。
"""

from collections.abc import Sequence
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, fields
from pathlib import Path

import minase_mnsd_reader
import numpy as np
from numpy.typing import NDArray

from .encoding import BOARD_SQUARES, INPUT_PLANES, policy_labels


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


def is_training_sample(seed: int, games: NDArray[np.integer]) -> NDArray[np.bool_]:
    """ハッシュを20で割った余りが1の対局を、訓練側の診断の標本にする。"""
    return hash64(seed, games) % np.uint64(20) == 1


@dataclass(frozen=True)
class Header:
    """拡張モジュールが検査したヘッダと、ファイル全体で共通の由来。"""

    origin: int | None
    teacher_kind: int
    rule_set: str
    lishogi: bool
    generation_commit: str
    network_checksum: bytes
    teacher_search_budget: int
    seed: int
    game_count: int
    entry_count: int


@dataclass(frozen=True)
class FileInfo:
    """正規化したパス、バイト数、ヘッダ、抽出前の局面数、および全対局番号を保持する。

    game_numbersには選別後の局面が0件の対局も含み、リプレイバッファでの重複検査に使う。
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
    """保持した局面の入力順の配列と、平坦に並べた方策の候補と全合法手。

    boardsはuint8の(n,144)、sidesとlionsはuint8、valuesは量子化したままのint32、
    resultsは手番側の勝ち1・引き分け0.5・負け0を表すfloat32である。
    validationとtraining_sampleは対局単位の検証集合と訓練診断標本を表すboolである。
    gamesはuint32、pliesはuint16、file_indicesは入力順の0始まりのuint32である。
    policy_offsetsはuint64の長さn+1、policy_labelsとpolicy_probabilitiesはuint16である。
    legal_offsetsはuint64の長さn+1、legal_labelsはuint16で、反復の履歴を考えない
    全合法手を保持する。平均81手なら100万局面で約170 MB増える計算になる。
    1ファイルだけを読む場合もfile_indicesを0として同じ構造で返す。
    """

    boards: NDArray[np.uint8]
    sides: NDArray[np.uint8]
    lions: NDArray[np.uint8]
    values: NDArray[np.int32]
    results: NDArray[np.float32]
    validation: NDArray[np.bool_]
    training_sample: NDArray[np.bool_]
    games: NDArray[np.uint32]
    plies: NDArray[np.uint16]
    file_indices: NDArray[np.uint32]
    policy_offsets: NDArray[np.uint64]
    policy_labels: NDArray[np.uint16]
    policy_probabilities: NDArray[np.uint16]
    legal_offsets: NDArray[np.uint64]
    legal_labels: NDArray[np.uint16]

    def __len__(self) -> int:
        """選別後の局面数を返す。"""
        return len(self.games)


def read_file(path: str | Path) -> tuple[Dataset, FileInfo]:
    """MCTS・乱数開始の自己対局・lishogiの1ファイルを読み込む。

    成り権の保留、候補や合法手の欠落、確率和の不一致もValueErrorとする。
    規則名の別表記は、解釈した規則がlishogiと一致すれば受け付ける。
    0局のファイルは由来を問わず、空の局面配列と区切り[0]を返す。
    """
    path = Path(path).resolve(strict=True)
    raw = minase_mnsd_reader.read(str(path))
    info = _training_file_info(path, raw)
    return _dataset(raw, info), info


def _file_info(path: Path, raw: dict) -> FileInfo:
    """拡張モジュールのヘッダと件数を、用途による選別なしで保持する。"""
    header = Header(**{field.name: raw[field.name] for field in fields(Header)})
    return FileInfo(
        path=path, size=path.stat().st_size, header=header,
        position_count=raw["training_count"] + raw["validation_count"],
        training_count=raw["training_count"], validation_count=raw["validation_count"],
        game_numbers=tuple(np.frombuffer(raw["game_numbers"], dtype="<u4").tolist()),
    )


def _training_file_info(path: Path, raw: dict) -> FileInfo:
    info = _file_info(path, raw)
    header = info.header
    if header.teacher_kind != 1:
        raise ValueError(f"{path}: teacher must be MCTS")
    if header.game_count != 0 and header.origin != 0:
        raise ValueError(f"{path}: origin must be random selfplay")
    if not header.lishogi:
        raise ValueError(f"{path}: rules must equal lishogi")
    return info


def _dataset(raw: dict, info: FileInfo) -> Dataset:
    path, header = info.path, info.header
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
        raise ValueError(f"{path}: deferred promotion cannot be encoded in {INPUT_PLANES} planes")
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
    legal_offsets = np.frombuffer(raw["legal_offsets"], dtype="<u8")
    legal_counts = np.diff(legal_offsets)
    if np.any(legal_counts == 0):
        raise ValueError(f"{path}: every selected position must have legal moves")
    legal_labels = policy_labels(
        np.frombuffer(raw["legal_from"], dtype=np.uint8),
        np.frombuffer(raw["legal_mid"], dtype=np.uint8),
        np.frombuffer(raw["legal_to"], dtype=np.uint8),
        np.frombuffer(raw["legal_promote"], dtype=np.uint8),
        np.repeat(sides, legal_counts.astype(np.intp)),
    ).astype(np.uint16)
    results = np.where(outcomes == 2, 0.5, outcomes == sides).astype(np.float32)
    validation = is_validation(header.seed, games)
    return Dataset(
        boards=boards, sides=sides, lions=lions, values=values, results=results,
        validation=validation, training_sample=is_training_sample(header.seed, games), games=games, plies=plies,
        file_indices=np.zeros(len(games), dtype=np.uint32),
        policy_offsets=offsets, policy_labels=labels, policy_probabilities=probabilities,
        legal_offsets=legal_offsets, legal_labels=legal_labels,
    )


def sample_training_indices(total: int, count: int, seed: int) -> NDArray[np.int64]:
    """[0,total)から重複なく選んだcount個の更新列を返す。countはtotal以下とする。

    専用のGenerator(PCG64(seed))を使い、A[j]=jから始める。
    k=0,...,count-1でj=integers(k,total,dtype=int64)を引き、
    A[k]とA[j]を交換してA[k]を出力する部分的なFisher–Yates法である。
    未使用のAのうち動かした要素だけを辞書に保持する。
    """
    if not (1 <= count <= np.iinfo(np.int64).max and 1 <= total <= np.iinfo(np.int64).max):
        raise ValueError("sample count and total must be positive and fit in int64")
    if count > total:
        raise ValueError("sample count must not exceed total")
    if not 0 <= seed <= 0xFFFF_FFFF_FFFF_FFFF:
        raise ValueError("seed must fit in uint64")
    rng = np.random.Generator(np.random.PCG64(seed))
    selected = np.empty(count, dtype=np.int64)
    moved: dict[int, int] = {}
    for index in range(count):
        other = int(rng.integers(index, total, dtype=np.int64))
        selected[index] = moved[other] if other in moved else other
        replacement = moved.pop(index) if index in moved else index
        if other != index:
            moved[other] = replacement
    return selected


def _scan_file(path: Path) -> FileInfo:
    return _training_file_info(path, minase_mnsd_reader.scan(str(path)))


def _read_selected(
    info: FileInfo, indices: list[int], diagnostic_indices: list[int], include_validation: bool,
) -> tuple[Dataset, NDArray[np.int64]]:
    raw = minase_mnsd_reader.read_selected(str(info.path), indices, diagnostic_indices, include_validation)
    if _training_file_info(info.path, raw) != info:
        raise ValueError(f"{info.path}: file metadata changed between counting and replay")
    dataset = _dataset(raw, info)
    retained = np.frombuffer(raw["training_indices"], dtype="<u8").astype(np.int64)
    validation_count = info.validation_count if include_validation else 0
    if len(dataset) != len(retained) + validation_count or np.count_nonzero(dataset.validation) != validation_count:
        raise ValueError(f"{info.path}: retained position counts differ from the selection")
    return dataset, retained


def read_replay_buffer(
    paths: Sequence[str | Path], latest_paths: Sequence[str | Path], seed: int,
    *, samples: str | int, rank: int, world_size: int, batch_size: int, micro_batch_size: int, replay_threads: int,
) -> tuple[Dataset, list[FileInfo], NDArray[np.int64], NDArray[np.int64]]:
    """担当する局面、全ファイル情報、抽出列、および保持した訓練局面の通し番号を返す。

    latest_pathsはpathsの末尾と文字列で一致する必要がある。
    samplesがlatestなら最新の訓練局面数Lを、正の整数なら指定数を、
    全訓練局面の通し番号[0,T)から重複なく抽出する。指定数はT以下とする。
    計数は再生なしで行い、再生はGILを解放してファイルごとに並列に1回行う。
    抽出列の端数を捨て、ミクロバッチの番号をworld_sizeで割った余りがrankの局面を保持する。
    ランク0は、端数を除いた抽出列の訓練診断標本と、最新の検証集合も保持する。
    第4戻り値は、保持した非検証局面の行に対応する、リプレイバッファ全体の通し番号である。
    配列は重複を除いてファイル順に保持し、抽出数がbatch_size未満なら再生前にエラーにする。
    正規化後のパス、または基本シードと対局番号の組が重複すればValueErrorとする。
    """
    if samples != "latest" and (type(samples) is not int or samples <= 0):
        raise ValueError("samples must be 'latest' or a positive integer")
    if not paths:
        raise ValueError("at least one MNSD file is required")
    if not latest_paths or [str(path) for path in paths[-len(latest_paths):]] != [str(path) for path in latest_paths]:
        raise ValueError("latest-data must be a nonempty suffix of data with identical path strings and order")
    normalized = [Path(path).resolve(strict=True) for path in paths]
    if len(set(normalized)) != len(normalized):
        raise ValueError("duplicate normalized MNSD path in replay buffer")
    latest_start = len(paths) - len(latest_paths)
    with ThreadPoolExecutor(max_workers=min(len(paths), replay_threads)) as pool:
        infos = list(pool.map(_scan_file, normalized))
        seen_games: set[tuple[int, int]] = set()
        for info in infos:
            for game in info.game_numbers:
                key = (info.header.seed, game)
                if key in seen_games:
                    raise ValueError(f"{info.path}: duplicate game (seed={key[0]}, game={key[1]})")
                seen_games.add(key)
        # latestは、AobaZero 5eb9441のlearn/yss_dcnn.cppのADJUSTと、Gumbel dlshogi 5538f7bの
        # 直近1サイクルを1周学ぶ既定に倣い、新規訓練局面と同じ数だけ抽出する。
        count = sum(info.training_count for info in infos[latest_start:]) if samples == "latest" else samples
        total = sum(info.training_count for info in infos)
        if count > total:
            raise ValueError(f"sample count {count} must not exceed replay buffer training count {total}")
        if count < batch_size:
            raise ValueError("sample count must be at least batch_size for one full update")
        selected = sample_training_indices(total, count, seed)
        used = selected[:len(selected) // batch_size * batch_size]
        ordered = np.unique(used.reshape(-1, micro_batch_size)[rank::world_size])
        diagnostic = np.unique(used) if rank == 0 else np.empty(0, dtype=np.int64)
        futures = []
        offsets = []
        offset = 0
        for index, info in enumerate(infos):
            end = offset + info.training_count
            first, last = np.searchsorted(ordered, [offset, end])
            indices = (ordered[first:last] - offset).tolist()
            first, last = np.searchsorted(diagnostic, [offset, end])
            diagnostic_indices = (diagnostic[first:last] - offset).tolist()
            futures.append(pool.submit(_read_selected, info, indices, diagnostic_indices,
                                       rank == 0 and index >= latest_start))
            offsets.append(offset)
            offset = end
        results = [future.result() for future in futures]
        datasets = [data for data, _ in results]
        retained = np.concatenate([indices + offset for (_, indices), offset in zip(results, offsets, strict=True)])
    prefixes = ("policy", "legal")
    offset_parts = {prefix: [] for prefix in prefixes}
    counts = dict.fromkeys(prefixes, 0)
    for index, dataset in enumerate(datasets):
        dataset.file_indices.fill(index)
        for prefix in prefixes:
            offset_parts[prefix].append(getattr(dataset, f"{prefix}_offsets")[:-1] + np.uint64(counts[prefix]))
            counts[prefix] += len(getattr(dataset, f"{prefix}_labels"))
    columns = {
        field.name: np.concatenate([getattr(dataset, field.name) for dataset in datasets])
        for field in fields(Dataset) if field.name not in {f"{prefix}_offsets" for prefix in prefixes}
    }
    for prefix in prefixes:
        columns[f"{prefix}_offsets"] = np.concatenate(
            [*offset_parts[prefix], np.array([counts[prefix]], dtype=np.uint64)]
        )
    return Dataset(**columns), infos, selected, retained
