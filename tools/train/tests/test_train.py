"""PSTの学習、検証損失、およびコマンドラインの契約を検証する。"""

from __future__ import annotations

import hashlib
import io
import json
import re
import struct
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from fractions import Fraction
from io import StringIO
from pathlib import Path
from unittest.mock import patch

import numpy as np
import torch

import minase_train.data.mnpt as mnpt
import minase_train.data.mnsd as mnsd
import minase_train.pst.model as pst_model
import minase_train.pst.removal as removal
import minase_train.pst.teacher as teacher
from helpers import (
    CPU,
    PIECE_VALUES,
    ProvenanceFixtures,
    constant_base,
    direct_loss,
    records_with,
    write_mnsd,
    write_provenance,
    write_rescore,
)
from minase_train.data.features import FEATURE_COUNT, INITIAL_BOARD, mirror
from minase_train.data.mnpt import float_weights_path, initial_piece_values, read_mnpt, write_mnpt
from minase_train.data.mnsd import Dataset, RECORD_DTYPE, hash64
from minase_train.pst import train
from minase_train.pst.model import expanded_model_weights, make_model
from minase_train.pst.teacher import estimate_k
from minase_train.pst.train import main as train_main, train_epoch, validation_loss


class LambdaOverrideTest(unittest.TestCase):

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / 'selfplay.bin'
        write_mnsd(self.source, seed=11, checksum=b'a' * 32,
                   games=np.repeat(np.arange(1, 81), 3).tolist(),
                   scores=[100, 200, -400] * 80, results=[2, 0, 2] * 80)

    def test_cli_options_reach_dataset_and_reject_invalid_values(self):
        import minase_train.diagnostics.comparison as pst_diagnostics
        from minase_train.pst.train import build_parser, main
        parser = build_parser()
        for mix in (None, '0', '1'):
            option = [] if mix is None else ['--lambda-override', mix]
            arguments = ['estimate-k', '--data', str(self.source), *option]
            self.assertEqual(parser.parse_args(arguments).lambda_override,
                             None if mix is None else float(mix))
            with redirect_stdout(io.StringIO()):
                main(arguments)
            output = self.root / f'diagnostic-{mix}'
            argv = ['pst_diagnostics.py', '--data', str(self.source), '--base', 'base.bin',
                    '--candidate', 'candidate.bin', '--probe', 'probe',
                    '--output-dir', str(output), *option]
            # 外部探査の手前で、CLIから構築されたデータセットを確認する。
            with patch('sys.argv', argv), patch.object(pst_diagnostics, 'diagnose', return_value={}) as diagnose:
                pst_diagnostics.main()
            dataset = diagnose.call_args.args[0]
            self.assertEqual(dataset.lambda_override, None if mix is None else float(mix))
            self.assertEqual(dataset.teacher_lambdas[0], .75 if mix is None else float(mix))
        for value in ('-0.01', '1.01', 'nan', 'inf', '-inf'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                main(['estimate-k', '--data', str(self.source), '--lambda-override=' + value])
            argv = ['pst_diagnostics.py', '--data', str(self.source), '--base', 'base.bin',
                    '--candidate', 'candidate.bin', '--probe', 'probe',
                    '--output-dir', str(self.root / 'invalid'), '--lambda-override=' + value]
            with patch('sys.argv', argv), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                pst_diagnostics.main()
            self.assertNotEqual(error.exception.code, 0)
            self.assertFalse((self.root / 'invalid').exists())


class LookaheadIntegrationTest(unittest.TestCase):

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.path = self.root / 'source.bin'
        candidates = np.arange(1, 100, dtype='u4')
        self.game = int(candidates[hash64(11, candidates) % 20 != 0][0])
        self.rows = np.zeros(4, dtype=RECORD_DTYPE)
        self.rows['game'] = self.game
        self.rows['ply'] = [0, 1, 3, 10]
        self.rows['score'] = [100, 200, -400, 50]
        self.rows['result'] = [2, 0, 0, 2]
        self.rows['lion'] = 255
        for row, count in zip(self.rows, [78, 77, 61, 46]):
            row['board'][:count] = 1
        mnsd.write_mnsd(self.path, self.rows, seed=11, network_checksum=b'a' * 32)
        write_provenance(self.path)
        self.current = Dataset([self.path])
        self.future = Dataset([self.path], lookahead={'gamma': .9, 'plies': 3})
        self.expected = np.array([124 / 1.81, -400, -400, 50])

    def test_validation_loss_uses_real_teacher_values(self):
        model = make_model(torch.ones((FEATURE_COUNT, 2)), torch.device('cpu'), 'tapered')
        loss, _ = validation_loss(model, self.future, np.array([100.]), 100, 2,
                                  torch.device('cpu'), indices=np.arange(4))
        logits = np.array([78, 77, 61, 46]) / 100
        target = (.75 / (1 + np.exp(-self.expected / 100)) + .25 * self.rows['result'] / 2).astype('f4')
        expected = np.mean(np.logaddexp(0, logits) - target * logits)
        self.assertAlmostEqual(loss, expected, places=7)
        epoch = train_epoch(model, torch.optim.SGD(model.parameters(), lr=0), self.future,
                            np.array([100.]), 100, 2, torch.Generator().manual_seed(1),
                            torch.device('cpu'), indices=np.arange(4), removal_penalty=0,
                            removal_reference=None)
        self.assertAlmostEqual(epoch.bce_loss, expected, places=7)


class Phase4Test(ProvenanceFixtures, unittest.TestCase):

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / "source.bin"
        self.sidecar = self.root / "replacement.any"
        self.rows = np.zeros(100, dtype=RECORD_DTYPE)
        self.rows["board"] = INITIAL_BOARD
        self.rows["lion"] = 255
        self.rows["game"] = np.arange(1, 101)
        self.rows["score"] = [-500, 500] * 50
        self.rows["result"] = [0, 2] * 50
        mnsd.write_mnsd(self.source, self.rows, seed=11, network_checksum=b"a" * 32, teacher_nodes=100000)
        self.provenance = write_provenance(self.source)


    def test_cpu_training_cli_records_initial_metrics_and_exclusions(self):
        from minase_train.data.mnpt import initial_piece_values, write_mnpt
        from minase_train.pst.train import main
        self.metadata(**{"lambda": 0})
        rows = [(1, 0, 100, 1, 20)] * 100
        rows[0] = (2, 0, 0, 0, 10)
        write_rescore(self.sidecar, self.source, rows)
        base = self.root / "base.bin"
        output = self.root / "trained.bin"
        weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
        write_mnpt(base, weights, weights, initial_piece_values(), 400.)
        stdout = StringIO()
        with patch('minase_train.pst.teacher.estimate_k', side_effect=AssertionError("unexpected K estimate")), redirect_stdout(stdout):
            main(["train", "--data", str(self.source), "--rescore", str(self.sidecar),
                  "--init", str(base), "--output", str(output), "--model", "single",
                  "--k", "400", "--lr", "0.01", "--epochs", "1", "--batch", "100", "--device", "cpu", "--removal-penalty", "0"])
        report = json.loads(output.with_suffix(".training.json").read_text())
        self.assertEqual(report["teacher_ks"], [None])
        self.assertEqual(report["rescore_exclusions"]["depth_incomplete"], 1)
        self.assertIn('"depth_incomplete": 1', stdout.getvalue())
        self.assertEqual([entry["epoch"] for entry in report["validation"]], [0, 1])
        self.assertAlmostEqual(report["validation"][0]["loss"], np.log(2), places=6)
        self.assertEqual(report["validation"][0]["sign_agreement"]["random-selfplay"]["agreement"], 0)
        for entry in report["validation"]:
            self.assertIsNone(entry["human_game_mean"]["loss"])
            self.assertEqual(set(entry["origins"]), {"random-selfplay", "human-start-selfplay", "human-game"})

    def test_sign_agreement_excludes_draws(self):
        self.rows["result"] = 1
        self.rows["result"][:2] = [0, 2]
        mnsd.write_mnsd(self.source, self.rows, seed=11, network_checksum=b"a" * 32)
        write_provenance(self.source, **{"lambda": 0})
        dataset = Dataset([self.source])
        model = make_model(torch.zeros((FEATURE_COUNT, 1)), torch.device("cpu"), "single")
        groups = {}
        validation_loss(model, dataset, np.array([np.nan]), 1., 10, torch.device("cpu"), indices=np.arange(100), breakdown=groups)
        self.assertEqual(groups["sign_agreement"]["random-selfplay"], {"positions": 2, "agreement": 0.})

    def test_validation_means_and_signs_from_results(self):
        # 棋譜Aは1局面・勝ち、棋譜Bは3局面・負け。局面平均と対局平均を区別する。
        rows = self.rows[:4].copy()
        rows["game"] = [1, 2, 2, 2]
        rows["result"] = [2, 0, 0, 0]
        rows["board"] = 0
        rows["board"][:, 0] = 1
        mnsd.write_mnsd(self.source, rows, seed=11, network_checksum=b"a" * 32)
        write_provenance(self.source, result_origin="human", games=[{"game": 1, "id": "A"}, {"game": 2, "id": "B"}], **{"lambda": 0})
        dataset = Dataset([self.source])
        model = make_model(torch.ones((FEATURE_COUNT, 1)), torch.device("cpu"), "single")
        groups = {}
        loss, classes = validation_loss(model, dataset, np.array([np.nan]), 1., 2, torch.device("cpu"), indices=np.arange(4), breakdown=groups)
        win_loss, loss_loss = np.logaddexp(0, -1), np.logaddexp(0, 1)
        self.assertAlmostEqual(loss, (win_loss + 3 * loss_loss) / 4, places=6)
        self.assertAlmostEqual(groups["human_game_mean"]["loss"], (win_loss + loss_loss) / 2, places=6)
        self.assertEqual(groups["human_game_mean"]["games"], 2)
        self.assertEqual(groups["sign_agreement"]["human-game"]["agreement"], 0.25)
        self.assertIsNone(groups["origins"]["random-selfplay"]["loss"])
        self.assertEqual(sum(v["positions"] for v in groups["piece_bands"].values()), 4)
        self.assertEqual(sum(v["loss"] is None for v in groups["piece_bands"].values()), 4)
        self.assertAlmostEqual(classes[0], loss)


class TrainHalfTest(unittest.TestCase):

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        threads = torch.get_num_threads()
        torch.set_num_threads(1)
        self.addCleanup(torch.set_num_threads, threads)
        self.source = self.root / 'selfplay.bin'
        board = np.zeros((240, 144), dtype='u1')
        board[:, 0] = 1
        board[1::3, 17] = 2
        board[2::3, 29] = 65
        write_mnsd(self.source, seed=11, checksum=b'a' * 32,
                   games=np.repeat(np.arange(1, 81), 3).tolist(),
                   scores=[100, 200, -400] * 80, results=[2, 0, 2] * 80,
                   board=board)
        self.initial = self.root / 'initial.bin'
        zero = np.zeros(FEATURE_COUNT, dtype='<i2')
        mnpt.write_mnpt(self.initial, zero, zero, mnpt.initial_piece_values(), 200)

    def arguments(self, output, half=None):
        return ['train', '--data', str(self.source), '--init', str(self.initial),
                '--output', str(output), '--model', 'mirrored', '--k', '200',
                '--lr', '1', '--epochs', '2', '--batch', '64', '--device', 'cpu',
                '--removal-penalty', '1000', '--lookahead-gamma', '.9',
                '--lookahead-plies', '3',
                *([] if half is None else ['--train-half', str(half)])]

    def test_omission_preserves_pre_change_weight_bytes(self):
        # 同じ合成データを変更前の学習器で実行して得たMNPTのSHA-256。
        output = self.root / 'full.bin'
        with redirect_stdout(io.StringIO()):
            train.main(self.arguments(output))
        report = json.loads(output.with_suffix('.training.json').read_text())
        self.assertGreater(report['best_epoch'], 0)
        self.assertEqual(hashlib.sha256(output.read_bytes()).hexdigest(),
                         'ae9be28f913ed63a2de71668eb52e8e59720d03bcd236dcdd509f68d61b49f50')
        self.assertIsNone(report['train_half'])
        count = Dataset([self.source]).training_indices.size
        self.assertEqual(sum(report['training_half_counts']), count)
        self.assertEqual(report['total_updates'], 2 * ((count + 63) // 64))

    def test_cli_filters_only_training_and_records_all_updates(self):
        reference = Dataset([self.source], lookahead={'gamma': .9, 'plies': 3})
        expected_ks, _ = teacher.estimate_generation_ks(reference, indices=reference.training_indices)
        halves = reference.training_halves()
        # 全体・両半分に加え、乱数シードとバッチ構成を変えても同じ対局を選ぶ。
        for run, (half, seed, batch) in enumerate(((None, 1, 64), (0, 1, 64),
                                                 (1, 1, 64), (0, 19, 37), (1, 19, 37))):
            with self.subTest(half=half, seed=seed, batch=batch):
                output = self.root / f'run-{run}.bin'
                arguments = self.arguments(output, half) + [
                    '--seed', str(seed), '--batch', str(batch), '--lr', '.5', '1']
                selected = reference.training_indices if half is None else halves[half]
                steps = []
                original_step = torch.optim.Adam.step

                def step(optimizer, *args, **kwargs):
                    steps.append(1)
                    return original_step(optimizer, *args, **kwargs)

                with (redirect_stdout(io.StringIO()),
                      patch.object(train, 'train_epoch', wraps=train.train_epoch) as epochs,
                      patch.object(train, 'validation_loss', wraps=train.validation_loss) as validation,
                      patch.object(train, 'estimate_generation_ks', wraps=teacher.estimate_generation_ks) as estimate,
                      patch.object(torch.optim.Adam, 'step', step)):
                    train.main(arguments)
                np.testing.assert_array_equal(estimate.call_args.kwargs['indices'], reference.training_indices)
                for call in epochs.call_args_list:
                    np.testing.assert_array_equal(call.kwargs['indices'], selected)
                    actual_dataset = call.args[2]
                    self.assertEqual(actual_dataset.record_count, reference.record_count)
                    np.testing.assert_array_equal(actual_dataset.training_indices, reference.training_indices)
                    np.testing.assert_array_equal(actual_dataset.lookahead_scores, reference.lookahead_scores)
                    np.testing.assert_array_equal(call.args[3], expected_ks)
                for call in validation.call_args_list:
                    np.testing.assert_array_equal(call.kwargs['indices'], reference.validation_indices)
                report = json.loads(output.with_suffix('.training.json').read_text())
                self.assertEqual(report['teacher_ks'], expected_ks.tolist())
                self.assertEqual(report['train_half'], half)
                self.assertEqual(report['training_half_counts'], [len(indices) for indices in halves])
                self.assertEqual(report['total_updates'], len(steps))
                # 2候補の各1エポックと本学習の2エポック。端数バッチも1更新。
                self.assertEqual(len(steps), 4 * ((len(selected) + batch - 1) // batch))

    def test_cli_rejects_values_other_than_zero_and_one(self):
        parser = train.build_parser()
        output = self.root / 'invalid.bin'
        for value in ('-1', '2', '0.5', 'nan', 'true'):
            with self.subTest(value=value), redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
                parser.parse_args(self.arguments(output) + ['--train-half=' + value])
            self.assertEqual(error.exception.code, 2)
        self.assertFalse(output.exists())

    def test_empty_selected_half_is_rejected_before_training(self):
        dataset = Dataset([self.source])
        train_game = int(dataset.gather(dataset.training_halves()[0][:1])['game'][0])
        validation_game = int(dataset.gather(dataset.validation_indices[:1])['game'][0])
        source = self.root / 'small.bin'
        write_mnsd(source, seed=11, checksum=b'a' * 32,
                   games=sorted([train_game, validation_game]))
        output = self.root / 'empty.bin'
        with patch.object(train, 'train_epoch') as epoch, self.assertRaisesRegex(ValueError, 'empty training'):
            train.main(self.arguments(output, 1) + ['--data', str(source)])
        epoch.assert_not_called()
        self.assertFalse(output.exists())


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
                        "--removal-penalty",
                        "0",
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
                "--seed", "1", "--validation-sample", "100", "--device", "cpu", "--removal-penalty", "0",
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
                "--seed", "1", "--validation-sample", "100", "--device", "cpu", "--removal-penalty", "0",
            ]
            with patch('minase_train.pst.train.mirror', side_effect=AssertionError("mirrored must not augment data")):
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
            dataset = Dataset([data])
            with patch('minase_train.pst.train.mirror', wraps=mirror) as augment:
                train_epoch(
                    model, torch.optim.SGD(model.parameters(), lr=0.1), dataset,
                    np.array([200.0]), 200.0, 64,
                    torch.Generator().manual_seed(1), device,
                    indices=dataset.training_indices, removal_penalty=0.0, removal_reference=None,
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
            1072.6529541015625, 1,
            torch.Generator().manual_seed(1), torch.device("cpu"),
            indices=dataset.training_indices, removal_penalty=0.0, removal_reference=None,
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


class RemovalApiTest(unittest.TestCase):

    def test_cli_requires_explicit_finite_nonnegative_model_appropriate_coefficient(self):
        arguments = ["train", "--data", "not-read.bin", "--init", "not-read.mnpt", "--output", "not-written.mnpt",
                     "--model", "mirrored", "--k", "2", "--lr", ".1", "--device", "cpu"]
        with redirect_stderr(StringIO()), self.assertRaises(SystemExit):
            train.main(arguments)
        for value in ("-1", "nan", "inf", "-inf"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                train.main(arguments + ["--removal-penalty=" + value])
        for kind in ("single", "tapered"):
            bad = arguments.copy()
            bad[bad.index("--model") + 1] = kind
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                train.main(bad + ["--removal-penalty", "1"])

    def test_positive_cli_rejects_asymmetric_initial_file_before_training(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, initial, output = root / "data.bin", root / "initial.bin", root / "output.bin"
            write_mnsd(data, seed=5, checksum=b"x" * 32, games=list(range(64)))
            base = constant_base()
            base[0, 1] = 1
            mnpt.write_mnpt(initial, base[:, 0], base[:, 1], PIECE_VALUES, 2)
            with patch('minase_train.pst.train.train_epoch', side_effect=AssertionError("invalid init reached training")), redirect_stdout(StringIO()):
                with self.assertRaises(ValueError):
                    train.main(["train", "--data", str(data), "--init", str(initial), "--output", str(output),
                                "--model", "mirrored", "--k", "2", "--lr", ".1", "--epochs", "1",
                                "--device", "cpu", "--removal-penalty", "1"])
            self.assertFalse(output.exists())

    def test_validation_is_pure_bce_on_explicit_calibration_indices(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "data.bin"
            board = records_with([[(0, 1), (142, 12), (143, 76)]])[0]["board"]
            write_mnsd(path, seed=5, checksum=b"v" * 32, games=list(range(4)),
                       scores=[-100, 300, 400, 200], results=[0, 1, 1, 2], board=board)
            write_provenance(path, **{"lambda": 0.0})
            dataset = Dataset([path])
            base = constant_base()
            model = pst_model.make_model(torch.from_numpy(-base.astype(np.float32) / 8), CPU, "mirrored")
            calibration = np.array([3, 0], dtype=np.int64)
            # Both logits are -.5; targets are 1 and 0. No removal term enters validation.
            expected = float(np.logaddexp(0, -.5) + .25)
            with patch('minase_train.pst.train.removal_loss', side_effect=AssertionError("validation must not add removal loss")):
                overall, by_generation = train.validation_loss(
                    model, dataset, np.array([2.0]), 2.0, 1, CPU, indices=calibration,
                )
            self.assertAlmostEqual(overall, expected, delta=1e-7)
            np.testing.assert_allclose(by_generation, [expected], atol=1e-7, rtol=0)

    def test_positive_coefficient_changes_real_cpu_update_and_retains_projection(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "data.bin"
            records = records_with([[(0, 1), (142, 12), (143, 76)]])
            write_mnsd(path, seed=5, checksum=b"u" * 32, games=[0, 1],
                       scores=[0, 32767], results=[1, 2], board=records[0]["board"])
            write_provenance(path, **{"lambda": 0.0})
            dataset = Dataset([path])
            base = constant_base()
            candidate = -base.astype(np.float32) / 8
            expected_r = float(Fraction(5, 8))
            expected_bce = float(np.logaddexp(0, -.5) + .25)
            updated = {}
            for rho, step_size in [(0.0, .1), (2.0, .1), (2.0, 1e8)]:
                with self.subTest(rho=rho, step_size=step_size):
                    model = pst_model.make_model(torch.from_numpy(candidate), CPU, "mirrored")
                    reference = removal.make_removal_reference(base[:, 0], base[:, 1], CPU) if rho else None
                    result = train.train_epoch(
                        model, torch.optim.SGD(model.parameters(), lr=step_size), dataset,
                        np.array([2.0]), 2.0, 1, torch.Generator().manual_seed(7), CPU,
                        count_features=True, indices=np.array([0]), removal_penalty=rho, removal_reference=reference,
                    )
                    expanded = pst_model.expanded_model_weights(model).detach().numpy()
                    if step_size == .1:
                        updated[rho] = expanded
                    self.assertAlmostEqual(result.bce_loss, expected_bce, delta=1e-7)
                    self.assertAlmostEqual(result.total_loss, expected_bce + rho * expected_r, delta=1e-7)
                    self.assertEqual(int(result.observations.sum()), 3)
                    if rho:
                        self.assertAlmostEqual(result.removal_loss, expected_r, delta=1e-7)
                    self.assertTrue(np.all(np.isfinite(expanded)))
                    self.assertTrue(np.all(expanded >= -4096))
                    self.assertTrue(np.all(expanded <= 4095.875))
                    np.testing.assert_array_equal(model.weight[-1].detach().numpy(), 0)
                    if step_size == .1:
                        self.assertLess(direct_loss(records, base, expanded, 2), direct_loss(records, base, candidate, 2))
                    else:
                        self.assertTrue(np.any(expanded == 4095.875) or np.any(expanded == -4096))
            self.assertGreater(updated[2.0][29 * 144, 1], updated[0.0][29 * 144, 1])

    def test_best_epoch_uses_bce_instead_of_training_total_or_removal_loss(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data, initial, output = root / "data.bin", root / "initial.bin", root / "trained.bin"
            write_mnsd(data, seed=5, checksum=b"b" * 32, games=list(range(64)))
            base = constant_base()
            mnpt.write_mnpt(initial, base[:, 0], base[:, 1], PIECE_VALUES, 2.0)
            calls = 0

            def epoch(model, *_args, **_kwargs):
                nonlocal calls
                calls += 1
                with torch.no_grad():
                    model.weight[:-1].fill_(calls + 1)
                return train.TrainEpochResult(.1 / calls, 500.0 if calls == 1 else 0.0,
                                              500.1 if calls == 1 else .05, None)

            losses = [(v, np.array([v])) for v in [.7, .6, .65]]
            log = StringIO()
            with patch('minase_train.pst.train.train_epoch', side_effect=epoch), patch('minase_train.pst.train.validation_loss', side_effect=losses), redirect_stdout(log):
                train.main(["train", "--data", str(data), "--init", str(initial), "--output", str(output),
                            "--model", "mirrored", "--k", "2", "--lr", ".1", "--epochs", "2", "--batch", "64",
                            "--device", "cpu", "--removal-penalty", "1"])
            self.assertIn("best epoch: 1 validation_loss=0.600000000", log.getvalue())
            mg, eg, _, _ = mnpt.read_mnpt(output)
            np.testing.assert_array_equal(mg, 16)
            np.testing.assert_array_equal(eg, 16)


if __name__ == "__main__":
    unittest.main()
