"""shuffle.rsの既知値で、対局単位の検証分割を固定する。"""

import unittest

import numpy as np

from minase_zero.data import hash64, is_validation


class SplitTests(unittest.TestCase):
    def test_rust_hash_vectors(self) -> None:
        for seed, game, expected in (
            (0, 0, 0),
            (1, 0, 0x5692_161D_100B_05E5),
            (0, 1, 0xE220_A839_7B1D_CDAF),
            (0xFFFF_FFFF_FFFF_FFFF, 0xFFFF_FFFF, 0x3A9B_57C2_77B2_2E0A),
        ):
            with self.subTest(seed=seed, game=game):
                actual = hash64(seed, np.array([game], dtype=np.uint32))
                self.assertEqual(actual.dtype, np.uint64)
                self.assertEqual(actual.tolist(), [expected])

    def test_rust_validation_games(self) -> None:
        games = np.arange(80, dtype=np.uint32)
        for seed, expected in ((11, [4, 15, 22, 41, 48, 59, 70]), (22, [6, 8, 35, 44, 58, 63, 74])):
            with self.subTest(seed=seed):
                actual = is_validation(seed, games)
                self.assertEqual(actual.dtype, np.bool_)
                self.assertEqual(games[actual].tolist(), expected)

    def test_empty_games_and_invalid_ranges(self) -> None:
        self.assertEqual(hash64(11, np.empty(0, dtype=np.uint32)).shape, (0,))
        for seed, games in (
            (-1, np.array([0])), (1 << 64, np.array([0])),
            (11, np.array([-1])), (11, np.array([1 << 32])), (11, np.array([1.5])),
        ):
            with self.subTest(seed=seed, games=games):
                with self.assertRaises(ValueError):
                    hash64(seed, games)


if __name__ == "__main__":
    unittest.main()
