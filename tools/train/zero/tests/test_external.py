"""指示書の固定抽出と診断専用の契約を、実際のαβ棋譜で検証する。

抽出は再現性・重複なし・全対局を選ぶ境界、診断は選択対局ごとの既知の局面数と
交差エントロピーの定義式を正とする。入力と出力の拒否は実ファイルで確かめる。
MNSDの組み立てはrecords.rsの版2の仕様による。学習器の実装から期待値を写さない。
"""

from contextlib import redirect_stderr
import hashlib
import io
import json
import math
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

import minase_mnsd_reader
import torch

from minase_zero.__main__ import main
from minase_zero.data import read_file, read_window
from minase_zero.external import external_eval
from minase_zero.net import ZeroNet

from .cli import require_success, run_cli
from .test_data import FIXTURE


def write_alpha(path: Path, seed: int, games: list[tuple[int, tuple[int, ...], int]]) -> None:
    """対局番号、各着手の除外フラグ、結果を指定して、αβの実ファイルを作る。"""
    records = []
    entry_count = 0
    for number, flags, outcome in games:
        board = bytearray(144)
        # 王将0、飛車12、後手の王将143、歩兵131で、最初の合法手は27手ある。
        board[0], board[12], board[143], board[131] = 12, 8, 76, 65
        reason = 11 if outcome == 2 else 7
        header = struct.pack("<IBBBBHI", number, 0, outcome, reason, 0, 0, len(flags) + 1)
        # 指した手の番号0を確率1の教師とし、最後は価値のない終局項目にする。
        entries = b"".join(
            struct.pack("<HhBH", 0, 0 if flag & 4 else -32768 + index, flag | 8, 0)
            for index, flag in enumerate(flags)
        ) + struct.pack("<HhBH", 65535, 0, 4, 0)
        records.append(header + board + bytes([0, 255, 0]) + entries)
        entry_count += len(flags) + 1
    header = (
        b"MNSD" + struct.pack("<I", 2) + b"engine-default".ljust(32, b"\0")
        + b"a" * 40 + bytes(32) + struct.pack("<B7xQQQQ", 0, 128, seed, len(games), entry_count)
    )
    path.write_bytes(header + b"".join(records))


class ExternalEvaluationTests(unittest.TestCase):
    def setUp(self) -> None:
        directory = tempfile.TemporaryDirectory(prefix="minase-zero-external-")
        self.addCleanup(directory.cleanup)
        self.directory = Path(directory.name)
        previous = torch.get_num_threads()
        torch.set_num_threads(1)
        self.addCleanup(torch.set_num_threads, previous)
        self.first, self.second = self.directory / "first.mnsd", self.directory / "second.mnsd"
        # seed=11のgame=4は既存の分割では検証対局である。game=9は全局面が選別で落ちる。
        write_alpha(self.first, 11, [(1, (0,), 0), (4, (0, 0), 1), (9, (1, 4), 2)])
        write_alpha(self.second, 29, [(2, (0, 0), 2), (7, (0,), 0)])
        self.paths = [self.first, self.second]
        self.checkpoint = self.directory / "checkpoint.pt"
        # 方策は定数マスク内で一様、価値は3/4とし、期待値を手計算できるようにする。
        with torch.random.fork_rng(devices=[]):
            model = ZeroNet(1, 2)
        with torch.no_grad():
            for parameter in model.parameters():
                parameter.zero_()
            model.value_head[-1].bias.fill_(math.log(3))
        torch.save({"config": {"blocks": 1, "channels": 2}, "model": model.state_dict(),
                    "completed_steps": 0, "total_steps": 0}, self.checkpoint)

    def evaluate(self, name: str, games: int, seed: int) -> dict:
        """公開関数を通し、実ファイルに書かれた診断を読み直す。"""
        out = self.directory / name
        external_eval(self.checkpoint, self.paths, games, seed, "cpu", out)
        return json.loads(out.read_text())

    def arguments(self, out: Path) -> list[str]:
        """サブコマンドの必須引数をすべて明示する。"""
        return ["external-eval", "--checkpoint", str(self.checkpoint),
                "--data", *map(str, self.paths), "--games", "5", "--seed", "42",
                "--device", "cpu", "--out", str(out)]

    def test_sampling_is_reproducible_distinct_and_keeps_only_selected_games(self) -> None:
        first = self.evaluate("first.json", 3, 42)
        again = self.evaluate("again.json", 3, 42)
        changed = self.evaluate("changed.json", 3, 99)
        self.assertEqual(first["selected_games"], again["selected_games"])
        self.assertEqual(first["sample_sha256"], again["sample_sha256"])
        self.assertEqual(first["diagnostics"], again["diagnostics"])
        self.assertNotEqual(first["selected_games"], changed["selected_games"])
        self.assertNotEqual(first["sample_sha256"], changed["sample_sha256"])
        # NumPyのPCG64から独立に求めたchoice(5, 3, replace=False)の集合は、
        # seed=42で{0,3,4}、seed=99で{2,3,4}となる。
        self.assertEqual(first["selected_games"], [{"seed": 11, "game": 1},
                                                 {"seed": 29, "game": 2}, {"seed": 29, "game": 7}])
        self.assertEqual(changed["selected_games"], [{"seed": 11, "game": 9},
                                                   {"seed": 29, "game": 2}, {"seed": 29, "game": 7}])
        counts = {(11, 1): 1, (11, 4): 2, (11, 9): 0, (29, 2): 2, (29, 7): 1}
        for summary in (first, changed):
            selected = [(game["seed"], game["game"]) for game in summary["selected_games"]]
            self.assertEqual(len(selected), 3)
            self.assertEqual(len(set(selected)), 3)
            self.assertTrue(set(selected) <= counts.keys())
            self.assertEqual(summary["diagnostics"]["position_count"], sum(counts[key] for key in selected))
            self.assertEqual(summary["diagnostics"]["game_count"], sum(counts[key] > 0 for key in selected))

    def test_all_games_include_validation_and_empty_games_in_population_order(self) -> None:
        summary = self.evaluate("all.json", 5, 42)
        expected = [(11, 1), (11, 4), (11, 9), (29, 2), (29, 7)]
        self.assertEqual(summary["selected_games"], [{"seed": seed, "game": game} for seed, game in expected])
        self.assertEqual(summary["games"], 5)
        self.assertEqual(summary["diagnostics"]["game_count"], 4)
        self.assertEqual(summary["diagnostics"]["position_count"], 6)
        encoded = b"".join(seed.to_bytes(8, "little") + game.to_bytes(4, "little") for seed, game in expected)
        self.assertEqual(summary["sample_sha256"], hashlib.sha256(encoded).hexdigest())
        self.paths.reverse()
        reversed_summary = self.evaluate("reversed.json", 5, 42)
        self.assertEqual(reversed_summary["selected_games"], summary["selected_games"][3:] + summary["selected_games"][:3])
        self.assertNotEqual(reversed_summary["sample_sha256"], summary["sample_sha256"])

    def test_losses_and_policy_metrics_follow_independent_position_means(self) -> None:
        summary = self.evaluate("metrics.json", 5, 42)
        # 先手の王将は2手、飛車は21手と成り4手。後手番では王将2手と歩兵1手になる。
        # 同率時の最小ラベルは先手では飛車、後手では歩兵なので、一致するのは後手の2局面。
        # 結果は手番順に1,0,1,1/2,1/2,1であり、平均は2/3となる。
        expected = {
            "policy": math.log(16_108),
            "value_result": math.log(4) - 2 * math.log(3) / 3,
            "legal_probability_mass": 19 / 16_108,
            "log_legal_probability_mass": (4 * math.log(27 / 16_108) + 2 * math.log(3 / 16_108)) / 6,
            "policy_agreement": 1 / 3,
        }
        self.assertEqual(summary["diagnostics"].keys(), expected.keys() | {"game_count", "position_count"})
        for name, value in expected.items():
            self.assertAlmostEqual(summary["diagnostics"][name], value, places=5, msg=name)
        self.assertNotIn("value", summary["diagnostics"])

    def test_cli_records_provenance_and_does_not_modify_checkpoint(self) -> None:
        before = self.checkpoint.read_bytes()
        out = self.directory / "cli.json"
        require_success(run_cli(self.arguments(out)))
        summary = json.loads(out.read_text())
        self.assertEqual(self.checkpoint.read_bytes(), before)
        self.assertEqual(summary["checkpoint_sha256"], hashlib.sha256(before).hexdigest())
        self.assertEqual((summary["blocks"], summary["channels"]), (1, 2))
        self.assertEqual((summary["games"], summary["seed"], summary["device"]), (5, 42, "cpu"))
        self.assertEqual(summary["files"], [
            {"path": str(path), "size": len(path.read_bytes()), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            for path in self.paths
        ])
        self.assertGreater(summary["elapsed_seconds"], 0)
        self.assertTrue(math.isfinite(summary["elapsed_seconds"]))

    def test_every_cli_argument_is_required_and_device_is_restricted(self) -> None:
        groups = [["--checkpoint", str(self.checkpoint)], ["--data", *map(str, self.paths)],
                  ["--games", "5"], ["--seed", "42"], ["--device", "cpu"],
                  ["--out", str(self.directory / "missing.json")]]
        invalid = [["external-eval", *(arg for index, group in enumerate(groups) if index != missing for arg in group)]
                   for missing in range(len(groups))]
        invalid.append(["external-eval", *(arg if arg != "cpu" else "auto" for group in groups for arg in group)])
        for arguments in invalid:
            with self.subTest(arguments=arguments), patch("sys.argv", ["minase_zero", *arguments]):
                error = io.StringIO()
                with redirect_stderr(error):
                    self.assertEqual(main(), 1)
                self.assertEqual(len(error.getvalue().splitlines()), 1)
        self.assertFalse((self.directory / "missing.json").exists())

    def test_mcts_is_rejected_and_alpha_beta_remains_unavailable_for_training(self) -> None:
        with self.assertRaisesRegex(ValueError, "alpha-beta"):
            external_eval(self.checkpoint, [FIXTURE], 1, 42, "cpu", self.directory / "mcts.json")
        with self.assertRaisesRegex(ValueError, "MCTS"):
            read_file(self.first)
        with self.assertRaisesRegex(ValueError, "MCTS"):
            read_window([self.first], [self.first], 42)
        self.assertFalse((self.directory / "mcts.json").exists())

    def test_out_of_range_sample_and_seed_are_rejected(self) -> None:
        for games, seed in ((6, 42), (0, 42), (-1, 42), (1, -1), (1, 1 << 64)):
            with self.subTest(games=games, seed=seed), self.assertRaises(ValueError):
                self.evaluate("invalid.json", games, seed)
        self.assertFalse((self.directory / "invalid.json").exists())
        self.assertEqual(self.evaluate("largest-seed.json", 5, (1 << 64) - 1)["games"], 5)

    def test_existing_output_is_rejected_without_changing_its_contents(self) -> None:
        out = self.directory / "existing.json"
        out.write_text("keep this file\n")
        result = run_cli(self.arguments(out))
        self.assertEqual(result.returncode, 1)
        self.assertEqual(len(result.stderr.splitlines()), 1)
        self.assertEqual(out.read_text(), "keep this file\n")
        dangling = self.directory / "dangling.json"
        dangling.symlink_to(self.directory / "absent.json")
        with self.assertRaises(FileExistsError):
            external_eval(self.checkpoint, self.paths, 5, 42, "cpu", dangling)
        self.assertTrue(dangling.is_symlink())
        self.assertFalse((self.directory / "absent.json").exists())

    def test_duplicate_game_keys_are_rejected_but_different_base_seeds_are_distinct(self) -> None:
        copy = self.directory / "copy.mnsd"
        copy.write_bytes(self.first.read_bytes())
        for paths in ([self.first, self.first], [self.first, copy]):
            with self.subTest(paths=paths), self.assertRaisesRegex(ValueError, "duplicate game"):
                external_eval(self.checkpoint, paths, 1, 42, "cpu", self.directory / "duplicate.json")
        write_alpha(self.second, 29, [(1, (0,), 0)])
        summary = self.evaluate("distinct.json", 4, 42)
        self.assertEqual(summary["diagnostics"]["game_count"], 3)
        self.assertIn({"seed": 11, "game": 1}, summary["selected_games"])
        self.assertIn({"seed": 29, "game": 1}, summary["selected_games"])

    def test_empty_inputs_and_selections_without_positions_are_rejected(self) -> None:
        out = self.directory / "empty.json"
        with self.assertRaises(ValueError):
            external_eval(self.checkpoint, [], 1, 42, "cpu", out)
        write_alpha(self.first, 11, [])
        with self.assertRaises(ValueError):
            external_eval(self.checkpoint, [self.first], 1, 42, "cpu", out)
        write_alpha(self.first, 11, [(9, (1, 2, 3, 4), 2)])
        with self.assertRaisesRegex(ValueError, "contain positions"):
            external_eval(self.checkpoint, [self.first], 1, 42, "cpu", out)
        self.assertFalse(out.exists())

    def test_fixture_contains_played_move_targets_and_both_split_memberships(self) -> None:
        raw = minase_mnsd_reader.read(str(self.first))
        self.assertEqual(raw["teacher_kind"], 0)
        self.assertEqual((raw["training_count"], raw["validation_count"]), (1, 2))
        self.assertEqual(raw["games"], struct.pack("<III", 1, 4, 4))
        self.assertEqual(raw["policy_offsets"], struct.pack("<QQQQ", 0, 1, 2, 3))
        self.assertEqual(raw["policy_probabilities"], struct.pack("<HHH", 65535, 65535, 65535))
        self.assertEqual(raw["values"], struct.pack("<iii", -32768, -32768, -32767))

    def test_extension_selects_game_pairs_and_preserves_every_column(self) -> None:
        # 対局1だけの集合と対局4だけの集合は、この棋譜の訓練・検証の分割と一致する。
        training = minase_mnsd_reader.read_games(str(self.first), {(11, 1), (29, 4)})
        validation = minase_mnsd_reader.read_games(str(self.first), {(29, 1), (11, 4)})
        self.assertEqual(training, minase_mnsd_reader.read_selected(str(self.first), [0], False))
        self.assertEqual(validation, minase_mnsd_reader.read_selected(str(self.first), [], True))
        self.assertEqual(training["games"], struct.pack("<I", 1))
        self.assertEqual(validation["games"], struct.pack("<II", 4, 4))
        self.assertEqual(len(training["boards"]), 144)
        self.assertEqual(len(validation["boards"]), 2 * 144)
        self.assertEqual(validation["game_numbers"], struct.pack("<III", 1, 4, 9))
        self.assertEqual((validation["training_count"], validation["validation_count"]), (1, 2))
        all_games = minase_mnsd_reader.read_games(str(self.first), {(11, 4), (11, 9), (11, 1)})
        self.assertEqual(all_games, minase_mnsd_reader.read(str(self.first)))

    def test_extension_empty_or_absent_game_keys_keep_only_metadata(self) -> None:
        for keys in (set(), {(29, 1), (11, 999)}, {(11, 9)}):
            with self.subTest(keys=keys):
                raw = minase_mnsd_reader.read_games(str(self.first), keys)
                self.assertEqual(raw["game_numbers"], struct.pack("<III", 1, 4, 9))
                self.assertEqual((raw["training_count"], raw["validation_count"]), (1, 2))
                for name in ("boards", "games", "policy_from", "policy_probabilities", "legal_from"):
                    self.assertEqual(raw[name], b"")
                for name in ("policy_offsets", "legal_offsets"):
                    self.assertEqual(raw[name], struct.pack("<Q", 0))

    def test_extension_rejects_game_keys_outside_unsigned_integer_ranges(self) -> None:
        for key in ((-1, 1), (1 << 64, 1), (11, -1), (11, 1 << 32)):
            with self.subTest(key=key), self.assertRaises(OverflowError):
                minase_mnsd_reader.read_games(str(self.first), {key})

    def test_external_evaluation_does_not_use_full_file_expansion(self) -> None:
        # 全局面を返すAPIを使うと、抽出後の件数が正しくてもメモリの契約に違反する。
        with patch.object(minase_mnsd_reader, "read", side_effect=AssertionError("full file expansion")):
            summary = self.evaluate("selected-only.json", 3, 42)
        self.assertEqual(summary["diagnostics"]["game_count"], 3)
        self.assertEqual(summary["diagnostics"]["position_count"], 4)


if __name__ == "__main__":
    unittest.main()
