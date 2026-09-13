"""MNSDストリーミング学習器の契約を単独実行で検証する。"""

from __future__ import annotations

from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path
import re
import struct
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

from features import (
    FEATURE_COUNT,
    INITIAL_BOARD,
    MIRRORED_FEATURE_COUNT,
    PADDING_INDEX,
    canonical_feature_indices,
    feature_indices,
    mirror,
)
from mnsd import Dataset, HEADER_LENGTH, RECORD_DTYPE, RECORD_LENGTH, hash64
from taper import band_indices, phase_numerators, phase_ratios, piece_counts
from train_pst import (
    FILE_LENGTH,
    HEADER_LENGTH as MNPT_HEADER_LENGTH,
    build_targets,
    estimate_generation_ks,
    estimate_k,
    expanded_model_weights,
    float_evaluate,
    float_weights_path,
    initial_piece_values,
    integer_evaluate,
    main as train_main,
    make_model,
    model_logits,
    quantize,
    read_mnpt,
    train_epoch,
    write_mnpt,
)

PIECE_VALUES = initial_piece_values()


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
        # 既存データの検証分割を保つ回帰値。厳密なハッシュ式は規範文書に未定義。
        # 0、シード差、u64の桁あふれを含む既知の参照値を固定する。
        for seed, game, expected in (
            (0, 0, 0),
            (1, 0, 0x5692161D100B05E5),
            (0, 1, 0xE220A8397B1DCDAF),
            (0xFFFFFFFFFFFFFFFF, 0xFFFFFFFF, 0x3A9B57C277B22E0A),
        ):
            with self.subTest(seed=seed, game=game):
                self.assertEqual(int(hash64(seed, np.array([game], dtype=np.uint32))[0]), expected)
        validation_games = {
            11: {4, 15, 22, 41, 48, 59, 70},
            22: {6, 8, 35, 44, 58, 63, 74},
        }

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
                            bool(selected), int(game) in validation_games[header.seed]
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

            single = torch.nn.Embedding(2, 1)
            with torch.no_grad():
                single.weight[:, 0] = torch.tensor([30.0, 10.0])
            phi = torch.tensor([0.25])
            logits = model_logits(single, torch.tensor([[0, 1]]), phi, 200.0)
            self.assertAlmostEqual(float(logits.item()), 0.2)
            tapered = torch.nn.Embedding(2, 2)
            with torch.no_grad():
                tapered.weight.copy_(torch.tensor([[30.0, 100.0], [10.0, 300.0]]))
            # φ=0.25: 0.25×40 + 0.75×400 = 310 → 310/200。
            logits = model_logits(tapered, torch.tensor([[0, 1]]), phi, 200.0)
            self.assertAlmostEqual(float(logits.item()), 1.55)


class TrainingPathTest(unittest.TestCase):
    """訓練CLIの最良重み、教師値、観測数、検証標本を検証する。"""

    def test_training_path_uses_documented_data_membership(self) -> None:
        count = 256
        games = list(range(count))
        asymmetric_board = INITIAL_BOARD.copy()
        asymmetric_board[[0, 60]] = asymmetric_board[[60, 0]]

        # 対局の所属を固定し、分割処理が変わって教師値まで同時に変わるのを防ぐ。
        validation_games = {
            101: {28, 36, 51, 68, 74, 82, 96, 97, 101, 143, 152, 181, 190, 200, 218, 224, 231, 243, 251},
            202: {45, 61, 73, 120, 171, 213, 230},
        }

        def labels(seed: int, training_score: int) -> tuple[list[int], list[int]]:
            selected = validation_games[seed]
            return (
                [0 if game in selected else training_score for game in games],
                [1 if game in selected else 2 for game in games],
            )

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            generation0 = root / "generation0.mnsd"
            generation1 = root / "generation1.mnsd"
            initial_path = root / "initial.mnpt"
            output_path = root / "trained.mnpt"
            scores0, results0 = labels(101, 1000)
            scores1, results1 = labels(202, -1000)
            write_mnsd(
                generation0,
                seed=101,
                checksum=b"a" * 32,
                games=games,
                scores=scores0,
                results=results0,
                board=asymmetric_board,
            )
            write_mnsd(
                generation1,
                seed=202,
                checksum=b"b" * 32,
                games=games,
                scores=scores1,
                results=results1,
                board=asymmetric_board,
            )
            initial_weights = np.zeros(FEATURE_COUNT, dtype="<i2")
            write_mnpt(initial_path, initial_weights, initial_weights, PIECE_VALUES, 200.0)

            dataset = Dataset([generation0, generation1])
            expected_ks = []
            for generation in range(dataset.generation_count):
                records = dataset.gather(
                    dataset.generation_training_indices(generation)
                )
                expected_ks.append(estimate_k(records["score"], records["result"]))

            # 特徴は2陣営×47駒状態×144升＋先獅子144升の13,680個。
            # 全訓練局面が同じ92枚の盤面で、検証対局は19＋7局なので、
            # 未観測は13,588個、各観測特徴の頻度は512−26＝486回になる。
            # 鏡映拡張を頻度へ二重計上する変更はこの固定値と一致しない。
            stdout = StringIO()
            with redirect_stdout(stdout):
                train_main(
                    [
                        "train",
                        "--data",
                        str(generation0),
                        str(generation1),
                        "--output",
                        str(output_path),
                        "--init",
                        str(initial_path),
                        "--model",
                        "single",
                        "--k",
                        "200",
                        "--lambda",
                        "0.75",
                        "--lr",
                        "10",
                        "--epochs",
                        "1",
                        "--batch",
                        "128",
                        "--seed",
                        "1",
                        "--validation-sample",
                        "10000",
                        "--device",
                        "cpu",
                    ]
                )
            output = stdout.getvalue()

            self.assertIn(
                "teacher K: "
                f"generation 0 = {expected_ks[0]:.9f}, "
                f"generation 1 = {expected_ks[1]:.9f}",
                output,
            )
            self.assertRegex(output, r"best epoch: 0 validation_loss=")
            observation_log = re.search(
                r"feature observations: unobserved=(\d+).* max=(\d+) mean=",
                output,
            )
            self.assertIsNotNone(observation_log)
            assert observation_log is not None
            self.assertEqual(int(observation_log.group(1)), 13_588)
            self.assertEqual(int(observation_log.group(2)), 486)
            self.assertIn(
                "quantization error: samples=26 ",
                output,
            )
            middlegame, endgame, piece_values, output_k = read_mnpt(output_path)
            np.testing.assert_array_equal(middlegame, initial_weights)
            np.testing.assert_array_equal(endgame, initial_weights)
            np.testing.assert_array_equal(piece_values, PIECE_VALUES)
            self.assertEqual(output_k, 200.0)
            saved = np.load(float_weights_path(output_path))
            np.testing.assert_array_equal(saved["middlegame"], 0)
            np.testing.assert_array_equal(saved["endgame"], 0)


class TaperedFormatTest(unittest.TestCase):
    """補間係数、整数評価の式、MNPTバージョン2の検査、およびモデル種別の契約を検証する。"""

    def test_phase_numerator_counts_board_pieces_only(self) -> None:
        boards = np.zeros((4, 144), dtype=np.uint8)
        boards[0] = INITIAL_BOARD  # 92枚
        boards[1, :2] = 12  # 王2枚
        boards[2, :47] = 1  # 47枚 → q=45
        boards[3, :] = 1  # 144枚 → 上限
        np.testing.assert_array_equal(piece_counts(boards), [92, 2, 47, 144])
        np.testing.assert_array_equal(phase_numerators(boards), [90, 0, 45, 90])
        np.testing.assert_array_equal(phase_ratios(boards), [1.0, 0.0, 0.5, 1.0])
        # 先獅子の対象升は特徴だが駒ではないので、局面の駒数に影響しない。
        features = feature_indices(boards[:1], np.zeros(1, dtype=np.uint8), np.array([5], dtype=np.uint8))
        self.assertEqual(int((features != PADDING_INDEX).sum()), 93)
        np.testing.assert_array_equal(
            band_indices(np.array([0.0, 0.19, 0.2, 0.4, 0.6, 0.8, 0.99, 1.0])), [0, 0, 1, 2, 3, 4, 4, 4]
        )

    def test_integer_evaluate_interpolates_truncates_and_clips(self) -> None:
        middlegame = np.zeros(FEATURE_COUNT, dtype=np.int16)
        endgame = np.zeros(FEATURE_COUNT, dtype=np.int16)
        middlegame[0] = 800  # 100 cp
        endgame[0] = -1600  # -200 cp
        features = np.full((5, 145), PADDING_INDEX, dtype=np.int32)
        features[:, 0] = 0
        numerators = np.array([90, 0, 45, 1, 89], dtype=np.int64)
        # q=45: (45×800 + 45×(−1600))/720 = −50。q=1: (800 − 89×1600)/720 = −196.6 → −196。
        # q=89: (89×800 − 1600)/720 = 96.67 → 96。
        np.testing.assert_array_equal(
            integer_evaluate(middlegame, endgame, features, numerators), [100, -200, -50, -196, 96]
        )
        # 分子−719は0、−720は−1へ切り捨てる。
        middlegame[0] = 0
        endgame[0] = 0
        middlegame[1] = -719
        endgame[1] = 0
        features[:, 1] = 1
        np.testing.assert_array_equal(
            integer_evaluate(middlegame, endgame, features[:1], np.array([1])), [0]
        )
        middlegame[1] = -720
        np.testing.assert_array_equal(
            integer_evaluate(middlegame, endgame, features[:1], np.array([1])), [-1]
        )
        clipped = np.full(FEATURE_COUNT, 32_000, dtype=np.int16)
        wide = np.zeros((1, 145), dtype=np.int32)
        wide[0, :144] = np.arange(144)
        wide[0, 144] = PADDING_INDEX
        np.testing.assert_array_equal(
            integer_evaluate(clipped, clipped, wide, np.array([30])), [28_999]
        )
        with self.assertRaises(ValueError):
            integer_evaluate(middlegame, endgame, features[:1], np.array([91]))

    def test_mnpt_corruption_and_inconsistent_piece_values_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "weights.bin"
            weights = np.arange(FEATURE_COUNT, dtype=np.int16)
            write_mnpt(path, weights, -weights, PIECE_VALUES, 300.0)
            self.assertEqual(path.stat().st_size, FILE_LENGTH)
            middlegame, endgame, values, k = read_mnpt(path)
            np.testing.assert_array_equal(middlegame, weights)
            np.testing.assert_array_equal(endgame, -weights)
            np.testing.assert_array_equal(values, PIECE_VALUES)
            self.assertEqual(k, 300.0)
            original = path.read_bytes()
            corruptions = {
                "length": original[:-1],
                "magic": b"MNPX" + original[4:],
                "version": original[:4] + struct.pack("<I", 1) + original[8:],
                "checksum": original[:MNPT_HEADER_LENGTH] + bytes([original[MNPT_HEADER_LENGTH] ^ 1]) + original[MNPT_HEADER_LENGTH + 1:],
            }
            for name, content in corruptions.items():
                with self.subTest(name=name):
                    path.write_bytes(content)
                    with self.assertRaises(ValueError):
                        read_mnpt(path)
            bad_values = [
                ("nonpositive", {29: 0}),
                ("royal", {11: 2601}),
                ("range", {4: 29_000, 11: 29_100, 21: 29_100}),
            ]
            for name, changes in bad_values:
                with self.subTest(name=name):
                    values = PIECE_VALUES.copy()
                    for state, value in changes.items():
                        values[state] = value
                    with self.assertRaises(ValueError):
                        write_mnpt(path, weights, weights, values, 300.0)

    def test_model_endpoints_are_validated_and_tapered_training_updates_the_active_endpoint(self) -> None:
        games = list(range(64))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = root / "data.mnsd"
            write_mnsd(data, seed=5, checksum=b"a" * 32, games=games, scores=[300] * 64, results=[2] * 64)
            initial_path = root / "initial.mnpt"
            output_path = root / "trained.mnpt"
            weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
            write_mnpt(initial_path, weights, weights + 8, PIECE_VALUES, 200.0)
            arguments = [
                "train", "--data", str(data), "--output", str(output_path), "--init", str(initial_path),
                "--model", "single", "--k", "200", "--lr", "1", "--epochs", "1", "--batch", "16",
                "--seed", "1", "--validation-sample", "100", "--device", "cpu",
            ]
            with redirect_stdout(StringIO()):
                with self.assertRaises(ValueError):
                    train_main(arguments)
            self.assertFalse(output_path.exists())
            write_mnpt(initial_path, weights, weights, PIECE_VALUES, 200.0)
            arguments[8] = "tapered"
            with redirect_stdout(StringIO()):
                train_main(arguments)
            middlegame, endgame, values, _ = read_mnpt(output_path)
            np.testing.assert_array_equal(values, PIECE_VALUES)
            # 初期局面は q=90 なので、終盤側の勾配は0で初期値のまま残る。
            np.testing.assert_array_equal(endgame, weights)
            self.assertTrue(np.any(middlegame != 0))


class MirroredModelTest(unittest.TestCase):
    """strength-stage7.md「鏡映の重み共有」の全特徴と学習経路の契約を検証する。"""

    def test_canonical_mapping_preserves_ranks_and_has_exactly_6840_pairs(self) -> None:
        # 各陣営・駒状態と先獅子の表は12段×12筋で、左右の筋だけを同一視する。
        indices = np.arange(FEATURE_COUNT, dtype=np.int32).reshape(95, 12, 12)
        canonical = canonical_feature_indices(indices)
        np.testing.assert_array_equal(canonical, canonical[:, :, ::-1])
        values, counts = np.unique(canonical, return_counts=True)
        np.testing.assert_array_equal(values, np.arange(6840))
        np.testing.assert_array_equal(counts, 2)
        self.assertEqual(MIRRORED_FEATURE_COUNT, 6840)
        # 先頭、中央、次段、次状態、敵駒、先獅子、末尾、paddingの境界。
        np.testing.assert_array_equal(
            canonical_feature_indices(np.array([0, 5, 6, 11, 12, 144, 6768, 13536, 13679, 13680])),
            [0, 5, 5, 0, 6, 72, 3384, 6768, 6834, 6840],
        )

    def test_initialization_averages_each_endpoint_and_padding_stays_zero_after_update(self) -> None:
        random = np.random.default_rng(731)
        initial = random.integers(-16_000, 16_000, size=(FEATURE_COUNT, 2)).astype(np.float32) / 8.0
        model = make_model(torch.from_numpy(initial), torch.device("cpu"), "mirrored")
        self.assertEqual(tuple(model.weight.shape), (6841, 2))
        self.assertEqual(sum(parameter.numel() for parameter in model.parameters()), 6841 * 2)
        expected = (initial.reshape(95, 12, 12, 2) + initial.reshape(95, 12, 12, 2)[:, :, ::-1]) / 2
        np.testing.assert_array_equal(expanded_model_weights(model).detach().numpy(), expected.reshape(FEATURE_COUNT, 2))
        # 全特徴を個別に入力して、各鏡映対が片側の更新を共有することを確認する。
        features = torch.arange(FEATURE_COUNT + 1).reshape(-1, 1)
        phi = torch.linspace(0, 1, FEATURE_COUNT + 1)
        optimizer = torch.optim.SGD(model.parameters(), lr=0.1)
        model_logits(model, features, phi, 200.0).sum().backward()
        optimizer.step()
        expanded = expanded_model_weights(model).detach().numpy().reshape(95, 12, 12, 2)
        np.testing.assert_array_equal(expanded, expanded[:, :, ::-1])
        np.testing.assert_array_equal(model(torch.tensor([[PADDING_INDEX]])).detach().numpy(), 0)

    def test_expansion_matches_tapered_predictions_and_initial_perspectives(self) -> None:
        random = np.random.default_rng(432)
        initial = torch.from_numpy(random.normal(0.0, 100.0, (FEATURE_COUNT, 2)).astype(np.float32))
        model = make_model(initial, torch.device("cpu"), "mirrored")
        expanded = expanded_model_weights(model).detach()
        tapered = make_model(expanded, torch.device("cpu"), "tapered")
        # 0, 2, 47, 92枚と全盤面の境界、両視点、先獅子対象升を含む。
        boards = np.zeros((6, 144), dtype=np.uint8)
        boards[1, [5, 138]] = [12, 76]
        boards[2, :47] = 1
        boards[3] = INITIAL_BOARD
        boards[4] = INITIAL_BOARD
        boards[5, :] = random.choice([1, 30, 65, 94], 144)
        stm = np.array([0, 1, 0, 0, 1, 1], dtype=np.uint8)
        lion = np.array([255, 255, 17, 255, 255, 143], dtype=np.uint8)
        features = feature_indices(boards, stm, lion)
        phi = torch.from_numpy(phase_ratios(boards).astype(np.float32))
        logits = model_logits(model, torch.from_numpy(features), phi, 1072.6529541015625)
        torch.testing.assert_close(logits, model_logits(tapered, torch.from_numpy(features), phi, 1072.6529541015625))
        reflected_board, reflected_lion = mirror(boards, lion)
        reflected = feature_indices(reflected_board, stm, reflected_lion)
        torch.testing.assert_close(logits, model_logits(model, torch.from_numpy(reflected), phi, 1072.6529541015625))
        floating = float_evaluate(expanded[:, 0].numpy(), expanded[:, 1].numpy(), features, phase_ratios(boards))
        self.assertAlmostEqual(float(floating[3]), float(floating[4]), delta=0.001)
        self.assertAlmostEqual(float(logits[3].detach()), float(logits[4].detach()), delta=0.000001)
        quantized = quantize(expanded.numpy())
        integer = integer_evaluate(quantized[:, 0], quantized[:, 1], features, phase_numerators(boards))
        self.assertEqual(int(integer[3]), int(integer[4]))
        np.testing.assert_array_equal(
            integer,
            integer_evaluate(quantized[:, 0], quantized[:, 1], reflected, phase_numerators(boards)),
        )

    def test_training_skips_mirror_augmentation_and_exports_symmetric_mnpt_v2(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = root / "data.mnsd"
            initial_path = root / "initial.mnpt"
            output_path = root / "trained.mnpt"
            board = INITIAL_BOARD.copy()
            board[[0, 60]] = board[[60, 0]]
            write_mnsd(data, seed=5, checksum=b"a" * 32, games=list(range(64)), scores=[300] * 64, results=[2] * 64, board=board)
            weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
            write_mnpt(initial_path, weights, weights, PIECE_VALUES, 200.0)
            arguments = [
                "train", "--data", str(data), "--output", str(output_path), "--init", str(initial_path),
                "--model", "mirrored", "--k", "200", "--lr", "0.1", "1", "--epochs", "1", "--batch", "16",
                "--seed", "1", "--validation-sample", "100", "--device", "cpu",
            ]
            with patch("train_pst.mirror", side_effect=AssertionError("mirrored must not augment data")):
                with redirect_stdout(StringIO()):
                    train_main(arguments)
            middlegame, endgame, values, k = read_mnpt(output_path)
            self.assertEqual(struct.unpack_from("<I", output_path.read_bytes(), 4)[0], 2)
            self.assertEqual(k, 200.0)
            np.testing.assert_array_equal(values, PIECE_VALUES)
            for endpoint in (middlegame, endgame):
                table = endpoint.reshape(95, 12, 12)
                np.testing.assert_array_equal(table, table[:, :, ::-1])
            np.testing.assert_array_equal(endgame, weights)
            self.assertTrue(np.any(middlegame != 0))
            with np.load(float_weights_path(output_path)) as floating:
                for endpoint in ("middlegame", "endgame"):
                    table = floating[endpoint].reshape(95, 12, 12)
                    np.testing.assert_array_equal(table, table[:, :, ::-1])

    def test_tapered_training_retains_mirror_augmentation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            data = Path(directory) / "data.mnsd"
            write_mnsd(data, seed=5, checksum=b"a" * 32, games=list(range(64)))
            device = torch.device("cpu")
            model = make_model(torch.zeros((FEATURE_COUNT, 2)), device, "tapered")
            with patch("train_pst.mirror", wraps=mirror) as augment:
                train_epoch(
                    model, torch.optim.SGD(model.parameters(), lr=0.1), Dataset([data]),
                    np.array([200.0]), 200.0, 0.75, 64,
                    torch.Generator().manual_seed(1), device,
                )
            self.assertTrue(augment.called)


class WeightProjectionTest(unittest.TestCase):
    """各Adam更新後に、MNPTのi16/8範囲へ射影する承認済みの契約を検証する。"""

    LOWER_CP = -4096.0
    UPPER_CP = 4095.875

    def make_case(self, path: Path, kind: str, direction: str):
        # 48枚なら両端点に勾配が流れる。正負24枚ずつの初期評価を0にして、
        # 飽和していない実際の損失からAdamが上下の保存境界を越えるようにする。
        board = np.zeros(144, dtype=np.uint8)
        board[:48] = 1
        games = np.arange(100, dtype=np.uint32)
        game = int(games[hash64(5, games) % 20 != 0][0])
        write_mnsd(
            path, seed=5, checksum=b"p" * 32, games=[game, game], board=board,
            scores=[32767 if direction == "upper" else -32768] * 2,
            results=[2 if direction == "upper" else 0] * 2,
        )
        dataset = Dataset([path])
        self.assertEqual(dataset.training_indices.size, 2)
        columns = 1 if kind == "single" else 2
        initial = np.full((FEATURE_COUNT, columns), 0.03125, dtype=np.float32)
        if columns == 2:
            initial[:, 1] = -0.03125
        initial[29 * 144:29 * 144 + 24] = 4095.0
        initial[29 * 144 + 24:29 * 144 + 48] = -4095.0
        model = make_model(torch.from_numpy(initial), torch.device("cpu"), kind)
        return model, dataset

    def run_epoch(self, model, optimizer, dataset) -> None:
        train_epoch(
            model, optimizer, dataset, np.array([1072.6529541015625]),
            1072.6529541015625, 0.75, 1,
            torch.Generator().manual_seed(1), torch.device("cpu"),
        )

    def test_each_adam_step_projects_before_the_next_batch_and_epoch_end(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            for kind in ("single", "tapered", "mirrored"):
                for direction in ("upper", "lower"):
                    with self.subTest(model=kind, direction=direction):
                        model, dataset = self.make_case(Path(directory) / f"{kind}-{direction}.mnsd", kind, direction)
                        optimizer = torch.optim.Adam(model.parameters(), lr=3.0)
                        before_forward = []
                        after_adam = []

                        def before(_model, _inputs):
                            before_forward.append(model.weight.detach().numpy().copy())

                        def after(_optimizer, _args, _kwargs):
                            after_adam.append(model.weight.detach().numpy().copy())

                        forward_hook = model.register_forward_pre_hook(before)
                        step_hook = optimizer.register_step_post_hook(after)
                        try:
                            self.run_epoch(model, optimizer, dataset)
                        finally:
                            forward_hook.remove()
                            step_hook.remove()
                        self.assertEqual(len(before_forward), 2)
                        self.assertEqual(len(after_adam), 2)
                        observed = before_forward[1:] + [model.weight.detach().numpy().copy()]
                        for raw, weights in zip(after_adam, observed):
                            crossed = raw > self.UPPER_CP if direction == "upper" else raw < self.LOWER_CP
                            self.assertTrue(np.all(crossed.any(axis=0)), "each endpoint must cross the boundary after Adam")
                            self.assertTrue(np.all(weights >= self.LOWER_CP), "next batch or epoch end sees a weight below the lower bound")
                            self.assertTrue(np.all(weights <= self.UPPER_CP), "next batch or epoch end sees a weight above the upper bound")
                            np.testing.assert_array_equal(weights[raw < self.LOWER_CP], self.LOWER_CP)
                            np.testing.assert_array_equal(weights[raw > self.UPPER_CP], self.UPPER_CP)
                            inside = (raw >= self.LOWER_CP) & (raw <= self.UPPER_CP)
                            self.assertTrue(np.any(inside & (raw * 8 != np.rint(raw * 8))))
                            np.testing.assert_array_equal(weights[inside], raw[inside])
                            np.testing.assert_array_equal(weights[-1], 0)
                        if kind == "mirrored":
                            expanded = expanded_model_weights(model).detach().numpy().reshape(95, 12, 12, 2)
                            np.testing.assert_array_equal(expanded, expanded[:, :, ::-1])

    def test_nonfinite_adam_weights_are_rejected_before_any_projection(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            for kind in ("single", "tapered", "mirrored"):
                for endpoint in range(1 if kind == "single" else 2):
                    for value in (np.nan, np.inf, -np.inf):
                        with self.subTest(model=kind, endpoint=endpoint, value=value):
                            model, dataset = self.make_case(Path(directory) / f"{kind}-{endpoint}.mnsd", kind, "upper")
                            optimizer = torch.optim.Adam(model.parameters(), lr=3.0)
                            after_adam = []

                            def inject(_optimizer, _args, _kwargs):
                                # 実Adam更新後の異常を作り、有限な範囲外値も残して
                                # 非有限値の拒否より先にclampが走らないことを検証する。
                                with torch.no_grad():
                                    model.weight[0, endpoint] = float(value)
                                after_adam.append(model.weight.detach().numpy().copy())

                            hook = optimizer.register_step_post_hook(inject)
                            try:
                                with self.assertRaises(ValueError):
                                    self.run_epoch(model, optimizer, dataset)
                            finally:
                                hook.remove()
                            self.assertEqual(len(after_adam), 1)
                            raw = after_adam[0]
                            self.assertTrue(np.any(raw[np.isfinite(raw)] > self.UPPER_CP))
                            np.testing.assert_array_equal(model.weight.detach().numpy(), raw)

    def test_quantize_keeps_i16_boundaries_and_rejects_invalid_weights(self) -> None:
        # 補間計画の1/8cp単位のi16形式から直接得られる保存境界。
        weights = np.array([self.LOWER_CP, 0.0, self.UPPER_CP], dtype=np.float32)
        np.testing.assert_array_equal(quantize(weights), [-32768, 0, 32767])
        for value in (-4096.125, 4096.0):
            with self.subTest(value=value), self.assertRaises(OverflowError):
                quantize(np.array([value], dtype=np.float32))
        for value in (np.nan, np.inf, -np.inf):
            with self.subTest(value=value), self.assertRaises(ValueError):
                quantize(np.array([value], dtype=np.float32))


if __name__ == "__main__":
    unittest.main()
