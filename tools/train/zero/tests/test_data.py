"""生成器の固定データとMNSDの形式仕様を使い、読み込み境界と学習窓を検証する。"""

from pathlib import Path
import shutil
import struct
import tempfile
import unittest

import minase_mnsd_reader
import numpy as np

from minase_zero.data import read_file, read_window


FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "zero-selfplay-small.mnsd"


def single_game(number: int, side: int, outcome: int, value: int | None, probabilities: tuple[int, ...]) -> bytes:
    """records.rsの版2の仕様から、最終項目だけを持つ小さな対局を構成する。"""
    board = bytearray(144)
    # 先手の王将と飛車、後手の王将と歩兵を置き、合法手と両陣営の王駒を確保する。
    board[0], board[12], board[143], board[131] = 12, 8, 76, 65
    reason = 11 if outcome == 2 else 7
    header = struct.pack("<IBBBBHI", number, 0, outcome, reason, 0, 23, 1)
    flags = 4 if value is None else 0
    entry = struct.pack("<HHBHI", 65535, 0 if value is None else value, flags, len(probabilities), 128)
    candidates = b"".join(struct.pack("<HH", index, probability) for index, probability in enumerate(probabilities))
    return header + board + bytes([side, 255, 0]) + entry + candidates


def write_small(path: Path, games: list[bytes], rule_set: str) -> None:
    """形式仕様どおりにヘッダを作り、1項目ずつの対局を実ファイルへ書く。"""
    header = (
        b"MNSD" + struct.pack("<I", 2) + rule_set.encode("utf-8").ljust(32, b"\0")
        + b"a" * 40 + bytes(range(32))
        + struct.pack("<B7xQQQQ", 1, 128, 20261038, len(games), len(games))
    )
    path.write_bytes(header + b"".join(games))


class DataTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.raw = minase_mnsd_reader.read(str(FIXTURE))
        cls.dataset, cls.info = read_file(FIXTURE)

    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-data-")
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)

    def test_generator_fixture_metadata_and_selected_positions(self) -> None:
        # 生成器の集計は8局・探索1009局面・先手3勝・後手5勝、基本シード20261038である。
        data, info = self.dataset, self.info
        self.assertEqual(info.header.teacher_kind, 1)
        self.assertEqual(info.header.origin, 0)
        self.assertEqual(info.header.rule_set, "L0,P0,R1,E0")
        self.assertTrue(info.header.engine_default)
        self.assertEqual(info.header.seed, 20261038)
        self.assertEqual(info.header.game_count, 8)
        self.assertEqual(info.header.entry_count, 1099)
        self.assertEqual(len(info.game_numbers), 8)
        self.assertEqual(info.position_count, 1009)
        self.assertEqual(len(data), 1009)
        self.assertEqual(data.boards.shape, (1009, 144))
        self.assertEqual(info.path, FIXTURE.resolve())
        self.assertEqual(info.size, FIXTURE.stat().st_size)
        self.assertEqual(info.training_count + info.validation_count, 1009)
        self.assertEqual(info.validation_count, np.count_nonzero(data.validation))
        self.assertEqual(len(np.unique(data.games[data.validation])), 2)
        outcomes = np.frombuffer(self.raw["outcomes"], dtype=np.uint8)
        game_outcomes = []
        for game in info.game_numbers:
            mask = data.games == game
            self.assertEqual(len(np.unique(data.validation[mask])), 1)
            result = np.unique(outcomes[mask])
            self.assertEqual(len(result), 1)
            game_outcomes.append(int(result[0]))
        self.assertEqual(game_outcomes.count(0), 3)
        self.assertEqual(game_outcomes.count(1), 5)
        expected = np.where(outcomes == data.sides, 1.0, 0.0)
        np.testing.assert_array_equal(data.results, expected)
        self.assertEqual(data.results.dtype, np.float32)
        self.assertTrue(np.all((data.values >= 0) & (data.values <= 65535)))

    def test_generator_fixture_sparse_policy(self) -> None:
        data = self.dataset
        self.assertEqual(data.policy_offsets.shape, (1010,))
        self.assertEqual(data.policy_offsets[0], 0)
        self.assertEqual(data.policy_offsets[-1], len(data.policy_labels))
        self.assertEqual(len(data.policy_labels), len(data.policy_probabilities))
        self.assertEqual(data.policy_labels.dtype, np.uint16)
        self.assertEqual(data.policy_probabilities.dtype, np.uint16)
        self.assertTrue(np.all(data.policy_labels < 35856))
        for start, end in zip(data.policy_offsets[:-1], data.policy_offsets[1:]):
            self.assertGreater(end, start)
            self.assertEqual(int(data.policy_probabilities[start:end].sum(dtype=np.uint64)), 65535)
            self.assertEqual(len(np.unique(data.policy_labels[start:end])), end - start)

    def test_extension_returns_python_scalars_and_little_endian_bytes(self) -> None:
        path = self.directory / "scalars.mnsd"
        write_small(path, [single_game(0x10203040, 0, 1, 0x1234, (65535,))], "engine-default")
        raw = minase_mnsd_reader.read(str(path))
        self.assertEqual(minase_mnsd_reader.FORMAT_VERSION, 2)
        for key in ("origin", "teacher_kind", "teacher_search_budget", "seed", "game_count", "entry_count"):
            self.assertIs(type(raw[key]), int)
        self.assertIs(type(raw["engine_default"]), bool)
        self.assertIs(type(raw["rule_set"]), str)
        self.assertEqual(raw["generation_commit"], "a" * 40)
        self.assertEqual(raw["network_checksum"], bytes(range(32)))
        self.assertEqual(raw["games"], b"\x40\x30\x20\x10")
        self.assertEqual(raw["game_numbers"], raw["games"])
        self.assertEqual(raw["plies"], b"\x17\0")
        self.assertEqual(raw["values"], b"\x34\x12\0\0")
        self.assertEqual(raw["visits"], b"\x80\0\0\0")
        self.assertEqual(raw["policy_offsets"], struct.pack("<QQ", 0, 1))
        self.assertEqual(raw["policy_probabilities"], b"\xff\xff")

    def test_same_normalized_path_is_rejected(self) -> None:
        alias = self.directory / "alias.mnsd"
        alias.symlink_to(FIXTURE)
        for paths in ([FIXTURE, FIXTURE], [FIXTURE, alias]):
            with self.subTest(paths=paths):
                with self.assertRaises(ValueError):
                    read_window(paths)

    def test_copied_games_are_rejected(self) -> None:
        copy = self.directory / "copy.mnsd"
        shutil.copyfile(FIXTURE, copy)
        with self.assertRaises(ValueError):
            read_window([FIXTURE, copy])

    def test_same_seed_disjoint_games_preserve_input_and_candidate_order(self) -> None:
        first, second = self.directory / "first.mnsd", self.directory / "second.mnsd"
        write_small(first, [single_game(9, 0, 0, 123, (12345, 53190))], "engine-default")
        write_small(second, [single_game(3, 1, 0, 456, (65535,))], "engine-default")
        data, infos = read_window([first, second])
        self.assertEqual([info.path for info in infos], [first, second])
        np.testing.assert_array_equal(data.games, [9, 3])
        np.testing.assert_array_equal(data.plies, [23, 23])
        np.testing.assert_array_equal(data.values, [123, 456])
        np.testing.assert_array_equal(data.file_indices, [0, 1])
        np.testing.assert_array_equal(data.results, [1, 0])
        np.testing.assert_array_equal(data.policy_offsets, [0, 2, 3])
        np.testing.assert_array_equal(data.policy_probabilities, [12345, 53190, 65535])
        # 方策の仕様表: 先手の王将の右・右前、後手の歩兵の前。移動元は0、0、131。
        np.testing.assert_array_equal(data.policy_labels, [3168, 1584, 12])
        reversed_data, _ = read_window([second, first])
        np.testing.assert_array_equal(reversed_data.games, [3, 9])
        np.testing.assert_array_equal(reversed_data.policy_offsets, [0, 1, 3])
        np.testing.assert_array_equal(reversed_data.policy_labels, [12, 3168, 1584])

    def test_same_game_number_with_different_seed_is_accepted(self) -> None:
        first, second = self.directory / "first.mnsd", self.directory / "second.mnsd"
        write_small(first, [single_game(1, 0, 2, 32768, (65535,))], "engine-default")
        changed = bytearray(first.read_bytes())
        struct.pack_into("<Q", changed, 128, 42)
        second.write_bytes(changed)
        data, infos = read_window([first, second])
        self.assertEqual([info.header.seed for info in infos], [20261038, 42])
        np.testing.assert_array_equal(data.games, [1, 1])
        np.testing.assert_array_equal(data.results, [0.5, 0.5])

    def test_unselected_games_still_participate_in_duplicate_detection(self) -> None:
        first, second = self.directory / "first.mnsd", self.directory / "second.mnsd"
        write_small(first, [single_game(1, 0, 2, None, ())], "engine-default")
        shutil.copyfile(first, second)
        data, info = read_file(first)
        self.assertEqual(len(data), 0)
        self.assertEqual(info.game_numbers, (1,))
        with self.assertRaises(ValueError):
            read_window([first, second])

    def test_empty_files_and_empty_window(self) -> None:
        path = self.directory / "empty.mnsd"
        write_small(path, [], "engine-default")
        data, infos = read_window([path])
        self.assertEqual(len(data), 0)
        self.assertEqual(data.boards.shape, (0, 144))
        np.testing.assert_array_equal(data.policy_offsets, [0])
        self.assertEqual(infos[0].header.origin, None)
        self.assertEqual(infos[0].training_count, 0)
        self.assertEqual(infos[0].validation_count, 0)
        with self.assertRaises(ValueError):
            read_window([])

    def test_equivalent_rule_name_is_accepted_and_other_rules_rejected(self) -> None:
        path = self.directory / "rules.mnsd"
        game = single_game(1, 0, 2, 32768, (65535,))
        write_small(path, [game], "L0,P0,R1,E0")
        data, info = read_file(path)
        self.assertEqual(len(data), 1)
        self.assertTrue(info.header.engine_default)
        write_small(path, [game], "L0,P1,R1,E0")
        with self.assertRaises(ValueError):
            read_file(path)

    def test_non_mcts_non_selfplay_and_deferred_positions_are_rejected(self) -> None:
        path = self.directory / "rejected.mnsd"
        write_small(path, [single_game(1, 0, 2, 123, (65535,))], "engine-default")
        original = path.read_bytes()
        alpha = bytearray(original)
        alpha[112] = 0
        del alpha[152 + 161 + 7:152 + 161 + 11]
        path.write_bytes(alpha)
        self.assertEqual(minase_mnsd_reader.read(str(path))["teacher_kind"], 0)
        with self.assertRaises(ValueError):
            read_file(path)
        for origin in (1, 2):
            changed = bytearray(original)
            changed[152 + 4] = origin
            path.write_bytes(changed)
            self.assertEqual(minase_mnsd_reader.read(str(path))["origin"], origin)
            with self.assertRaises(ValueError):
                read_file(path)
        # recordsの駒コードのビット128は、盤の100番升の歩兵に付く保留印である。
        write_small(path, [single_game(1, 0, 2, 123, (65535,))], "engine-default")
        changed = bytearray(path.read_bytes())
        changed[152 + 14 + 100] = 129
        path.write_bytes(changed)
        self.assertEqual(minase_mnsd_reader.read(str(path))["boards"][100], 129)
        with self.assertRaises(ValueError):
            read_file(path)

    def test_missing_candidates_and_wrong_probability_sum_are_rejected(self) -> None:
        path = self.directory / "policy.mnsd"
        for probabilities in ((), (65534,)):
            with self.subTest(probabilities=probabilities):
                write_small(path, [single_game(1, 0, 2, 123, probabilities)], "engine-default")
                with self.assertRaises(ValueError):
                    read_file(path)

    def test_extension_distinguishes_io_and_format_errors(self) -> None:
        path = self.directory / "missing.mnsd"
        with self.assertRaises(OSError):
            minase_mnsd_reader.read(str(path))
        path.write_bytes(b"MNSD")
        with self.assertRaises(ValueError):
            minase_mnsd_reader.read(str(path))


if __name__ == "__main__":
    unittest.main()
