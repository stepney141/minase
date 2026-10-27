"""Rustが書いた288局面の入力と全合法手のラベルを独立した期待値にする。"""

import json
from pathlib import Path
import unittest

import numpy as np
import torch

from minase_zero.encoding import encode_planes, policy_labels


class EncodingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        fixture = Path(__file__).resolve().parents[1] / "fixtures" / "nn-positions.jsonl"
        cls.fixtures = [json.loads(line) for line in fixture.read_text(encoding="utf-8").splitlines()]
        cls.previous_threads = torch.get_num_threads()
        torch.set_num_threads(4)

    @classmethod
    def tearDownClass(cls) -> None:
        torch.set_num_threads(cls.previous_threads)

    def test_all_fixture_input_planes(self) -> None:
        self.assertEqual(len(self.fixtures), 288)
        boards = np.zeros((288, 144), dtype=np.uint8)
        sides = np.empty(288, dtype=np.uint8)
        lions = np.empty(288, dtype=np.uint8)
        for index, fixture in enumerate(self.fixtures):
            sides[index] = int(fixture["side_to_move"] == "white")
            lions[index] = 255 if fixture["lion_capture"] is None else fixture["lion_capture"]
            for square, piece in enumerate(fixture["board"]):
                if piece is not None:
                    color, kind, promoted = piece
                    # recordsの駒コードの仕様式で変換する。面の期待値はfixtureのinputだけから得る。
                    boards[index, square] = 1 + kind + 29 * int(promoted) + 64 * color
        actual = encode_planes(torch.from_numpy(boards), torch.from_numpy(sides), torch.from_numpy(lions))
        self.assertEqual(actual.shape, (288, 95, 12, 12))
        self.assertEqual(actual.dtype, torch.float32)
        self.assertTrue(torch.all((actual == 0) | (actual == 1)))
        flat = actual.reshape(288, -1)
        for index, fixture in enumerate(self.fixtures):
            with self.subTest(position=index):
                self.assertEqual(flat[index].nonzero().flatten().tolist(), fixture["input"])

    def test_all_fixture_policy_labels(self) -> None:
        moves = []
        sides = []
        for fixture in self.fixtures:
            moves.extend([start, 255 if middle is None else middle, end, promote, label]
                         for start, middle, end, promote, label in fixture["moves"])
            sides.extend([int(fixture["side_to_move"] == "white")] * len(fixture["moves"]))
        columns = np.array(moves, dtype=np.int64)
        actual = policy_labels(*(columns[:, index] for index in range(4)), np.array(sides, dtype=np.uint8))
        np.testing.assert_array_equal(actual, columns[:, 4])
        self.assertEqual(actual.dtype, np.int64)

    def test_empty_batches(self) -> None:
        empty = torch.empty(0, dtype=torch.uint8)
        actual = encode_planes(torch.empty((0, 144), dtype=torch.uint8), empty, empty)
        self.assertEqual(actual.shape, (0, 95, 12, 12))
        moves = np.empty(0, dtype=np.uint8)
        self.assertEqual(policy_labels(moves, moves, moves, moves, moves).shape, (0,))

    def test_deferred_and_invalid_piece_codes_are_rejected(self) -> None:
        side = torch.zeros(1, dtype=torch.uint8)
        lion = torch.full((1,), 255, dtype=torch.uint8)
        for code in (129, 64, 127, 30, 22):
            with self.subTest(code=code):
                board = torch.zeros((1, 144), dtype=torch.uint8)
                board[0, 0] = code
                with self.assertRaises(ValueError):
                    encode_planes(board, side, lion)

    def test_invalid_tensor_shapes_types_and_coordinates_are_rejected(self) -> None:
        boards = torch.zeros((1, 144), dtype=torch.uint8)
        side = torch.zeros(1, dtype=torch.uint8)
        lion = torch.full((1,), 255, dtype=torch.uint8)
        for inputs in (
            (boards.float(), side, lion), (boards[:, :143], side, lion),
            (boards, side[:, None], lion), (boards, side + 2, lion),
            (boards, side, torch.tensor([144], dtype=torch.uint8)),
        ):
            with self.subTest(shapes=[tuple(t.shape) for t in inputs]):
                with self.assertRaises(ValueError):
                    encode_planes(*inputs)

    def test_rust_invalid_move_shapes_are_rejected(self) -> None:
        # nn/policy.rsのinvalid_move_shapes_panicと同じ幾何学的な入力である。
        for middle, end, promote in (
            (65, 77, 0), (77, 77, 0), (89, 101, 0), (77, 89, 1),
            (77, 65, 1), (255, 65, 1), (255, 90, 1), (255, 103, 0),
        ):
            for side in (0, 1):
                with self.subTest(middle=middle, end=end, promote=promote, side=side):
                    arrays = [np.array([value], dtype=np.int64) for value in (65, middle, end, promote, side)]
                    with self.assertRaises(ValueError):
                        policy_labels(*arrays)

    def test_invalid_policy_arrays_are_rejected(self) -> None:
        for values in ((-1, 255, 12, 0, 0), (144, 255, 12, 0, 0), (0, 144, 12, 0, 0),
                       (0, 255, 144, 0, 0), (0, 255, 12, 2, 0), (0, 255, 12, 0, 2)):
            with self.subTest(values=values):
                with self.assertRaises(ValueError):
                    policy_labels(*(np.array([value]) for value in values))
        arrays = [np.array([value]) for value in (0, 255, 12, 0, 0)]
        with self.assertRaises(ValueError):
            policy_labels(arrays[0].astype(float), *arrays[1:])
        with self.assertRaises(ValueError):
            policy_labels(arrays[0][:0], *arrays[1:])
        with self.assertRaises(ValueError):
            policy_labels(*(array[:, None] for array in arrays))


if __name__ == "__main__":
    unittest.main()
