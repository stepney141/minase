"""追加特徴の手計算例と独立したprobe応答で、照合の拒否条件を固定する。"""

import hashlib
from pathlib import Path
import tempfile
import unittest

import numpy as np

from features import FEATURE_COUNT
from mnsd import Dataset, HEADER, RECORD_DTYPE, map_records, write_mnsd
from pst_diagnostics import diagnose
from train_pst import initial_piece_values, read_mnpt, write_mnpt


class ExtraDiagnosticsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.base = self.root / 'base.bin'
        self.candidate = self.root / 'candidate.bin'
        self.floating = self.root / 'candidate-float.npz'
        zeros = np.zeros(FEATURE_COUNT, dtype=np.int16)
        write_mnpt(self.base, zeros, zeros, initial_piece_values(), 1000)
        # 人工特徴を「先手の未成歩の枚数」とし、端点を2cpと1cpにする。
        write_mnpt(self.candidate, np.append(zeros, np.int16(16)),
                   np.append(zeros, np.int16(8)), initial_piece_values(), 1000,
                   feature_count=FEATURE_COUNT + 1)
        self.write_float(0)
        records = np.zeros(200, dtype=RECORD_DTYPE)
        records['lion'] = 255
        records['game'] = np.arange(200)
        records['board'][:, :2] = [12, 76]
        records['board'][:, 2:47] = 1
        records['score'] = 100
        records['result'] = 2
        data = self.root / 'data.bin'
        write_mnsd(data, records, seed=0, network_checksum=bytes(32))
        features = self.root / 'data.mnkf'
        values = np.zeros((200, 68), dtype=np.uint8)
        values[:, 62] = 45
        features.write_bytes(HEADER.pack(b'MNKF', 1, 1, 68, 200,
                                         hashlib.sha256(data.read_bytes()).digest()) + values.tobytes())
        self.dataset = Dataset([data], king_features=[features], extra_columns=[62])

    def write_float(self, drift):
        zeros = np.zeros(FEATURE_COUNT, dtype=np.float32)
        np.savez(self.floating, middlegame=np.append(zeros, np.float32(2 + drift)),
                 endgame=np.append(zeros, np.float32(1 + drift)))

    def probe(self, weights, data, promotions):
        mg, _, _, _ = read_mnpt(weights, feature_count=None)
        active = bool(mg[-1])
        def evaluation(record):
            board = record['board']
            pawns = int(np.count_nonzero(board == 1))
            q = min(90, max(0, int(np.count_nonzero(board)) - 2))
            # 序中盤2cp、終盤1cpなので、1枚あたり1+q/90 cpになる。
            value = int(pawns * (1 + q / 90)) if active else 0
            return {'eval': value, 'pst_eval': 0, 'king_features': [pawns]}
        rows = []
        for index, record in enumerate(map_records(data)):
            row = {'index': index, **evaluation(record)}
            if data.name == 'removals.bin' and index == 0:
                row = {'index': index, 'skipped': 'synthetic restoration failure'}
            if promotions:
                after = np.zeros((), dtype=RECORD_DTYPE)
                after['board'][:3] = [12, 76, 1]
                after['stm'] = 1 - record['stm']
                after['lion'] = 255
                result = evaluation(after)
                row['promotions'] = [{'move': '1a1b+', 'delta': -result['eval'] - row['eval'],
                                      'after': {'board': after['board'].tolist(),
                                                'stm': int(after['stm']), 'lion': 255, **result}}]
            rows.append(row)
        return rows

    def run_diagnose(self, output, probe=None):
        output.mkdir()
        return diagnose(self.dataset, self.base, self.candidate, self.floating,
                        output, 10, 1, .75, self.probe if probe is None else probe)

    def test_extra_evaluation_promotions_removals_and_explicit_base_conversion(self):
        output = self.root / 'ok'
        report = self.run_diagnose(output)
        self.assertGreater(report['rust_agreement']['candidate'], 0)
        self.assertGreater(report['rust_promotion_agreement']['candidate'], 0)
        self.assertEqual(report['rust_removal_agreement']['candidate']['skipped'], 1)
        self.assertGreater(report['rust_removal_agreement']['candidate']['checked'], 0)
        # 47枚ではq=45、45枚の歩×1.5cp=67.5cpを切り捨てて67cp。
        representative = report['representatives'][1]
        self.assertEqual(representative['total_evaluations']['candidate'], 67)
        self.assertEqual(representative['evaluations']['candidate'], 0)
        self.assertEqual(representative['promotions']['candidate'][0]['delta'], -68)
        self.assertTrue((output / 'diagnostic-base.bin').is_file())
        self.assertTrue(any(item['total_delta_cp']['candidate'] != item['delta_cp']['candidate']
                            for item in representative['removals']))

    def test_changed_probe_fields_are_rejected_independently(self):
        for mutation in ('original', 'columns', 'after', 'delta', 'removal'):
            def wrong(weights, data, promotions):
                rows = self.probe(weights, data, promotions)
                if weights == self.candidate:
                    if mutation == 'original' and data.name == 'diagnostic-samples.bin':
                        rows[0]['eval'] += 1
                    if mutation == 'columns' and data.name == 'diagnostic-samples.bin':
                        rows[0]['king_features'].append(0)
                    if mutation == 'after' and promotions:
                        rows[0]['promotions'][0]['after']['eval'] += 1
                    if mutation == 'delta' and promotions:
                        rows[0]['promotions'][0]['delta'] += 1
                    if mutation == 'removal' and data.name == 'removals.bin':
                        rows[1]['eval'] += 1
                return rows
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.run_diagnose(self.root / mutation, wrong)

    def test_extra_float_drift_is_included_in_quantization_gate(self):
        self.write_float(3)
        with self.assertRaisesRegex(ValueError, 'quantization'):
            self.run_diagnose(self.root / 'drift')

    def test_shelter_candidate_diagnostics_accept_both_mnkf_widths(self):
        # 同じ24特徴を68列から選んでも24列から読んでも、候補診断の整数照合は一致する。
        mg = np.zeros(13680 + 24, dtype=np.int16)
        eg = mg.copy()
        mg[-1], eg[-1] = 16, 8
        write_mnpt(self.candidate, mg, eg, initial_piece_values(), 1000, feature_count=len(mg))
        np.savez(self.floating, middlegame=mg.astype(np.float32) / 8,
                 endgame=eg.astype(np.float32) / 8)

        def probe(weights, data, promotions):
            rows = self.probe(weights, data, promotions)
            for row in rows:
                if 'skipped' in row:
                    continue
                row['king_features'] = [0] * 23 + row['king_features']
                for promotion in row.get('promotions', []):
                    after = promotion['after']
                    after['king_features'] = [0] * 23 + after['king_features']
            return rows

        reports = []
        data = self.root / 'data.bin'
        features = self.root / 'shelter.mnkf'
        for width in (68, 24):
            values = np.zeros((200, width), dtype=np.uint8)
            values[:, 23] = 45
            features.write_bytes(HEADER.pack(b'MNKF', 1, 1, width, 200,
                                             hashlib.sha256(data.read_bytes()).digest()) + values.tobytes())
            self.dataset = Dataset([data], king_features=[features], extra_columns=range(24))
            reports.append(self.run_diagnose(self.root / f'shelter-{width}', probe))
        for key in ('rust_agreement', 'rust_promotion_agreement', 'rust_removal_agreement', 'quantization'):
            self.assertEqual(reports[0][key], reports[1][key])
        self.assertGreater(reports[0]['rust_agreement']['candidate'], 0)


if __name__ == '__main__':
    unittest.main()
