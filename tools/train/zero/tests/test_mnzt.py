"""MNZTの読み込みが、Rustの書き出した形式と一致し、壊れたファイルを拒否することを検査する。"""

from __future__ import annotations

import json
import pickle
import struct

import numpy as np
import pytest
from conftest import INITIAL_SFEN, game_records, write_mnzt

from minase_zero.encoding import POLICY_PLANES, encode, parse_sfen, relative_square
from minase_zero.mnzt import (
    HEADER_LEN,
    RECORD_LEN,
    MnztCollection,
    MnztError,
    MnztFile,
    record_position,
    sha256_file,
)

MNSD_HEADER_LEN = 136
MNSD_RECORD_DTYPE = np.dtype(
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
"""同じ生成で書かれるMNSD（crates/minase/src/training/records.rs）のレコード。"""


def test_sample_header_matches_generation_provenance(sample_mnzt):
    """見本のヘッダが、同じ生成で書かれたMNSDの来歴とヘッダに一致し、記録数が方策の個数と整合する。"""
    data = MnztFile(sample_mnzt)
    header = data.header
    provenance = json.loads(sample_mnzt.with_suffix(".mnsd.provenance.json").read_text())
    teacher = provenance["teacher"]
    assert header.rule_set == teacher["rule_set"]
    assert header.generation_commit == teacher["generation_commit"]
    assert header.teacher_kind == "alphabeta_nodes"
    assert header.teacher_value == teacher["nodes"]
    mnsd_header = sample_mnzt.with_suffix(".mnsd").read_bytes()[:MNSD_HEADER_LEN]
    assert header.seed == struct.unpack_from("<Q", mnsd_header, 120)[0]
    assert header.network_checksum == bytes(32)
    assert len(data.records) == header.record_count > 0
    assert int(data.records["policy_count"].astype(np.int64).sum()) == header.policy_count
    assert sample_mnzt.stat().st_size == HEADER_LEN + RECORD_LEN * header.record_count + 4 * header.policy_count


def test_sample_records_agree_with_mnsd(sample_mnzt):
    """MNSDの各局面が、同じ対局番号と手数のMNZTの局面と盤面、手番、先獅子、および結果で一致する。"""
    records = MnztFile(sample_mnzt).records
    mnsd = np.fromfile(sample_mnzt.with_suffix(".mnsd"), dtype=MNSD_RECORD_DTYPE, offset=MNSD_HEADER_LEN)
    assert len(mnsd) > 0
    by_key = {(int(r["game_number"]), int(r["ply"])): r for r in records}
    for expected in mnsd:
        actual = by_key[(int(expected["game"]), int(expected["ply"]))]
        assert np.array_equal(actual["board"], expected["board"])
        assert actual["side_to_move"] == expected["stm"]
        assert actual["lion_square"] == expected["lion"]
        assert actual["lion_by_kirin_promotion"] == expected["kirin"]
        assert actual["outcome"] == expected["result"]


def test_sample_policy_sums_to_one_and_starts_on_own_piece(sample_mnzt):
    """SLの見本は各局面に確率1の組を1個持ち、その手の移動元に手番側の駒がある。"""
    data = MnztFile(sample_mnzt)
    for index in range(len(data.records)):
        labels, probs = data.policy_of(index)
        assert len(labels) == 1
        assert probs.sum() == pytest.approx(1.0, abs=1e-6)
        position = record_position(data.records[index])
        source = int(labels[0]) // POLICY_PLANES
        piece = position.board[relative_square(source, position.side_to_move)]
        assert piece is not None and piece.color == position.side_to_move


def test_synthetic_records_round_trip(tmp_path):
    """合成ファイルの欄、方策、および局面の符号化が、書いた値と一致する。"""
    sfen = "12/12/12/12/12/5n6/12/12/12/12/12/5K6 w 7f 9"
    records = [
        {"game": 3, "ply": 9, "outcome": 1, "value": 0.25, "policy": [(5, 0.5), (35855, 0.5)],
         "sfen": sfen},
        {"game": 4, "ply": 0, "outcome": 0, "value": 1.0, "policy": []},
    ]
    data = MnztFile(write_mnzt(tmp_path / "a.mnzt", records, seed=11))
    assert data.header.seed == 11
    assert data.header.record_count == 2
    assert data.header.policy_count == 2
    labels, probs = data.policy_of(0)
    assert labels.tolist() == [5, 35855]
    np.testing.assert_allclose(probs, [0.5, 0.5], atol=1 / 65535)
    assert data.policy_of(1)[0].tolist() == []
    first = data.records[0]
    assert (int(first["game_number"]), int(first["ply"]), int(first["outcome"])) == (3, 9, 1)
    np.testing.assert_array_equal(encode(record_position(first)), encode(parse_sfen(sfen)))
    np.testing.assert_array_equal(
        encode(record_position(data.records[1])), encode(parse_sfen(INITIAL_SFEN))
    )


def test_pickle_reopens_without_copying(tmp_path):
    """pickleで渡したファイルは同じ内容を`memmap`で読む。"""
    data = MnztFile(write_mnzt(tmp_path / "a.mnzt", game_records(1, 5)))
    restored = pickle.loads(pickle.dumps(data))
    assert isinstance(restored.records, np.memmap)
    np.testing.assert_array_equal(np.asarray(restored.records), np.asarray(data.records))


def test_collection_locates_records_and_distinguishes_files(tmp_path):
    """連結した番号がファイルとファイル内の番号へ戻り、同じ対局番号でもファイルが違えば別の対局になる。"""
    first = write_mnzt(tmp_path / "a.mnzt", game_records(1, 3))
    second = write_mnzt(tmp_path / "b.mnzt", game_records(1, 2))
    collection = MnztCollection.open([first, second])
    assert len(collection) == 5
    files, local = collection.locate(np.array([0, 2, 3, 4]))
    assert files.tolist() == [0, 0, 1, 1]
    assert local.tolist() == [0, 2, 0, 1]
    assert len(np.unique(collection.game_keys())) == 2


def test_sha256_matches_hashlib(tmp_path):
    """検査和は内容のSHA-256である。"""
    path = tmp_path / "x.bin"
    path.write_bytes(b"abc")
    assert sha256_file(path) == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"


def corrupt(path, offset: int, data: bytes) -> None:
    """ファイルの`offset`から`data`で上書きする。"""
    raw = bytearray(path.read_bytes())
    raw[offset:offset + len(data)] = data
    path.write_bytes(bytes(raw))


RECORD = HEADER_LEN
"""最初のレコードの開始位置。"""


@pytest.mark.parametrize(
    ("offset", "data", "message"),
    [
        (0, b"MNSD", "magic"),
        (4, struct.pack("<I", 2), "version"),
        (8, struct.pack("<I", 160), "header length"),
        (12, struct.pack("<I", 160), "record length"),
        (16, b"\xff", "UTF-8"),
        (48, b"g", "hexadecimal"),
        (120, b"\x02", "teacher kind"),
        (121, b"\x01", "reserved"),
        (160, struct.pack("<Q", 0), "position-table offset"),
        (168, struct.pack("<Q", 0), "policy-array offset"),
        (RECORD + 0, b"\x3b", "piece code"),
        (RECORD + 0, bytes([1 + 22]), "piece code"),
        (RECORD + 0, bytes([1 + 29 + 0]), "piece code"),
        (RECORD + 144, b"\x02", "side to move"),
        (RECORD + 145, b"\x90", "lion square"),
        (RECORD + 146, b"\x02", "lion promotion flag"),
        (RECORD + 146, b"\x01", "lion promotion without square"),
        (RECORD + 147, b"\x03", "outcome"),
        (RECORD + 156, struct.pack("<f", 1.5), "root value"),
        (RECORD + 156, struct.pack("<f", float("nan")), "root value"),
        (RECORD + 154, struct.pack("<H", 0xFFFF), "policy range"),
        (RECORD + 160, struct.pack("<Q", 2**64 - 1), "policy range"),
        (RECORD + 3 * RECORD_LEN, struct.pack("<H", 35856), "label"),
    ],
)
def test_corrupted_file_is_rejected(tmp_path, offset, data, message):
    """識別子、版、長さ、予約欄、位置、局面の欄、および方策のラベルの不正を拒否する。"""
    path = write_mnzt(tmp_path / "a.mnzt", game_records(1, 3))
    corrupt(path, offset, data)
    with pytest.raises(MnztError, match=message):
        MnztFile(path)


def test_truncated_and_extended_files_are_rejected(tmp_path):
    """ヘッダの記録数から決まる長さと異なるファイルを拒否する。"""
    path = write_mnzt(tmp_path / "a.mnzt", game_records(1, 3))
    raw = path.read_bytes()
    path.write_bytes(raw[:-1])
    with pytest.raises(MnztError, match="file length"):
        MnztFile(path)
    path.write_bytes(raw + b"\0")
    with pytest.raises(MnztError, match="file length"):
        MnztFile(path)
    path.write_bytes(raw[:100])
    with pytest.raises(MnztError, match="header"):
        MnztFile(path)
