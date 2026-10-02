"""世代3計画の半分割契約を、対局の同一性と学習出力から検証する。"""

from contextlib import redirect_stderr, redirect_stdout
import hashlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

from features import FEATURE_COUNT
from mnsd import Dataset, hash64
from test_phase4 import write_rescore
from test_train_pst import write_mnsd, write_provenance
import train_pst as train


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
        train.write_mnpt(self.initial, zero, zero, train.initial_piece_values(), 200)

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

    def assert_partition(self, dataset):
        halves = dataset.training_halves()
        self.assertTrue(all(indices.size for indices in halves))
        self.assertEqual(np.intersect1d(*halves).size, 0)
        np.testing.assert_array_equal(np.sort(np.concatenate(halves)), dataset.training_indices)
        keys = [set(dataset.game_keys(indices)) for indices in halves]
        self.assertTrue(keys[0].isdisjoint(keys[1]))
        for indices, group in zip(halves, keys):
            self.assertEqual(np.intersect1d(indices, dataset.validation_indices).size, 0)
            self.assertTrue(group.isdisjoint(dataset.game_keys(dataset.validation_indices)))
            self.assertTrue(np.all(np.diff(indices) > 0))
        return keys

    def test_partition_and_identity_survive_file_order_and_record_order(self):
        other = self.root / 'other.bin'
        write_mnsd(other, seed=2**64 - 1, checksum=b'b' * 32,
                   games=np.repeat(np.arange(80), 2).tolist())
        human_paths = []
        for file in range(2):
            path = self.root / f'human-{file}.bin'
            # 同じ棋譜IDに別のファイルシード・ローカル対局番号を与える。
            numbers = np.arange(1, 81) + file * 100
            if file:
                numbers = numbers[::-1]
            write_mnsd(path, seed=20 + file, checksum=b'c' * 32,
                       games=np.repeat(numbers, 2).tolist())
            write_provenance(path, result_origin='human' if file == 0 else 'selfplay',
                             start_origin='human-game', **{
                'lambda': 0 if file == 0 else .75,
                'games': [{'game': int(n), 'id': f'棋譜-{n - file * 100}', 'ply': 0}
                          for n in numbers]})
            human_paths.append(path)
        paths = [self.source, other, *human_paths]
        expected = self.assert_partition(Dataset(paths))
        self.assertEqual(expected, self.assert_partition(Dataset(paths[::-1])))
        first = self.assert_partition(Dataset([human_paths[0]]))
        self.assertEqual(first, self.assert_partition(Dataset([human_paths[1]])))
        # 検証ハッシュの下位ビットを群番号に流用していないことも確かめる。
        dataset = Dataset([self.source])
        sizes = [len(indices) for indices in dataset.training_halves()]
        self.assertTrue(all(.3 < size / sum(sizes) < .7 for size in sizes))
        for indices in dataset.training_halves():
            validation_bits = hash64(11, dataset.gather(indices)['game']) % 2
            self.assertEqual(set(validation_bits), {0, 1})

    def test_rescore_exclusions_remain_outside_both_halves(self):
        sidecar = self.root / 'rescore.bin'
        write_rescore(sidecar, self.source,
                      [(1, 1, 120, 1, 100), (2, 0, 0, 0, 100), (0, 0, 0, 0, 0)] * 80)
        dataset = Dataset([self.source], rescore=[sidecar])
        self.assert_partition(dataset)
        for indices in dataset.training_halves():
            self.assertTrue(np.all(indices % 3 == 2))

    def test_cli_filters_only_training_and_records_all_updates(self):
        reference = Dataset([self.source], lookahead={'gamma': .9, 'plies': 3})
        expected_ks, _ = train.estimate_generation_ks(reference, indices=reference.training_indices)
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
                      patch.object(train, 'estimate_generation_ks', wraps=train.estimate_generation_ks) as estimate,
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


if __name__ == '__main__':
    unittest.main()
