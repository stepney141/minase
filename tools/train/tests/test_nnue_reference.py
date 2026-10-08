"""設計書の手計算例、丸め、および左右鏡映の整数不変性を検証する。"""

from pathlib import Path
import tempfile
import unittest

import numpy as np

from helpers import records_with, write_mnsd
from minase_train.data.features import mirror
from minase_train.data.mnpt import initial_piece_values, write_mnpt
from minase_train.data.mnsd import map_records
from minase_train.nnue.format import read_mnue, write_mnue
from minase_train.nnue.reference import activate, evaluate_details, integer_evaluate, nonlinear_cp, output_scale
from minase_train.nnue.train import main


class ReferenceTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def test_two_active_features_follow_all_six_steps(self):
        linear = np.zeros((13680, 2), dtype=np.int16)
        weights = np.zeros((13680, 2), dtype=np.int16)
        # 先手の歩兵が0升、先獅子が13升。相手視点は段だけが反転する。
        for feature, endpoints, hidden in (
            (29 * 144, [80, -81], [40, -20]), (13536 + 13, [0, 8], [30, 50]),
            (76 * 144 + 132, [48, 16], [120, 0]), (13536 + 121, [8, -8], [-5, -40]),
        ):
            reflected = feature // 12 * 12 + 11 - feature % 12
            linear[feature] = linear[reflected] = endpoints
            weights[feature], weights[reflected] = hidden, hidden[::-1]
        path = self.root / "net.bin"
        write_mnue(path, linear, initial_piece_values(), 254, weights, np.array([10, 10]),
                   np.array([[4096, 4096], [-2048, -2048]]), -7)
        records = records_with([[(0, 1)]], lion=13)
        result = evaluate_details(read_mnue(path), records["board"], records["stm"], records["lion"])
        np.testing.assert_array_equal(result.linear_accumulators, [[[80, -73], [56, 8]]])
        np.testing.assert_array_equal(result.accumulators, [[[80, 40], [125, -30]]])
        np.testing.assert_array_equal(result.activations, [[[50, 12], [122, 0]]])
        np.testing.assert_array_equal(result.linear_cp, [-9])
        np.testing.assert_array_equal(result.output, [4089])
        np.testing.assert_array_equal(result.cp_nl, [2])
        np.testing.assert_array_equal(result.evaluation, [-7])
        self.assertEqual(result.evaluation.dtype, np.int32)

    def test_activation_and_two_distinct_rounding_rules(self):
        np.testing.assert_array_equal(activate(np.array([-1016, -1, 0, 1, 11, 12, 64, 126, 127, 10000])),
                                      [0, 0, 0, 0, 0, 1, 32, 124, 126, 126])
        self.assertEqual(output_scale(19.84375), 2)  # 2.5を偶数へ丸める。
        self.assertEqual(output_scale(27.78125), 4)  # 3.5を偶数へ丸める。
        np.testing.assert_array_equal(nonlinear_cp(np.array([-3073, -3072, -1025, -1024, 1023, 1024, 3072]), 254),
                                      [-2, -1, -1, 0, 0, 1, 2])

    def test_linear_is_clipped_only_after_adding_nonlinear(self):
        path = self.root / "clip.bin"
        write_mnue(path, np.full((13680, 2), 32767), initial_piece_values(), 1000,
                   np.zeros((13680, 2), dtype=np.int16), np.zeros(2, dtype=np.int32),
                   np.zeros((2, 2), dtype=np.int16), -4161536)
        records = records_with([[(i, 1) for i in range(9)]])
        result = evaluate_details(read_mnue(path), records["board"], records["stm"], records["lion"])
        self.assertEqual(result.linear_cp[0], 36862)
        self.assertEqual(result.cp_nl[0], -8001)
        self.assertEqual(result.evaluation[0], 28861)

    def test_maximum_width_and_firing_count_fit_i32(self):
        path = self.root / "max.bin"
        write_mnue(path, np.full((13680, 2), 32767), initial_piece_values(), 100000,
                   np.full((13680, 512), 1016), np.full(512, 1016),
                   np.full((2, 512), 8192), 4161536)
        records = records_with([[(i, 1) for i in range(144)]], lion=0)
        result = evaluate_details(read_mnue(path), records["board"], records["stm"], records["lion"])
        self.assertTrue(np.all(result.accumulators == 148336))
        self.assertTrue(np.all(result.linear_accumulators == 145 * 32767))
        self.assertEqual(result.output[0], 1024 * 126 * 8192 + 4161536)
        self.assertEqual(result.evaluation[0], 28999)

    def test_seeded_random_network_matches_mirrors_for_mnsd_positions(self):
        rng = np.random.default_rng(521)
        board = rng.choice(np.array([0, 1, 65, 12, 76, 47, 111, 51, 115], dtype=np.uint8), (30, 144))
        data = self.root / "positions.bin"
        write_mnsd(data, seed=1, checksum=b"a" * 32, games=list(range(30)), board=board)
        records = map_records(data)
        base = self.root / "base.bin"
        zeros = np.zeros(13680, dtype=np.int16)
        write_mnpt(base, zeros, zeros, initial_piece_values(), 1072.6529541015625)
        for width in (2, 4, 8):
            path = self.root / f"random{width}.bin"
            main(["random", "--width", str(width), "--seed", "12", "--base", str(base), "--output", str(path)])
            copy = self.root / f"copy{width}.bin"
            main(["random", "--width", str(width), "--seed", "12", "--base", str(base), "--output", str(copy)])
            self.assertEqual(path.read_bytes(), copy.read_bytes())
            net = read_mnue(path)
            for side in (0, 1):
                stm = np.full(len(board), side, dtype=np.uint8)
                lion = np.arange(len(board), dtype=np.uint8)
                lion[::3] = 255
                reflected, reflected_lion = mirror(records["board"], lion)
                np.testing.assert_array_equal(integer_evaluate(net, board, stm, lion),
                                              integer_evaluate(net, reflected, stm, reflected_lion))


if __name__ == "__main__":
    unittest.main()
