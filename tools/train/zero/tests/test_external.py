"""探索なしの検証データの診断を、版3のバイト列と手計算の期待値で検証する。

MNSDはrecords.rsの仕様から組み立てる。全重みを0、価値のバイアスをlog(3)とした
ネットは勝率3/4を返すので、勝ちの損失はlog(4/3)、負けはlog(4)となる。
方策の同率は最小ラベルで解き、対局内、対局間の順に平均した値を期待値とする。
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
from minase_zero.data import read_file, read_replay_buffer
from minase_zero.external import external_eval
from minase_zero.net import ZeroNet

from tests.cli import require_success, run_cli
from tests.test_data import FIXTURE


WIN_LOSS = math.log(4 / 3)
LOSS_LOSS = math.log(4)


def write_validation(
    path: Path, games: list[tuple[int, tuple[int, ...], int, int]],
    *, rule_set: str = "lishogi", origin: int = 2,
) -> None:
    """対局番号、着手番号の列、結果、開始手数から探索なしのMNSD版3を書く。"""
    records = []
    entry_count = 0
    for number, moves, outcome, start_ply in games:
        board = bytearray(144)
        # 王将0、飛車12、後手の王将143、歩兵131。駒の動きは競技規則第9条による。
        board[0], board[12], board[143], board[131] = 12, 8, 76, 65
        reason = 0 if origin == 3 else 7
        header = struct.pack("<IBBBBHI", number, origin, outcome, reason, 0, start_ply, len(moves) + 1)
        entries = b"".join(struct.pack("<HhBH", move, 0, 8 | 16, 0) for move in moves)
        entries += struct.pack("<HhBH", 65535, 0, 8, 0)
        records.append(header + board + bytes([0, 255, 0]) + entries)
        entry_count += len(moves) + 1
    header = (
        b"MNSD" + struct.pack("<I", 3) + rule_set.encode().ljust(32, b"\0")
        + b"a" * 40 + bytes(32) + struct.pack("<B7xQQQQ", 2, 0, 11, len(games), entry_count)
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
        self.data = self.directory / "validation.mnsd"
        # 初手の最小ラベルは飛車12→24の12で、着手番号は13。
        # (0,0,0)は王将0→1、後手歩兵131→119、王将1→0。
        # 各局面の最小ラベルの手は飛車12→24、後手歩兵131→119、王将1→13なので、
        # 一致は(0,1,0)。後手のラベルは盤を180度回転して数える。
        # 対局9は最終項目だけを持ち、診断の分母には入らない。
        write_validation(self.data, [(1, (13,), 0, 0), (4, (0, 0, 0), 1, 0), (9, (), 0, 0)])
        self.checkpoint = self.directory / "checkpoint.pt"
        with torch.random.fork_rng(devices=[]):
            model = ZeroNet(1, 2)
        with torch.no_grad():
            for parameter in model.parameters():
                parameter.zero_()
            model.value_head[-1].bias.fill_(math.log(3))
        torch.save({"config": {"blocks": 1, "channels": 2}, "model": model.state_dict(),
                    "completed_steps": 0, "total_steps": 0}, self.checkpoint)

    def evaluate(self, name: str) -> dict:
        """公開関数を通し、実ファイルに書かれた診断を読み直す。"""
        out = self.directory / name
        external_eval(self.checkpoint, self.data, "cpu", out)
        return json.loads(out.read_text(encoding="utf-8"))

    def arguments(self, out: Path) -> list[str]:
        """サブコマンドの必須引数をすべて明示する。"""
        return ["external-eval", "--checkpoint", str(self.checkpoint), "--data", str(self.data),
                "--device", "cpu", "--out", str(out)]

    def test_diagnostics_average_games_equally_and_empty_bands_are_null(self) -> None:
        summary = self.evaluate("averages.json")
        actual = summary["diagnostics"]
        self.assertEqual(set(actual), {"game_count", "position_count", "value_result", "policy_agreement"})
        self.assertEqual((actual["game_count"], actual["position_count"]), (2, 4))
        # 1局目は勝ち1局面で一致1、2局目は負け・勝ち・負けで一致は1/3。
        self.assertAlmostEqual(actual["value_result"], (2 * WIN_LOSS + LOSS_LOSS) / 3, places=5)
        self.assertAlmostEqual(actual["policy_agreement"], 2 / 3, places=5)
        self.assertNotAlmostEqual(actual["value_result"], (WIN_LOSS + LOSS_LOSS) / 2, places=5)
        self.assertNotAlmostEqual(actual["policy_agreement"], 1 / 2, places=5)
        self.assertEqual(summary["bands"][0], {"first_move": 1, "last_move": 40, **actual})
        self.assertEqual(summary["bands"][1:], [
            {"first_move": first, "last_move": last, "game_count": 0, "position_count": 0,
             "value_result": None, "policy_agreement": None}
            for first, last in ((41, 100), (101, 200), (201, None))
        ])
        self.assertEqual(summary["data"]["game_count"], 3)

    def test_move_bands_include_both_sides_of_every_boundary(self) -> None:
        write_validation(self.data, [
            (1, (13, 0), 0, 39), (4, (0, 0, 0), 1, 39),
            (7, (13, 0), 1, 99), (9, (0, 0), 0, 199),
        ])
        summary = self.evaluate("bands.json")
        # 41〜100手目は3局がそれぞれ負け、勝ちと負け、負けを持つ。
        # その一致率は1、1/2、1なので、局面平均3/4ではなく5/6になる。
        expected = [
            (1, 40, 2, 2, (WIN_LOSS + LOSS_LOSS) / 2, 1 / 2),
            (41, 100, 3, 4, (WIN_LOSS + 5 * LOSS_LOSS) / 6, 5 / 6),
            (101, 200, 2, 2, WIN_LOSS, 1 / 2),
            (201, None, 1, 1, LOSS_LOSS, 1),
        ]
        for actual, (first, last, games, positions, value, agreement) in zip(summary["bands"], expected, strict=True):
            with self.subTest(first_move=first):
                self.assertEqual(set(actual), {"first_move", "last_move", "game_count", "position_count",
                                               "value_result", "policy_agreement"})
                self.assertEqual((actual["first_move"], actual["last_move"]), (first, last))
                self.assertEqual((actual["game_count"], actual["position_count"]), (games, positions))
                self.assertAlmostEqual(actual["value_result"], value, places=5)
                self.assertAlmostEqual(actual["policy_agreement"], agreement, places=5)

    def test_batch_boundary_preserves_all_positions_and_game_weights(self) -> None:
        games = [(number, (13,), 0, 0) for number in range(1, 129)]
        write_validation(self.data, [*games, (129, (0, 0, 0), 1, 0)])
        summary = self.evaluate("batches.json")
        actual = summary["diagnostics"]
        self.assertEqual((actual["game_count"], actual["position_count"]), (129, 131))
        self.assertAlmostEqual(actual["value_result"], (385 * WIN_LOSS + 2 * LOSS_LOSS) / 387, places=5)
        self.assertAlmostEqual(actual["policy_agreement"], 385 / 387, places=5)
        self.assertEqual(summary["batch_size"], 128)

    def test_extension_keeps_every_played_move_without_search_values(self) -> None:
        raw = minase_mnsd_reader.read_validation(str(self.data))
        self.assertEqual(raw["teacher_kind"], 2)
        self.assertEqual(raw["origin"], 2)
        self.assertIs(raw["lishogi"], True)
        self.assertIs(raw["jcsa"], False)
        self.assertEqual(raw["network_checksum"], bytes(32))
        self.assertEqual(raw["teacher_search_budget"], 0)
        self.assertEqual((raw["training_count"], raw["validation_count"]), (1, 3))
        self.assertEqual(raw["game_numbers"], struct.pack("<III", 1, 4, 9))
        self.assertEqual(raw["games"], struct.pack("<IIII", 1, 4, 4, 4))
        self.assertEqual(raw["plies"], struct.pack("<HHHH", 0, 0, 1, 2))
        self.assertEqual(raw["sides"], bytes([0, 0, 1, 0]))
        self.assertEqual(raw["outcomes"], bytes([0, 1, 1, 1]))
        self.assertEqual(raw["values"], bytes(4 * 4))
        self.assertEqual(raw["visits"], bytes(4 * 4))
        self.assertEqual(raw["policy_offsets"], struct.pack("<QQQQQ", 0, 1, 2, 3, 4))
        self.assertEqual(raw["policy_probabilities"], struct.pack("<HHHH", 65535, 65535, 65535, 65535))
        self.assertEqual(raw["policy_from"], bytes([12, 0, 131, 1]))
        self.assertEqual(raw["policy_to"], bytes([24, 1, 119, 0]))
        self.assertEqual(minase_mnsd_reader.read(str(self.data))["games"], b"")

    def test_missing_played_move_policy_is_rejected_without_output(self) -> None:
        contents = bytearray(self.data.read_bytes())
        contents[152 + 161 + 4] = 8
        self.data.write_bytes(contents)
        with self.assertRaises(ValueError):
            self.evaluate("missing-policy.json")
        self.assertFalse((self.directory / "missing-policy.json").exists())

    def test_search_teachers_are_rejected(self) -> None:
        alpha = self.directory / "alpha.mnsd"
        contents = bytearray(self.data.read_bytes())
        contents[112] = 0
        alpha.write_bytes(contents)
        for data in (alpha, FIXTURE):
            with self.subTest(data=data), self.assertRaisesRegex(ValueError, "NoSearch"):
                external_eval(self.checkpoint, data, "cpu", self.directory / "search.json")
        self.assertFalse((self.directory / "search.json").exists())

    def test_nonhuman_origins_and_jcsa_rules_are_rejected(self) -> None:
        for origin in (0, 1, 3):
            outcome = 2 if origin == 3 else 0
            write_validation(self.data, [(1, (13,), outcome, 0)], origin=origin)
            with self.subTest(origin=origin), self.assertRaisesRegex(ValueError, "origin"):
                self.evaluate("origin.json")
            self.assertFalse((self.directory / "origin.json").exists())
        write_validation(self.data, [(1, (13,), 0, 0)], rule_set="JCSA")
        with self.assertRaisesRegex(ValueError, "rules must equal lishogi"):
            self.evaluate("rules.json")
        self.assertFalse((self.directory / "rules.json").exists())

    def test_equivalent_lishogi_rule_codes_are_accepted(self) -> None:
        rules = "L1,L2,P0,P3,R1,E1,E3"
        write_validation(self.data, [(1, (13,), 0, 0)], rule_set=rules)
        summary = self.evaluate("rule-codes.json")
        self.assertEqual(summary["data"]["rule_set"], rules)
        self.assertAlmostEqual(summary["diagnostics"]["value_result"], WIN_LOSS, places=5)
        self.assertEqual(summary["diagnostics"]["policy_agreement"], 1)

    def test_zero_games_and_zero_positions_are_rejected(self) -> None:
        for games, message in (([], "contain games"), ([(1, (), 0, 0)], "contain positions")):
            write_validation(self.data, games)
            with self.subTest(games=games), self.assertRaisesRegex(ValueError, message):
                self.evaluate("empty.json")
            self.assertFalse((self.directory / "empty.json").exists())

    def test_training_readers_reject_validation_data_as_non_mcts(self) -> None:
        with self.assertRaisesRegex(ValueError, "teacher must be MCTS"):
            read_file(self.data)
        with self.assertRaisesRegex(ValueError, "teacher must be MCTS"):
            read_replay_buffer([self.data], [self.data], 42,
                rank=0, world_size=1, batch_size=1, micro_batch_size=1, replay_threads=1,
            )

    def test_nonfinite_diagnostics_are_rejected_without_output(self) -> None:
        state = torch.load(self.checkpoint, weights_only=True)
        for value in (float("nan"), float("inf"), -float("inf")):
            state["model"]["value_head.6.bias"][0] = value
            torch.save(state, self.checkpoint)
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "non-finite diagnostic"):
                self.evaluate("nonfinite.json")
            self.assertFalse((self.directory / "nonfinite.json").exists())
        state["model"]["value_head.6.bias"].fill_(math.log(3))
        state["model"]["policy_head.3.bias"].fill_(float("nan"))
        torch.save(state, self.checkpoint)
        with self.assertRaisesRegex(ValueError, "finite maximum legal logit"):
            self.evaluate("nonfinite-policy.json")
        self.assertFalse((self.directory / "nonfinite-policy.json").exists())

    def test_cli_records_provenance_and_does_not_modify_checkpoint(self) -> None:
        before = self.checkpoint.read_bytes()
        data_bytes = self.data.read_bytes()
        alias = self.directory / "alias.mnsd"
        alias.symlink_to(self.data)
        out = self.directory / "cli.json"
        args = self.arguments(out)
        args[args.index("--data") + 1] = str(alias)
        require_success(run_cli(args))
        text = out.read_text(encoding="utf-8")
        summary = json.loads(text)
        self.assertEqual(self.checkpoint.read_bytes(), before)
        self.assertEqual(self.data.read_bytes(), data_bytes)
        self.assertEqual(set(summary), {"checkpoint_sha256", "blocks", "channels", "data", "device",
                                        "precision", "batch_size", "diagnostics", "bands", "elapsed_seconds"})
        self.assertEqual(summary["checkpoint_sha256"], hashlib.sha256(before).hexdigest())
        self.assertEqual((summary["blocks"], summary["channels"]), (1, 2))
        self.assertEqual((summary["device"], summary["precision"], summary["batch_size"]), ("cpu", "fp32", 128))
        self.assertEqual(summary["data"], {
            "path": str(self.data.resolve(strict=True)), "size": len(data_bytes),
            "sha256": hashlib.sha256(data_bytes).hexdigest(), "rule_set": "lishogi",
            "generation_commit": "a" * 40, "seed": 11, "game_count": 3,
        })
        self.assertGreater(summary["elapsed_seconds"], 0)
        self.assertTrue(math.isfinite(summary["elapsed_seconds"]))
        self.assertEqual(text, json.dumps(summary, ensure_ascii=False, allow_nan=False, indent=2) + "\n")

    def test_every_cli_argument_is_required_and_device_is_restricted(self) -> None:
        out = self.directory / "missing.json"
        groups = [["--checkpoint", str(self.checkpoint)], ["--data", str(self.data)],
                  ["--device", "cpu"], ["--out", str(out)]]
        invalid = [(groups[missing][0],
                    ["external-eval", *(arg for index, group in enumerate(groups) if index != missing for arg in group)])
                   for missing in range(len(groups))]
        for device in ("auto", "cuda:0", "mps"):
            args = self.arguments(out)
            args[args.index("--device") + 1] = device
            invalid.append(("--device", args))
        invalid.append(("unrecognized", [*self.arguments(out), str(self.data)]))
        for option, arguments in invalid:
            with self.subTest(arguments=arguments), patch("sys.argv", ["minase_zero", *arguments]):
                error = io.StringIO()
                with redirect_stderr(error):
                    self.assertEqual(main(), 1)
                self.assertEqual(len(error.getvalue().splitlines()), 1)
                self.assertIn(option, error.getvalue())
        # CUDAの利用可否に達すれば、選択肢としては受理されている。
        cuda_args = self.arguments(out)
        cuda_args[cuda_args.index("--device") + 1] = "cuda"
        with patch("sys.argv", ["minase_zero", *cuda_args]), patch("torch.cuda.is_available", return_value=False):
            error = io.StringIO()
            with redirect_stderr(error):
                self.assertEqual(main(), 1)
            self.assertIn("CUDA is unavailable", error.getvalue())
        self.assertFalse(out.exists())

    def test_existing_output_and_dangling_symlink_are_rejected_immediately(self) -> None:
        existing = self.directory / "existing.json"
        original = b"keep this output\n"
        existing.write_bytes(original)
        dangling = self.directory / "dangling.json"
        missing = self.directory / "absent.json"
        dangling.symlink_to(missing)
        for out in (existing, dangling):
            with self.subTest(out=out), self.assertRaises(FileExistsError):
                external_eval(missing, missing, "cpu", out)
        self.assertEqual(existing.read_bytes(), original)
        self.assertTrue(dangling.is_symlink())
        self.assertEqual(dangling.readlink(), missing)
        self.assertFalse(missing.exists())


if __name__ == "__main__":
    unittest.main()
