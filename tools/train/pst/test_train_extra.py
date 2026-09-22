"""段階9の固定範囲、列対応、評価式を指示書と手計算から検証する。"""

from contextlib import redirect_stdout
from io import StringIO
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

from features import FEATURE_COUNT, PADDING_INDEX, feature_indices
from test_train_pst import write_provenance
from mnsd import HEADER, Dataset, RECORD_DTYPE, write_mnsd
from train_pst import (
    expanded_model_weights, float_evaluate, initial_piece_values, integer_evaluate,
    main, make_model, model_logits, parse_ranges, quantize, read_mnpt, train_epoch,
    write_mnpt,
)


class ExtraTrainingTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.data = self.root / 'data.bin'
        self.mnkf = self.root / 'features.bin'
        self.initial = self.root / 'initial.bin'
        self.records = np.zeros(200, dtype=RECORD_DTYPE)
        self.records['board'][:, :47] = 1
        self.records['board'][:, :2] = [12, 76]
        self.records['board'][::2, 2] = 51
        self.records['lion'] = 255
        self.records['game'] = np.arange(200)
        self.records['score'] = 1000
        self.records['result'] = 2
        write_mnsd(self.data, self.records, seed=0, network_checksum=bytes(32))
        write_provenance(self.data)
        self.values = np.tile(np.arange(68, dtype=np.uint8), (200, 1)) % 3 + 1
        self.write_features()
        self.device = torch.device('cpu')

    def write_features(self):
        self.mnkf.write_bytes(HEADER.pack(
            b'MNKF', 1, 1, self.values.shape[1], len(self.records), hashlib.sha256(self.data.read_bytes()).digest()
        ) + self.values.tobytes())

    def dataset(self, columns=(0, 1, 62)):
        return Dataset([self.data], king_features=[self.mnkf], extra_columns=columns)

    def arguments(self, model='mirrored'):
        return ['train', '--data', str(self.data), '--king-features', str(self.mnkf),
                '--extra-columns', '0:2,62:63', '--train-extra', '1:2', '--freeze-pst',
                '--init', str(self.initial), '--output', str(self.root / 'output.bin'),
                '--model', model, '--k', '1000', '--removal-penalty', '0', '--device', 'cpu',
                '--epochs', '1', '--batch', '64', '--lr', '1']

    def test_zero_extras_preserve_logits_for_every_model(self):
        records = self.records[:3]
        features = torch.as_tensor(feature_indices(records['board'], records['stm'], records['lion']))
        phi = torch.tensor([0.0, 0.5, 1.0])
        for kind in ('single', 'tapered', 'mirrored'):
            with self.subTest(kind=kind):
                initial = torch.ones((FEATURE_COUNT, 1 if kind == 'single' else 2))
                pst = make_model(initial, self.device, kind)
                extended = make_model(initial, self.device, kind, extra_initial=torch.zeros((30, 2)))
                torch.testing.assert_close(
                    model_logits(pst, features, phi, 1000),
                    model_logits(extended, features, phi, 1000, torch.ones((3, 30))),
                    rtol=0, atol=0,
                )

    def test_single_pst_keeps_two_independent_extra_endpoints(self):
        model = make_model(torch.zeros((FEATURE_COUNT, 1)), self.device, 'single',
                           extra_initial=torch.tensor([[10., 30.]]))
        values = model_logits(model, torch.full((3, 145), PADDING_INDEX),
                              torch.tensor([0., 0.5, 1.]), 10., torch.ones((3, 1)))
        torch.testing.assert_close(values, torch.tensor([3., 2., 1.]))

    def test_frozen_export_retains_pst_old_extras_piece_values_and_k(self):
        for kind in ('single', 'tapered', 'mirrored'):
            with self.subTest(kind=kind):
                mg = np.full(FEATURE_COUNT + 2, 1, dtype=np.int16)
                eg = mg.copy()
                mg[-2:] = [8, -8]
                eg[-2:] = [16, -16]
                write_mnpt(self.initial, mg, eg, initial_piece_values(), 1000, feature_count=len(mg))
                args = self.arguments(kind)
                output = self.root / f'{kind}.bin'
                args[args.index('--output') + 1] = str(output)
                with redirect_stdout(StringIO()):
                    main(args)
                out_mg, out_eg, values, k = read_mnpt(output, feature_count=None)
                self.assertEqual(len(out_mg), FEATURE_COUNT + 3)
                for saved, baseline in ((out_mg, mg), (out_eg, eg)):
                    self.assertEqual(saved[:FEATURE_COUNT + 1].tobytes(), baseline[:FEATURE_COUNT + 1].tobytes())
                    self.assertNotEqual(saved[FEATURE_COUNT + 1], baseline[FEATURE_COUNT + 1])
                    self.assertEqual(saved[-1], 0)
                self.assertEqual(values.tobytes(), initial_piece_values().tobytes())
                self.assertEqual(k, 1000)
                with np.load(output.with_name(output.stem + '-float.npz')) as floating:
                    np.testing.assert_array_equal(quantize(floating['middlegame']), out_mg)
                    np.testing.assert_array_equal(quantize(floating['endgame']), out_eg)
                history = json.loads(output.with_suffix('.training.json').read_text())['validation']
                self.assertEqual([entry['epoch'] for entry in history], [0, 1])
                self.assertLess(history[1]['loss'], history[0]['loss'])
                for entry in history:
                    self.assertGreater(entry['two_royals']['positions'], 0)
                    self.assertGreater(entry['other_royals']['positions'], 0)
                    self.assertGreater(entry['game_half']['positions'], 0)

    def test_noncontiguous_columns_keep_requested_order_across_files(self):
        other = self.root / 'other.bin'
        other_features = self.root / 'other.mnkf'
        write_mnsd(other, self.records[:2], seed=5, network_checksum=b'a' * 32)
        write_provenance(other)
        other_values = np.full((2, 68), 7, dtype=np.uint8)
        other_features.write_bytes(HEADER.pack(
            b'MNKF', 1, 1, 68, 2, hashlib.sha256(other.read_bytes()).digest()
        ) + other_values.tobytes())
        columns = parse_ranges('62:64,0:2', 68)
        self.assertEqual(columns, [62, 63, 0, 1])
        dataset = Dataset([self.data, other], king_features=[self.mnkf, other_features], extra_columns=columns)
        expected = np.array([[7, 7, 7, 7], [3, 1, 1, 2], [7, 7, 7, 7], [3, 1, 1, 2]])
        np.testing.assert_array_equal(dataset.gather_extra(np.array([201, 0, 200, 1])), expected)

    def test_range_rejections(self):
        for value in ('', '0:0', '-1:3', '0:69', '0:2,1:3', '0', '1:2:3', '2:1'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                parse_ranges(value, 68)

    def test_shelter_training_matches_for_full_and_selected_mnkf_files(self):
        # 項目1は定義ID 1の列0〜23。同じ値なら68列から選んでも24列でも学習結果は同じ。
        zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
        write_mnpt(self.initial, zeros, zeros, initial_piece_values(), 1000)
        exported = []
        for width in (68, 24):
            with self.subTest(width=width):
                self.values = self.values[:, :width]
                self.write_features()
                args = self.arguments()
                output = self.root / f'output-{width}.bin'
                args[args.index('--output') + 1] = str(output)
                args[args.index('--extra-columns') + 1] = '0:24'
                args[args.index('--train-extra') + 1] = '0:24'
                with redirect_stdout(StringIO()):
                    main(args)
                exported.append(output.read_bytes())
                mg, eg, values, k = read_mnpt(output, feature_count=13680 + 24)
                np.testing.assert_array_equal(mg[:13680], zeros)
                np.testing.assert_array_equal(eg[:13680], zeros)
                self.assertTrue(np.any(mg[13680:] != 0))
                self.assertEqual(values.tobytes(), initial_piece_values().tobytes())
                self.assertEqual(k, 1000)
        self.assertEqual(exported[0], exported[1])

    def test_selected_columns_must_exist_in_every_mnkf_file(self):
        # ファイル境界を越えて68列と24列を同じ列0〜23で読む。
        other = self.root / 'shelter.bin'
        other_features = self.root / 'shelter.mnkf'
        write_mnsd(other, self.records[:2], seed=5, network_checksum=b'a' * 32)
        write_provenance(other)
        rows = np.full((2, 24), 7, dtype=np.uint8)
        other_features.write_bytes(HEADER.pack(
            b'MNKF', 1, 1, 24, 2, hashlib.sha256(other.read_bytes()).digest()
        ) + rows.tobytes())
        dataset = Dataset([self.data, other], king_features=[self.mnkf, other_features],
                          extra_columns=range(24))
        np.testing.assert_array_equal(dataset.gather_extra(np.array([201, 0, 200, 1])),
                                      np.vstack((rows[1], self.values[0, :24], rows[0], self.values[1, :24])))
        for columns in (range(25), [24], [62]):
            with self.subTest(columns=columns), self.assertRaisesRegex(ValueError, 'selected columns'):
                Dataset([self.data, other], king_features=[self.mnkf, other_features],
                        extra_columns=columns)

    def test_frozen_mirrored_model_rejects_averaging_baseline(self):
        initial = torch.zeros((FEATURE_COUNT, 2))
        initial[0] = 1
        with self.assertRaisesRegex(ValueError, 'exactly mirrored'):
            make_model(initial, self.device, 'mirrored', freeze_pst=True,
                       extra_initial=torch.zeros((3, 2)))

    def test_frozen_cli_rejects_penalty_scale_and_oversized_baseline(self):
        zeros = np.zeros(FEATURE_COUNT + 4, dtype=np.int16)
        write_mnpt(self.initial, zeros, zeros, initial_piece_values(), 1000, feature_count=len(zeros))
        with redirect_stdout(StringIO()), self.assertRaisesRegex(ValueError, 'more additional'):
            main(self.arguments())
        write_mnpt(self.initial, zeros[:FEATURE_COUNT], zeros[:FEATURE_COUNT], initial_piece_values(), 1000)
        for option, value, message in (('--k', '999', 'equal'), ('--removal-penalty', '1', 'freeze-pst')):
            args = self.arguments()
            args[args.index(option) + 1] = value
            with redirect_stdout(StringIO()), self.assertRaisesRegex(ValueError, message):
                main(args)

    def test_save_check_rejects_accidental_frozen_weight_mutation(self):
        zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
        write_mnpt(self.initial, zeros, zeros, initial_piece_values(), 1000)
        original = expanded_model_weights
        def corrupt(model):
            result = original(model).clone()
            result[0] += 1
            return result
        with patch('train_pst.expanded_model_weights', side_effect=corrupt), \
                redirect_stdout(StringIO()), self.assertRaisesRegex(ValueError, 'frozen weights changed'):
            main(self.arguments())
        self.assertFalse((self.root / 'output.bin').exists())

    def test_extra_projection_and_frozen_gradients(self):
        dataset = self.dataset()
        model = make_model(torch.zeros((FEATURE_COUNT, 2)), self.device, 'mirrored',
                           extra_initial=torch.tensor([[7., -3.], [0., 0.], [0., 0.]]),
                           train_extra=[1], freeze_pst=True)
        optimizer = torch.optim.SGD(model.parameters(), lr=1e9)
        train_epoch(model, optimizer, dataset, np.array([1000.]), 1000., 200,
                    torch.Generator().manual_seed(1), self.device, indices=dataset.training_indices,
                    removal_penalty=0, removal_reference=None)
        self.assertIsNone(model.pst.weight.grad)
        torch.testing.assert_close(model.extra.detach()[0], torch.tensor([7., -3.]), rtol=0, atol=0)
        torch.testing.assert_close(model.extra.detach()[1], torch.tensor([4095.875, 4095.875]), rtol=0, atol=0)
        torch.testing.assert_close(model.extra.grad[[0, 2]], torch.zeros((2, 2)), rtol=0, atol=0)

    def test_extra_integer_formula_blends_once_then_truncates_and_clips(self):
        # PST=1/8 cp、追加特徴値2、追加重みmg=3/8, eg=-9/8。
        # q=30なら (30*7+60*(-17))/720 = -1.125 を0へ切り捨てて-1。
        mg = np.zeros(FEATURE_COUNT + 1, dtype=np.int16)
        eg = mg.copy()
        mg[0] = eg[0] = 1
        mg[-1], eg[-1] = 3, -9
        features = np.full((3, 145), PADDING_INDEX, dtype=np.int32)
        features[:, 0] = 0
        extra = np.full((3, 1), 2)
        np.testing.assert_array_equal(integer_evaluate(mg, eg, features, np.array([30, 90, 0]), extra), [-1, 0, -2])
        np.testing.assert_allclose(float_evaluate(mg / 8., eg / 8., features, np.array([1/3, 1., 0.]), extra),
                                   [-1.125, .875, -2.125], atol=1e-6)
        mg[-1] = eg[-1] = -32768
        np.testing.assert_array_equal(integer_evaluate(mg, eg, features, np.array([30, 90, 0]), extra * 16), [-28999] * 3)
        for invalid in (None, np.ones((3, 2))):
            with self.assertRaises(ValueError):
                integer_evaluate(mg, eg, features, np.array([30, 90, 0]), invalid)


if __name__ == '__main__':
    unittest.main()
