"""学習ツールのテストで共有する人工データと独立した参照計算を提供する。"""

from __future__ import annotations

import hashlib
import json
import struct
from fractions import Fraction
from pathlib import Path

import numpy as np
import torch

import minase_train.pst.model as pst_model
import minase_train.pst.removal as removal
from minase_train.data.features import INITIAL_BOARD, NO_LION_SQUARE, feature_indices
from minase_train.data.mnpt import initial_piece_values
from minase_train.data.mnsd import (
    HEADER_LENGTH,
    RECORD_DTYPE,
    RECORD_LENGTH,
    map_records,
    provenance_path,
    read_header,
)
from minase_train.diagnostics.comparison import Weights


def write_rescore(path, source, rows, *, nodes=200000, commit="a" * 40):
    """共有契約第1節のオフセットと16バイトの記録から直接構築する。"""
    header = bytearray(240)
    struct.pack_into("<4sI", header, 0, b"MNRS", 1)
    header[8:40] = hashlib.sha256(source.read_bytes()).digest()
    struct.pack_into("<Q", header, 40, len(rows))
    targets = b"".join(struct.pack("<Q", i) for i, row in enumerate(rows) if row[0])
    header[48:80] = hashlib.sha256(targets).digest()
    struct.pack_into("<Q", header, 80, len(targets) // 8)
    header[88:120] = b"a" * 32
    struct.pack_into("<I", header, 120, nodes)
    header[124:156] = b"L0,P0,R1,E0".ljust(32, b"\0")
    struct.pack_into("<I", header, 156, 16)
    header[164:204] = commit.encode("ascii")
    header[204:236] = b"b" * 32
    path.write_bytes(header + b"".join(struct.pack("<BBhIQ", *row) for row in rows))


PIECE_VALUES = initial_piece_values()


def python_probe(mnpt: Path, mnsd: Path, promotions: bool) -> list[dict]:
    """参照評価で応答し、成り後の応答には先手の成金1枚だけの固定局面を使う。"""
    weights = Weights(mnpt)
    records = map_records(mnsd)
    scores = weights.evaluate(records)
    after = np.zeros(len(records), dtype=RECORD_DTYPE)
    after["board"][:, 0] = 47  # MNSD: 1 + 成駒29 + 金将17。
    after["stm"] = 1 - records["stm"]
    after["lion"] = NO_LION_SQUARE
    after_scores = weights.evaluate(after)
    return [
        {"index": index, "eval": int(score),
         **({"promotions": [{
             "move": "1a1b+", "delta": -int(after_scores[index]) - int(score),
             "after": {"board": after[index]["board"].tolist(),
                       "stm": int(after[index]["stm"]), "lion": int(after[index]["lion"])},
         }]} if promotions else {})}
        for index, score in enumerate(scores)
    ]


def write_mnsd(
    path: Path,
    *,
    seed: int,
    checksum: bytes,
    games: list[int],
    scores: list[int] | None = None,
    results: list[int] | None = None,
    board: np.ndarray | None = None,
) -> None:
    """テスト用の最小MNSDファイルを書く。"""
    count = len(games)
    records = np.zeros(count, dtype=RECORD_DTYPE)
    records["board"] = INITIAL_BOARD if board is None else board
    records["lion"] = 255
    records["game"] = games
    records["ply"] = np.arange(count, dtype=np.uint16)
    records["score"] = scores if scores is not None else np.arange(count)
    records["result"] = results if results is not None else np.arange(count) % 3

    header = bytearray(HEADER_LENGTH)
    struct.pack_into("<4sII", header, 0, b"MNSD", 1, RECORD_LENGTH)
    header[12:44] = b"L0,P0,R1,E0".ljust(32, b"\0")
    header[44:84] = b"0" * 40
    header[84:116] = checksum
    struct.pack_into("<IQQ", header, 116, 100_000, seed, count)
    path.write_bytes(bytes(header) + records.tobytes())
    write_provenance(path)


def write_provenance(path: Path, **changes):
    """契約第3節の明示的なテスト来歴を書く。"""
    from minase_train.data.mnsd import read_header
    header = read_header(path)
    value = {
        "format": "minase-provenance", "version": 1,
        "mnsd_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
        "teacher": {"generation_commit": header.generation_commit,
                    "network_checksum": header.network_checksum.hex(),
                    "nodes": header.teacher_nodes, "rule_set": header.rule_set,
                    "search_condition": "in-game"},
        "result_origin": "selfplay", "start_origin": "random", "lambda": 0.75, "games": None,
    }
    value.update(changes)
    Path(str(path) + ".provenance.json").write_text(json.dumps(value))
    return value


PROMOTABLE = (0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16, 17, 18, 19)


CPU = torch.device("cpu")


def decoded(byte: int, square: int, stm: int) -> tuple[int, int, int]:
    """MNSDの公開byte形式から状態・相対陣営・特徴番号を直接求める。"""
    color = int(byte >= 65)
    payload = byte - 1 - 64 * color
    kind = payload % 29
    state = 29 + PROMOTABLE.index(kind) if payload < 29 and kind in PROMOTABLE else kind
    relative = int(color != stm)
    mapped = square if stm == 0 else (11 - square // 12) * 12 + square % 12
    return state, relative, (relative * 47 + state) * 144 + mapped


def direct_score(record: np.void, weights_cp: np.ndarray) -> Fraction:
    """両端点の和を盤面から毎回作り、未丸め評価を厳密に求める。"""
    features = [decoded(int(byte), square, int(record["stm"]))[2]
                for square, byte in enumerate(record["board"]) if byte]
    if record["lion"] != 255:
        square = int(record["lion"])
        mapped = square if record["stm"] == 0 else (11 - square // 12) * 12 + square % 12
        features.append(13536 + mapped)
    mg = sum((Fraction(float(weights_cp[i, 0])) for i in features), Fraction())
    eg = sum((Fraction(float(weights_cp[i, 1])) for i in features), Fraction())
    q = min(90, max(0, int(np.count_nonzero(record["board"])) - 2))
    return (q * mg + (90 - q) * eg) / 90


def integer_score(value: Fraction) -> int:
    return min(28999, max(-28999, int(value)))


def direct_loss(records: np.ndarray, base_i16: np.ndarray, candidate_cp: np.ndarray, k: int) -> Fraction:
    """式の2項と等局面平均を、1枚ずつ実際に消した盤面から計算する。"""
    base_cp = base_i16.astype(np.float64) / 8
    per_position = []
    for record in records:
        before_base = direct_score(record, base_cp)
        before_candidate = direct_score(record, candidate_cp)
        losses = []
        for square, byte in enumerate(record["board"]):
            if not byte:
                continue
            state, relative, _ = decoded(int(byte), square, int(record["stm"]))
            if state in (11, 21):
                continue
            after = record.copy()
            after["board"][square] = 0
            after_base = direct_score(after, base_cp)
            integer_delta = integer_score(after_base) - integer_score(before_base)
            if integer_delta == 0:
                continue
            sign = 1 if integer_delta > 0 else -1
            base_delta = after_base - before_base
            candidate_delta = direct_score(after, candidate_cp) - before_candidate
            margin = min(abs(base_delta), Fraction(1, 4))
            lower = max(Fraction(), margin - sign * candidate_delta)
            bad_base = (relative == 0 and integer_delta > 0) or (relative == 1 and integer_delta < 0)
            upper = max(Fraction(), sign * candidate_delta - abs(base_delta)) if bad_base else Fraction()
            losses.append((lower + upper) / k)
        per_position.append(sum(losses, Fraction()) / len(losses) if losses else Fraction())
    return sum(per_position, Fraction()) / len(per_position)


def records_with(pieces: list[list[tuple[int, int]]], *, stm: int = 0, lion: int = 255) -> np.ndarray:
    records = np.zeros(len(pieces), dtype=RECORD_DTYPE)
    records["stm"] = stm
    records["lion"] = lion
    for record, positions in zip(records, pieces):
        for square, byte in positions:
            record["board"][square] = byte
    return records


def constant_base(own: int = 8, enemy: int = -8) -> np.ndarray:
    weights = np.zeros((13680, 2), dtype=np.int16)
    for relative, value in [(0, own), (1, enemy)]:
        for state in range(47):
            if state not in (11, 21):
                start = (relative * 47 + state) * 144
                weights[start:start + 144] = value
    return weights


def model_and_loss(records, base, candidate, k=2):
    model = pst_model.make_model(torch.tensor(candidate, dtype=torch.float64), CPU, "mirrored").double()
    # Explicit assignment avoids depending on initialization's floating dtype policy.
    canonical = candidate.reshape(95, 12, 12, 2)[:, :, :6].reshape(6840, 2)
    with torch.no_grad():
        model.weight[:6840].copy_(torch.from_numpy(canonical))
    indices = torch.from_numpy(feature_indices(records["board"], records["stm"], records["lion"]))
    counts = torch.tensor(np.count_nonzero(records["board"], axis=1), device=CPU)
    reference = removal.make_removal_reference(base[:, 0], base[:, 1], CPU)
    loss = removal.removal_loss(model(indices), indices, reference, counts, k)
    return model, loss


class ProvenanceFixtures:
    """来歴の検証と教師値のテストで人工の棋譜情報を共有する。"""

    def metadata(self, **changes):
        value = dict(self.provenance, **changes)
        provenance_path(self.source).write_text(json.dumps(value))

    def human_games(self):
        return [{"game": i, "id": f"game-{i}", "ply": 40} for i in range(1, 101)]
