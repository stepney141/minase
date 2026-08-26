"""MNSDストリーミング学習器の契約を単独実行で検証する。"""

from __future__ import annotations

from pathlib import Path
import struct
import tempfile
import unittest

import numpy as np
import torch

from features import INITIAL_BOARD
from mnsd import Dataset, HEADER_LENGTH, RECORD_DTYPE, RECORD_LENGTH, hash64
from train_pst import (
    build_targets,
    estimate_generation_ks,
    estimate_k,
    model_logits,
)


def write_mnsd(
    path: Path,
    *,
    seed: int,
    checksum: bytes,
    games: list[int],
    scores: list[int] | None = None,
    results: list[int] | None = None,
) -> None:
    """テスト用の最小MNSDファイルを書く。"""
    count = len(games)
    records = np.zeros(count, dtype=RECORD_DTYPE)
    records["board"] = INITIAL_BOARD
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


def reference_hash64(seed: int, game: int) -> int:
    """指示書のu64演算をPython整数で独立に計算する。"""
    mask = (1 << 64) - 1
    value = (seed ^ (game * 0x9E3779B97F4A7C15)) & mask
    value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & mask
    value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & mask
    return (value ^ (value >> 31)) & mask


class DatasetTest(unittest.TestCase):
    """Datasetの来歴検証、分割、世代、収集順を検証する。"""

    def test_rejects_duplicate_path_and_seed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first.bin"
            second = root / "second.bin"
            write_mnsd(first, seed=7, checksum=b"a" * 32, games=[0])
            write_mnsd(second, seed=7, checksum=b"b" * 32, games=[1])

            with self.assertRaises(ValueError):
                Dataset([first, first.resolve()])
            with self.assertRaises(ValueError):
                Dataset([first, second])

    def test_hash_split_is_stable_when_file_order_changes(self) -> None:
        games = list(range(80))
        expected_hashes = np.array(
            [reference_hash64(1, game) for game in games], dtype=np.uint64
        )
        np.testing.assert_array_equal(
            hash64(1, np.array(games, dtype=np.uint32)), expected_hashes
        )
        self.assertEqual(reference_hash64(0, 0), 0)
        self.assertEqual(reference_hash64(1, 0), 0x5692161D100B05E5)

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first.bin"
            second = root / "second.bin"
            write_mnsd(first, seed=11, checksum=b"a" * 32, games=games)
            write_mnsd(second, seed=22, checksum=b"a" * 32, games=games)

            memberships: list[dict[tuple[int, int], bool]] = []
            for paths in ([first, second], [second, first]):
                dataset = Dataset(paths)
                membership: dict[tuple[int, int], bool] = {}
                for header, records, validation in zip(
                    dataset.headers, dataset.records, dataset.validation_masks
                ):
                    for game, selected in zip(records["game"], validation):
                        key = (header.seed, int(game))
                        membership[key] = bool(selected)
                        self.assertEqual(
                            bool(selected), reference_hash64(*key) % 20 == 0
                        )
                memberships.append(membership)
            self.assertEqual(memberships[0], memberships[1])

    def test_gather_preserves_arbitrary_order_and_duplicates(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first.bin"
            second = root / "second.bin"
            write_mnsd(
                first, seed=1, checksum=b"a" * 32, games=[10, 11, 12], scores=[1, 2, 3]
            )
            write_mnsd(
                second, seed=2, checksum=b"a" * 32, games=[20, 21], scores=[4, 5]
            )
            dataset = Dataset([first, second])

            gathered = dataset.gather(np.array([4, 0, 3, 1, 4], dtype=np.int64))
            self.assertEqual(gathered.dtype, RECORD_DTYPE)
            self.assertEqual(gathered["score"].tolist(), [5, 1, 4, 2, 5])
            self.assertEqual(dataset.gather(np.array([], dtype=np.int64)).shape, (0,))


class TeacherScaleTest(unittest.TestCase):
    """世代別教師Kとモデル出力Kの責務分離を検証する。"""

    def test_generations_have_independent_teacher_scales(self) -> None:
        games = list(range(40))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = root / "first.bin"
            second = root / "second.bin"
            write_mnsd(
                first,
                seed=101,
                checksum=b"a" * 32,
                games=games,
                scores=[-1000, 1000] * 20,
                results=[0, 2] * 20,
            )
            write_mnsd(
                second,
                seed=202,
                checksum=b"b" * 32,
                games=games,
                scores=[-1000, 1000] * 20,
                results=[2, 0] * 20,
            )
            dataset = Dataset([first, second])
            self.assertEqual(dataset.file_generations.tolist(), [0, 1])

            generation_ks, generation_counts = estimate_generation_ks(dataset)
            expected_ks = []
            expected_counts = []
            for generation in range(dataset.generation_count):
                records = dataset.gather(
                    dataset.generation_training_indices(generation)
                )
                expected_ks.append(estimate_k(records["score"], records["result"]))
                expected_counts.append(records.size)
            np.testing.assert_allclose(generation_ks, expected_ks)
            self.assertEqual(generation_counts, expected_counts)
            self.assertNotAlmostEqual(generation_ks[0], generation_ks[1])

            records = np.zeros(2, dtype=RECORD_DTYPE)
            records["score"] = 1000
            records["result"] = 1
            targets = build_targets(
                records,
                np.array(generation_ks, dtype=np.float64),
                np.array([0, 1], dtype=np.int64),
                1.0,
            )
            expected = 1.0 / (
                1.0 + np.exp(-1000.0 / np.array(generation_ks))
            )
            np.testing.assert_allclose(targets, expected, rtol=1e-6)

            model = torch.nn.Embedding(2, 1)
            with torch.no_grad():
                model.weight[:, 0] = torch.tensor([30.0, 10.0])
            logits = model_logits(model, torch.tensor([[0, 1]]), 200.0)
            self.assertAlmostEqual(float(logits.item()), 0.2)


if __name__ == "__main__":
    unittest.main()
