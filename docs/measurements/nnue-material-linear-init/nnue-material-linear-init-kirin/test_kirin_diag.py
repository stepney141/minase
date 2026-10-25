"""指示書と設計書「麒麟の診断」から導いた検査。

段だけの評価は同段差0、3筋への定数加算はその定数をRとする。
局面の重みは等しく、ブートストラップでは同一対局の局面が一緒に動く。
置き直しの範囲・色・占有、麒麟の消失、選別と再生失敗を独立に検査する。
19標本の升は表に含めず、20標本の升は含める。判定の境界0は判定不能。
合成棋譜の捕獲・成りの試験は盤面更新の試験であり、合法手生成を要求しない。
"""

import copy
import json
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import kirin_diag as diag


def game(game_id, clear_pawns=False):
    opening = ["4j4k", "9c9b", "3k4j", "10b9c"]
    if clear_pawns:
        opening += ["3i3h", "10d10e", "7i7h", "6d6e"]
    moves = (opening + ["7j6h", "6c7e", "6h7j", "7e6c"] * 16)[:62]
    return {"id": game_id, "players": {"sente": {"user": {"id": "a"}},
                                       "gote": {"user": {"id": "b"}}},
            "moves": " ".join(moves)}


def games():
    asymmetric = game("c")
    opening = asymmetric["moves"].split()[:4] + ["3i3h", "10d10e"]
    asymmetric["moves"] = " ".join(
        (opening + ["7j6h", "6c7e", "6h7j", "7e6c"] * 16)[:62])
    return [game("a"), game("b", True), asymmetric]


def kirin_coordinates(sfen):
    """評価器自身がSFENを読み、持ち主から見た筋と段を求める。"""
    board, side, _, _ = sfen.split()
    kirin = "O" if side == "b" else "o"
    found = []
    for row_number, row in enumerate(board.split("/"), 1):
        column = 12
        for token in re.findall(r"\d+|\+?[a-zA-Z]", row):
            if token.isdigit():
                column -= int(token)
            else:
                if token == kirin:
                    found.append((column, 13 - row_number) if side == "b"
                                 else (13 - column, row_number))
                column -= 1
    if len(found) != 1:
        raise ValueError("fake evaluator needs one kirin")
    return found[0]


def evaluator(bonus):
    def evaluate(sfens):
        for sfen in sfens:
            file, rank = kirin_coordinates(sfen)
            yield 100 * rank + (bonus if file == 3 else 0), 10 * rank
    return evaluate


def raw_position(game_id, ply, pairs):
    """pairsは (段, 3筋の値, 7筋の値)。期待値は各テストで明記する。"""
    targets = []
    for rank, file3, file7 in pairs:
        targets.extend([{"file": 3, "rank": rank, "evaluation_cp": file3, "nonlinear_cp": 0},
                        {"file": 7, "rank": rank, "evaluation_cp": file7, "nonlinear_cp": 0}])
    return {"game_id": game_id, "ply": ply, "sfen": diag.START + " b - 1", "targets": targets}


class EvaluationTests(unittest.TestCase):
    def test_rank_only_evaluation_has_exactly_zero_R(self):
        result = diag.evaluate_games(games(), evaluator(0))
        self.assertEqual(result["R"], 0.0)
        self.assertEqual(result["counts"]["positions"], 18)
        self.assertEqual(result["counts"]["games"], 3)
        self.assertEqual(result["counts"]["r_positions"], 18)
        self.assertEqual(result["counts"]["replay_failed_games"], 0)
        self.assertEqual({p["ply"] for p in result["positions"]}, {20, 21, 40, 41, 60, 61})

    def test_constant_file_bonus_is_R(self):
        result = diag.evaluate_games(games(), evaluator(50))
        self.assertEqual(result["R"], 50.0)
        self.assertTrue(all(diag.position_d(p) == 50 for p in result["positions"]))

    def test_positions_have_equal_weight_despite_different_matched_rank_counts(self):
        # First position: d=(0+100)/2=50. Second: d=200. R=(50+200)/2=125.
        result = {"positions": [raw_position("a", 20, [(2, 0, 0), (4, 100, 0)]),
                                raw_position("b", 20, [(2, 200, 0)])]}
        self.assertEqual(diag.judge(result, result)["R_cur"]["estimate"], 125)

    def test_positions_without_a_matched_rank_are_excluded_only_from_R(self):
        # Unchanged initial position has occupied files 3 and 7 on rank 4,
        # and an occupied file 3 on rank 2, so it has no comparison pair.
        unchanged = game("blocked")
        unchanged["moves"] = " ".join((["7j6h", "6c7e", "6h7j", "7e6c"] * 16)[:62])
        result = diag.evaluate_games([game("open"), unchanged], evaluator(50))
        self.assertEqual(result["R"], 50)
        self.assertEqual(result["counts"]["positions"], 12)
        self.assertEqual(result["counts"]["r_positions"], 6)
        self.assertEqual(result["counts"]["r_games"], 1)
        self.assertEqual(result["counts"]["excluded_positions_no_pair"], 6)
        self.assertEqual(result["counts"]["excluded_positions"], 6)

    def test_evaluator_must_return_one_pair_per_target(self):
        for delta in (-1, 1):
            with self.subTest(delta=delta):
                def wrong_count(sfens):
                    return [(0, 0)] * (len(sfens) + delta)
                with self.assertRaises(ValueError):
                    diag.evaluate_games([game("a")], wrong_count)

    def test_file_table_centers_before_excluding_cells_with_less_than_20_samples(self):
        positions = []
        for i in range(20):
            p = raw_position(str(i), 20, [(2, 50, 0)])
            p["targets"][0]["nonlinear_cp"] = 10
            if i < 19:
                p["targets"].append({"file": 3, "rank": 4, "evaluation_cp": 150, "nonlinear_cp": 30})
            positions.append(p)
        table = {row["file"]: row for row in diag.file_table(positions)}
        # For the first 19 positions the means are 200/3 and 40/3 cp;
        # the last position has means 25 and 5 cp. The rank-4 cell is excluded.
        self.assertEqual(table[3]["n"], 20)
        self.assertEqual(table[7]["n"], 20)
        self.assertAlmostEqual(table[3]["evaluation_cp"], -175 / 12)
        self.assertAlmostEqual(table[3]["nonlinear_cp"], -35 / 12)
        self.assertAlmostEqual(table[3]["linear_cp"], -35 / 3)
        self.assertAlmostEqual(table[7]["evaluation_cp"], -775 / 12)
        self.assertEqual(table[1]["n"], 0)
        self.assertIsNone(table[1]["evaluation_cp"])
        positions[-1]["targets"].append(
            {"file": 3, "rank": 4, "evaluation_cp": 150, "nonlinear_cp": 30})
        table = {row["file"]: row for row in diag.file_table(positions)}
        self.assertEqual(table[3]["n"], 40)
        self.assertAlmostEqual(table[3]["evaluation_cp"], 100 / 3)


class SelectionTests(unittest.TestCase):
    def test_humans_standard_start_and_at_least_62_plies(self):
        records = [game("valid")]
        for side in ("sente", "gote"):
            bot = game(f"bot-{side}")
            bot["players"][side]["user"]["title"] = "BOT"
            ai = game(f"ai-{side}")
            ai["players"][side]["aiLevel"] = 0
            anonymous = game(f"anonymous-{side}")
            del anonymous["players"][side]["user"]
            records.extend([bot, ai, anonymous])
        custom = game("custom")
        custom["initialSfen"] = None
        short = game("short")
        short["moves"] = " ".join(short["moves"].split()[:61])
        records.extend([custom, short])
        result = diag.evaluate_games(records, evaluator(0))
        self.assertEqual({p["game_id"] for p in result["positions"]}, {"valid"})
        self.assertEqual(result["counts"]["filtered_games"], 8)
        self.assertEqual(result["counts"]["eligible_games"], 1)

    def test_replay_failure_after_last_sample_discards_the_whole_game(self):
        for bad_move in ("1g1h", "1a1b", "6h7k", "nonsense", "13a1b"):
            with self.subTest(move=bad_move):
                broken = game("broken")
                broken["moves"] = " ".join(broken["moves"].split()[:62] + [bad_move])
                result = diag.evaluate_games([game("valid"), broken], evaluator(0))
                self.assertEqual(result["counts"]["replay_failed_games"], 1)
                self.assertEqual(result["counts"]["positions"], 6)
                self.assertEqual({p["game_id"] for p in result["positions"]}, {"valid"})

    def test_kirin_capture_at_destination_and_intermediate_square_stops_tracking(self):
        for capture, following in (("6c7k", "7k6c"), ("6c7k6c", "6c6c")):
            with self.subTest(capture=capture):
                record = game("capture")
                moves = record["moves"].split()
                moves[21], moves[23] = capture, following
                record["moves"] = " ".join(moves)
                result = diag.evaluate_games([record], evaluator(0))
                self.assertEqual(result["counts"]["replay_failed_games"], 0)
                self.assertEqual([p["ply"] for p in result["positions"]], [20, 21, 41, 61])
                self.assertEqual(result["counts"]["excluded_positions_missing_kirin"], 2)

    def test_promoted_kirin_is_never_selected_again_for_either_side(self):
        for first, promote, later, return_move, expected in (
            (20, "7k9k+", 22, "9k7k", [20, 21, 41, 61]),
            (21, "6b4b+", 23, "4b6b", [20, 21, 40, 60]),
        ):
            with self.subTest(promote=promote):
                record = game("promotion")
                moves = record["moves"].split()
                moves[first], moves[later] = promote, return_move
                record["moves"] = " ".join(moves)
                result = diag.evaluate_games([record], evaluator(0))
                self.assertEqual(result["counts"]["replay_failed_games"], 0)
                self.assertEqual([p["ply"] for p in result["positions"]], expected)
                self.assertEqual(result["counts"]["excluded_positions_missing_kirin"], 2)

    def test_moving_kirin_is_tracked_from_its_initial_square(self):
        record = game("moving")
        moves = record["moves"].split()
        moves[18], moves[20] = "7k9k", "9k7k"
        record["moves"] = " ".join(moves)
        result = diag.evaluate_games([record], evaluator(50))
        self.assertEqual(result["counts"]["positions"], 6)
        self.assertEqual(kirin_coordinates(result["positions"][0]["sfen"]), (9, 2))
        self.assertEqual(result["R"], 50)

    def test_duplicate_game_ids_are_rejected(self):
        with self.assertRaises(ValueError):
            diag.evaluate_games([game("same"), game("same")], evaluator(0))


class RelocationTests(unittest.TestCase):
    def test_targets_obey_rank_color_and_occupancy_for_both_sides(self):
        board = {(7, 10): "O", (3, 10): "P", (5, 8): "p", (7, 11): "K", (6, 0): "k"}
        expected = ({(f, r) for r in (1, 3) for f in (2, 4, 6, 8, 10, 12)}
                    | {(f, r) for r in (2, 4) for f in (1, 3, 5, 7, 9, 11)}) - {(3, 2), (5, 4)}
        for side in ("b", "w"):
            with self.subTest(side=side):
                oriented = board if side == "b" else {
                    (13 - f, 11 - r): p.swapcase() for (f, r), p in board.items()}
                source = diag.emit_board(oriented) + f" {side} - 1"
                targets = diag.relocations(source)
                self.assertEqual({(f, r) for f, r, _ in targets}, expected)
                self.assertIn((7, 2), expected)
                for file, rank, sfen in targets:
                    self.assertEqual(kirin_coordinates(sfen), (file, rank))
                    piece = "O" if side == "b" else "o"
                    moved = diag.parse_board(sfen.split()[0])
                    self.assertEqual({sq: p for sq, p in moved.items() if p != piece},
                                     {sq: p for sq, p in oriented.items() if p != piece})
                self.assertEqual(next(s for f, r, s in targets if (f, r) == (7, 2)), source)

    def test_current_square_outside_home_four_ranks_is_not_a_target(self):
        sfen = diag.emit_board({(7, 6): "O", (7, 11): "K", (6, 0): "k"}) + " b - 1"
        targets = diag.relocations(sfen)
        self.assertEqual(len(targets), 24)
        self.assertNotIn((7, 6), {(f, r) for f, r, _ in targets})
        self.assertTrue(all(1 <= r <= 4 for _, r, _ in targets))


class JudgeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.known = {bonus: diag.evaluate_games(games(), evaluator(bonus))
                     for bonus in (0, 20, 25, 30, 50)}

    def test_D_and_all_four_decision_branches(self):
        for old, new, expected_d, decision in (
            (0, 30, 30, "判定不能（R_curが正と確かめられない）"),
            (50, 30, 5, "半分より多くが残る"),
            (50, 20, -5, "半分より多くは残らない"),
            (50, 25, 0, "判定不能"),
        ):
            with self.subTest(current=old, new=new):
                result = diag.judge(self.known[old], self.known[new])
                self.assertEqual(result["R_cur"]["estimate"], old)
                self.assertEqual(result["R_new"]["estimate"], new)
                self.assertEqual(result["D"], {"estimate": expected_d, "ci95": [expected_d, expected_d]})
                self.assertEqual(result["decision"], decision)

    def test_bootstrap_samples_whole_games_with_multiplicity_and_position_weights(self):
        # A has one position with d=10; B has three with d=30. R=25, not 20.
        # Drawing AA, AB/BA, BB gives R=10,25,30. The 95% limits are 10 and 30.
        current = {"positions": [raw_position("a", 20, [(2, 10, 0)])]
                   + [raw_position("b", ply, [(2, 30, 0)]) for ply in (20, 40, 60)]}
        new = {"positions": [raw_position("a", 20, [(2, 7, 0)])]
               + [raw_position("b", ply, [(2, 17, 0)]) for ply in (20, 40, 60)]}
        result = diag.judge(current, new)
        self.assertEqual(result["R_cur"], {"estimate": 25, "ci95": [10, 30]})
        self.assertEqual(result["R_new"], {"estimate": 14.5, "ci95": [7, 17]})
        self.assertEqual(result["D"], {"estimate": 2, "ci95": [2, 2]})
        self.assertEqual(result["bootstrap_replicates"], 1000)
        self.assertEqual(result["bootstrap_seed"], 1)
        self.assertEqual(diag.judge(current, new), result)

    def test_D_interval_crossing_zero_is_inconclusive(self):
        current = {"positions": [raw_position(g, 20, [(2, 50, 0)]) for g in ("a", "b")]}
        new = {"positions": [raw_position("a", 20, [(2, 0, 0)]),
                             raw_position("b", 20, [(2, 100, 0)])]}
        result = diag.judge(current, new)
        self.assertEqual(result["D"], {"estimate": 25, "ci95": [-25, 75]})
        self.assertEqual(result["decision"], "判定不能")

    def test_R_cur_interval_touching_zero_prevents_a_positive_D_decision(self):
        current = {"positions": [raw_position("a", 20, [(2, 0, 0)]),
                                 raw_position("b", 20, [(2, 100, 0)])]}
        new = {"positions": [raw_position(g, 20, [(2, 200, 0)]) for g in ("a", "b")]}
        result = diag.judge(current, new)
        self.assertEqual(result["R_cur"], {"estimate": 50, "ci95": [0, 100]})
        self.assertGreater(result["D"]["ci95"][0], 0)
        self.assertEqual(result["decision"], "判定不能（R_curが正と確かめられない）")

    def test_comparison_rejects_different_positions_targets_or_source_boards(self):
        for change in ("position", "target", "sfen", "duplicate_position", "duplicate_target"):
            with self.subTest(change=change):
                changed = copy.deepcopy(self.known[30])
                first = changed["positions"][0]
                if change == "position":
                    changed["positions"].pop()
                elif change == "target":
                    first["targets"].pop()
                elif change == "sfen":
                    first["sfen"] = first["sfen"].replace(" b ", " w ")
                elif change == "duplicate_position":
                    changed["positions"].append(copy.deepcopy(first))
                else:
                    first["targets"].append(copy.deepcopy(first["targets"][0]))
                with self.assertRaises(ValueError):
                    diag.judge(self.known[50], changed)

    def test_input_order_and_cached_R_do_not_change_the_result(self):
        changed = copy.deepcopy(self.known[30])
        changed["R"] = -9999
        changed["positions"].reverse()
        for position in changed["positions"]:
            position["targets"].reverse()
        self.assertEqual(diag.judge(self.known[50], changed),
                         diag.judge(self.known[50], self.known[30]))

    def test_no_matched_positions_is_an_explicit_error(self):
        result = {"positions": [raw_position("a", 20, [])]}
        with self.assertRaises(ValueError):
            diag.judge(result, result)

    def test_percentile_uses_linear_interpolation(self):
        self.assertAlmostEqual(diag.percentile([30, 10, 0, 20], 0.025), 0.75)
        self.assertAlmostEqual(diag.percentile([30, 10, 0, 20], 0.975), 29.25)
        self.assertEqual(diag.percentile([9], 0.025), 9)

    def test_judge_cli_writes_json_and_rejects_mismatched_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            current, new, output = (root / name for name in ("current.json", "new.json", "result.json"))
            diag.write_json(current, self.known[50])
            diag.write_json(new, self.known[30])
            command = [sys.executable, "-I", str(Path(diag.__file__)), "judge",
                       str(current), str(new), "--output", str(output)]
            run = subprocess.run(command, capture_output=True, text=True, check=True)
            saved = json.loads(output.read_text())
            self.assertEqual(saved, diag.judge(self.known[50], self.known[30]))
            self.assertIn("半分より多くが残る", run.stdout)
            changed = copy.deepcopy(self.known[30])
            changed["positions"].pop()
            diag.write_json(new, changed)
            run = subprocess.run(command, capture_output=True, text=True)
            self.assertNotEqual(run.returncode, 0)
            self.assertIn("position sets differ", run.stderr)


class ProtocolTests(unittest.TestCase):
    def test_eval_output_counts_missing_extra_and_error_lines(self):
        handshake = ["usiok", "readyok"]
        pair = ["info string evaluation 123", "info string nonlinear -40"]
        self.assertEqual(diag.parse_evaluations(handshake + pair, 1), [(123, -40)])
        for lines in (handshake, handshake + pair * 2, handshake + pair[:1],
                      handshake + pair[1:], handshake + pair + ["info string error invalid position"], pair):
            with self.subTest(lines=lines), self.assertRaises(ValueError):
                diag.parse_evaluations(lines, 1)

    def test_real_subprocess_uses_required_usi_commands(self):
        # A tiny protocol peer asserts the external command contract; no minase is started.
        with tempfile.TemporaryDirectory() as directory:
            peer = Path(directory) / "peer"
            peer.write_text(f"#!{sys.executable}\n" + '''import sys
assert sys.argv[1:] == ["--protocol", "usi", "--rules", "engine-default"]
assert input() == "usi"
print("usiok", flush=True)
assert input() == "isready"
print("readyok", flush=True)
assert input() == "usinewgame"
while True:
    command = input()
    if command == "quit":
        break
    assert command.startswith("position sfen ")
    assert input() == "eval"
    print("info string evaluation 123")
    print("info string nonlinear -40")
    print("info string end", flush=True)
''')
            peer.chmod(0o700)
            sfens = [diag.START + " b - 1", diag.START + " w - 1"]
            self.assertEqual(diag.usi_evaluate(peer, sfens), [(123, -40), (123, -40)])


if __name__ == "__main__":
    unittest.main()
