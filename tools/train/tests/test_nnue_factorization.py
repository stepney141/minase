"""フェーズ3の特徴分解、初期値、観測規則、学習記録の契約をCPUで検証する。"""

from contextlib import redirect_stdout, redirect_stderr
from io import StringIO
import json
from pathlib import Path
import struct
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import numpy as np
import torch

from helpers import records_with
from minase_train.data import mnpt
from minase_train.data.features import FEATURE_COUNT, PIECE_STATE_BY_BYTE, ROYAL_STATES, mirror
from minase_train.data.mnsd import Dataset
from minase_train.data.taper import phase_ratios, piece_counts
from minase_train.nnue import train
from minase_train.nnue.feature_sets import get_feature_set
from minase_train.nnue.format import Network, read_mnue
from minase_train.nnue.model import NNUE, float_evaluate
from minase_train.nnue.reference import integer_evaluate
from test_train_nnue import arguments, fixture


CPU = torch.device("cpu")
P = get_feature_set("P")
HALFKP = get_feature_set("HalfKP-hm-b8")


def inputs(records, spec, dtype=torch.float32):
    pairs = [tuple(torch.from_numpy(indices) for indices in
                   spec.indices(records["board"], records["lion"], side))
             for side in (records["stm"], 1 - records["stm"])]
    return *pairs, torch.tensor(phase_ratios(records["board"]), dtype=dtype)


def integer_network(model):
    linear, weights, biases, output, bias = train.quantized_weights(model.floating_weights())
    return Network(model.spec.name, linear, mnpt.initial_piece_values(), model.k,
                   weights, biases, output, bias)


class FactorizationTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.threads = torch.get_num_threads()
        torch.set_num_threads(1)

    @classmethod
    def tearDownClass(cls):
        torch.set_num_threads(cls.threads)

    def setUp(self):
        self.initial = np.tile(mnpt.initial_weights().astype(np.float32)[:, None] / 8, (1, 2))
        self.base_observed = np.ones(FEATURE_COUNT, dtype=bool)

    def make_model(self, spec=HALFKP, observed=None):
        if observed is None:
            observed = np.ones(spec.feature_count, dtype=bool)
        return NNUE(self.initial, self.base_observed, 4, 1000, CPU, 13, spec, observed)

    def test_epoch_zero_matches_p_including_missing_and_multiple_royals(self):
        # 仮想行の固定と初期化は観測マスクが疎でも成立する。
        self.base_observed[::24] = False
        self.base_observed[11::24] = False
        p = self.make_model(P, self.base_observed)
        halfkp = self.make_model(observed=np.zeros(HALFKP.feature_count, dtype=bool))
        torch.testing.assert_close(halfkp.ft.virtual.weight, p.ft.expanded_weights(), atol=0, rtol=0)
        self.assertTrue(torch.all(halfkp.ft.embedding.weight == 0))
        rng = np.random.default_rng(175)
        boards = rng.choice(np.array([0, 0, 1, 65, 47, 111], dtype=np.uint8), (144, 144))
        for square, row in enumerate(boards):
            row[square] = 12
            row[143 - square] = 76
        records = records_with([[] for _ in boards])
        records["board"] = boards
        records["stm"] = np.arange(len(records)) % 2
        records["lion"] = np.arange(len(records))
        special = records_with([[], [(50, 1)], [(5, 22), (138, 86), (50, 1)],
                                [(0, 12), (11, 12), (132, 76), (143, 76), (50, 1)],
                                [(0, 22), (11, 22), (132, 86), (143, 86), (50, 1)],
                                [(0, 12), (5, 22), (138, 76), (143, 86), (50, 1)]])
        records = np.concatenate((records, special))
        flipped = records.copy()
        flipped["board"], flipped["lion"] = mirror(records["board"], records["lion"])
        records = np.concatenate((records, flipped))
        with torch.no_grad():
            torch.testing.assert_close(p(*inputs(records, P)), halfkp(*inputs(records, HALFKP)), atol=0, rtol=0)
        values = records["board"], records["stm"], records["lion"]
        np.testing.assert_array_equal(float_evaluate(p.floating_weights(), p.k, *values, P),
                                      float_evaluate(halfkp.floating_weights(), halfkp.k, *values, HALFKP))
        np.testing.assert_array_equal(integer_evaluate(integer_network(p), *values),
                                      integer_evaluate(integer_network(halfkp), *values))

    def gradient_fixture(self):
        # 同じ仮想行へ写る3区分を、両視点・異なる頻度で発火させる。
        features = np.full((3, 145), HALFKP.feature_count, dtype=np.int32)
        for i, bucket in enumerate((0, 2, 7)):
            features[i, 50] = bucket * FEATURE_COUNT + 4226
        opposite = features[[1, 1, 2]].copy()
        observed = np.zeros(HALFKP.feature_count, dtype=bool)
        observed[features[:, 50]] = True
        model = self.make_model(observed=observed).double()
        with torch.no_grad():
            model.ft.virtual.weight[4226] = 0.2
            model.ft.embedding.weight[features[:, 50]] = 0.1
            model.head.biases.fill_(0.05)
            model.head.output_weights.copy_(torch.tensor([[0.2, -0.3, 0.4, 0.1], [-0.1, 0.2, 0.3, 0.4]]))
        base = torch.full((3, 145), FEATURE_COUNT, dtype=torch.int32)
        base[:, 50] = 4226
        batch = (base, torch.from_numpy(features)), (base, torch.from_numpy(opposite)), torch.zeros(3).double()
        return model, batch

    def test_virtual_gradient_is_sum_over_feature_rows_from_both_perspectives(self):
        model, batch = self.gradient_fixture()
        model(*batch).sum().backward()
        gradient = model.ft.embedding.weight.grad[:-1].reshape(8, FEATURE_COUNT, 4).sum(dim=0)
        torch.testing.assert_close(model.ft.virtual.weight.grad[:-1], gradient, atol=1e-14, rtol=0)
        self.assertGreater(float(gradient.abs().sum()), 0)
        torch.testing.assert_close(model.ft.embedding.weight.grad[-1], torch.zeros(4).double())
        torch.testing.assert_close(model.ft.virtual.weight.grad[-1], torch.zeros(4).double())

    def test_updates_and_clipping_preserve_unobserved_rows_and_folded_bounds(self):
        model, batch = self.gradient_fixture()
        virtual_before = model.ft.virtual.weight.detach().clone()
        optimizer = torch.optim.AdamW(model.parameters(), lr=0.01, weight_decay=0.1)
        model(*batch).sum().backward()
        optimizer.step()
        model.constrain()
        self.assertFalse(torch.equal(model.ft.virtual.weight[4226], virtual_before[4226]))
        for extreme in (None, -100.0, 100.0):
            with torch.no_grad():
                if extreme is not None:
                    model.ft.embedding.weight.fill_(extreme)
                    model.ft.virtual.weight.fill_(-extreme)
                    for parameter in model.head.parameters():
                        parameter.fill_(extreme)
                    model.constrain()
            self.assertTrue(torch.all(model.ft.embedding.weight[~model.ft.observed] == 0))
            torch.testing.assert_close(model.ft.virtual.weight[~model.ft.virtual_observed],
                                       model.ft.initial_virtual[~model.ft.virtual_observed], rtol=0, atol=0)
            self.assertTrue(torch.any(model.ft.initial_virtual[~model.ft.virtual_observed] != 0))
            self.assertTrue(torch.all(model.ft.expanded_weights().abs() <= 8))
            self.assertTrue(torch.all(model.ft.embedding.weight[-1] == 0))
            self.assertTrue(torch.all(model.ft.virtual.weight[-1] == 0))
            train.quantized_weights(model.floating_weights())

    def test_padding_maps_to_virtual_padding_and_contributes_nothing(self):
        model = self.make_model()
        np.testing.assert_array_equal(HALFKP.base_indices(np.array([0, 13680, 109439, 109440])),
                                      [0, 0, 13679, 13680])
        np.testing.assert_array_equal(P.base_indices(np.array([0, 13680])), [0, 13680])
        with torch.no_grad():
            model.ft.virtual.weight[0].fill_(7)
            model.head.output_weights.fill_(0.5)
            padding = torch.full((1, 145), HALFKP.feature_count)
            self.assertTrue(torch.all(model.ft(padding) == 0))
            self.assertEqual(float(model(*inputs(records_with([[]]), HALFKP))), 0)

    def test_folded_float_evaluation_and_removal_match_forward_with_nonzero_head(self):
        model = self.make_model()
        rng = np.random.default_rng(20)
        with torch.no_grad():
            model.ft.embedding.weight[:-1].copy_(torch.from_numpy(
                rng.uniform(-0.01, 0.01, (HALFKP.feature_count, 4)).astype(np.float32)))
            model.head.biases.fill_(0.1)
            model.head.output_weights.copy_(torch.tensor([[0.2, -0.3, 0.4, 0.1], [0.1, 0.2, -0.1, 0.3]]))
            model.head.output_bias.fill_(0.125)
        for count in (1, 2, 3, 91, 92, 93):
            for stm in (0, 1):
                records = records_with([[(i, (1, 65, 47, 111)[i % 4]) for i in range(count)]], stm=stm, lion=17)
                if count >= 3:
                    records["board"][0, :2] = [12, 76]
                with self.subTest(count=count, stm=stm), torch.no_grad():
                    values = records["board"], records["stm"], records["lion"]
                    np.testing.assert_allclose(float_evaluate(model.floating_weights(), model.k, *values, HALFKP),
                                               model(*inputs(records, HALFKP)).numpy() * model.k, rtol=0, atol=0.002)
                    # 独立再計算では王駒を残すので、どちらの視点でも文脈は不変。
                    squares = np.flatnonzero((records["board"][0] != 0) &
                        ~np.isin(PIECE_STATE_BY_BYTE[records["board"][0]], ROYAL_STATES))
                    after = np.repeat(records, len(squares))
                    after["board"][np.arange(len(squares)), squares] = 0
                    model.double()
                    own, opposite, ratios = inputs(records, HALFKP, torch.float64)
                    delta = model.removal_deltas(own, opposite, torch.from_numpy(piece_counts(records["board"])))
                    expected = (model(*inputs(after, HALFKP, torch.float64)) - model(own, opposite, ratios)) * model.k
                    torch.testing.assert_close(delta[0, squares], expected, atol=1e-6, rtol=0)
                    model.float()

    def test_observation_count_uses_both_perspectives_only_training_and_one_pass(self):
        records = records_with([[(5, 12), (50, 1), (143, 76)], [(5, 12), (50, 1), (143, 76)],
                                [(11, 12), (51, 1), (138, 76)]], lion=13)
        dataset = SimpleNamespace(training_indices=np.array([0, 1]), gather=Mock(side_effect=lambda i: records[i]))
        counts = train.input_feature_observations(dataset, 1, HALFKP)
        expected = np.zeros(109440, dtype=np.int64)
        # 先手は区分5・鏡映あり、後手は区分0・鏡映なし。各特徴は2回。
        expected[[69990, 72633, 76884, 81958, 8489, 11030, 1595, 13657]] = 2
        np.testing.assert_array_equal(counts, expected)
        self.assertEqual(dataset.gather.call_count, 2)


class TrainingContractsTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.threads = torch.get_num_threads()
        torch.set_num_threads(1)

    @classmethod
    def tearDownClass(cls):
        torch.set_num_threads(cls.threads)

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.data, self.init, self.base, self.initial = fixture(self.root)

    def test_parameter_groups_defaults_and_explicit_overrides(self):
        argv = arguments(self.data, self.init, self.base, self.root / "net.bin")
        for spec in (P, HALFKP):
            model = NNUE(self.initial, np.ones(13680, dtype=bool), 4, 1000, CPU, 3,
                         spec, np.ones(spec.feature_count, dtype=bool))
            for extra, expected in (([], [(0.03, 0), (0.001, 0.0001), (0.001, 0.0001)]),
                    (["--lr-linear", "0.02", "--weight-decay-linear", "0.3", "--lr-ft", "0.04",
                      "--weight-decay-ft", "0.5", "--lr-head", "0.06", "--weight-decay-head", "0.7"],
                     [(0.02, 0.3), (0.04, 0.5), (0.06, 0.7)])):
                optimizer = train.make_optimizer(model, train.build_parser().parse_args(argv + extra))
                self.assertEqual([(g["lr"], g["weight_decay"]) for g in optimizer.param_groups], expected)
                self.assertEqual([g["name"] for g in optimizer.param_groups], ["linear", "ft", "head"])
                ids = [id(p) for g in optimizer.param_groups for p in g["params"]]
                self.assertEqual(len(ids), len(set(ids)))
                self.assertEqual(set(ids), {id(p) for p in model.parameters()})
                self.assertEqual({id(p) for p in optimizer.param_groups[1]["params"]},
                                 {id(p) for p in model.ft.parameters()})
                self.assertEqual({id(p) for p in optimizer.param_groups[2]["params"]},
                                 {id(p) for p in model.head.parameters()})
        for argv in (argv[:1] + argv[3:], ["random", "--width", "4", "--seed", "1",
                    "--base", str(self.base), "--output", str(self.root / "random.bin")]):
            with redirect_stderr(StringIO()), self.assertRaises(SystemExit):
                train.build_parser().parse_args(argv)

    def test_validation_sign_accuracy_excludes_draws_and_predicts_zero_as_loss(self):
        records = records_with([[] for _ in range(6)])
        records["result"] = [2, 0, 2, 0, 1, 1]
        dataset = SimpleNamespace(validation_indices=np.arange(6), gather=lambda i: records[i],
            generations=lambda i: np.zeros(len(i), dtype=np.int64), teacher_lambdas=np.array([0.0]),
            teacher_scores=lambda i: np.zeros(len(i)))
        model = NNUE(np.zeros((13680, 2), dtype=np.float32), np.ones(13680, dtype=bool),
                     4, 1000, CPU, 3, P, np.ones(13680, dtype=bool))
        # 線形だけ、非線形だけの符号は、合算したロジットと異なる。
        components = (torch.tensor([-1000., 1000., 0., 1000., 0., 0.]),
                      torch.tensor([2., -2., 0., -1., 10., -10.]), torch.zeros(6, 2, 4))
        with patch.object(model, "components", return_value=components):
            _, statistics = train.validation_loss(model, dataset, np.array([np.nan]), 6, None)
            self.assertEqual(statistics["sign_accuracy"], 3 / 4)
            self.assertEqual(statistics["decisive_samples"], 4)
            # 0の勝ち負けが同数だと、判定を逆転しても集計値が相殺される。
            records["result"][2] = 0
            _, statistics = train.validation_loss(model, dataset, np.array([np.nan]), 6, None)
            self.assertEqual(statistics["sign_accuracy"], 1)
            records["result"] = 1
            _, statistics = train.validation_loss(model, dataset, np.array([np.nan]), 6, None)
            self.assertIsNone(statistics["sign_accuracy"])
            self.assertEqual(statistics["decisive_samples"], 0)

    def test_both_feature_sets_train_and_export_v5_with_observations_and_timing(self):
        dataset = Dataset([self.data])
        for spec in (P, HALFKP):
            output = self.root / f"{spec.name}.bin"
            argv = arguments(self.data, self.init, self.base, output, epochs=2)
            argv[2] = spec.name
            with redirect_stdout(StringIO()):
                train.main(argv)
            net = read_mnue(output)
            self.assertEqual(net.feature_set, spec.name)
            self.assertEqual(net.weights.shape, (spec.feature_count, 4))
            self.assertEqual(struct.unpack_from("<I", output.read_bytes(), 4)[0], 5)
            report = json.loads(train.training_report_path(output).read_text())
            self.assertEqual(report["feature_set"], spec.name)
            self.assertEqual(report["status"], "candidate")
            self.assertEqual(report["last_epoch"], 2)
            self.assertEqual(sum(report["observations"]["feature_rows"].values()),
                             spec.feature_count if spec.factorized else spec.feature_count // 2)
            for epoch in report["epochs"]:
                self.assertGreater(epoch["validation_seconds"], 0)
                self.assertIsNone(epoch["cuda_peak_memory_bytes"])
                self.assertTrue(0 <= epoch["sign_accuracy"] <= 1)
                if epoch["epoch"]:
                    self.assertGreater(epoch["training_positions_per_second"], 0)
                    self.assertAlmostEqual(epoch["training_positions_per_second"] * epoch["training_seconds"],
                                           len(dataset.training_indices))
                else:
                    self.assertIsNone(epoch["training_positions_per_second"])
            if spec.factorized:
                counts = train.input_feature_observations(dataset, 17, spec)
                for name, active in (("feature_rows", counts > 0),
                                     ("virtual_rows", counts.reshape(8, 13680).any(axis=0))):
                    self.assertEqual(report["observations"][name],
                                     {"observed": int(active.sum()), "unobserved": int((~active).sum())})
            self.assertEqual(set(report["gradients"][0]), {"step", "linear", "ft", "head"})

    def test_halfkp_random_is_seeded_and_has_unshared_hidden_and_head_weights(self):
        paths = [self.root / name for name in ("random.bin", "copy.bin")]
        for path in paths:
            train.main(["random", "--feature-set", HALFKP.name, "--width", "4", "--seed", "9",
                        "--base", str(self.base), "--output", str(path)])
        self.assertEqual(paths[0].read_bytes(), paths[1].read_bytes())
        net = read_mnue(paths[0])
        self.assertEqual(net.weights.shape, (109440, 4))
        self.assertFalse(np.array_equal(net.weights[0], net.weights[13680]))
        self.assertFalse(np.array_equal(net.biases[:2], net.biases[2:]))
        self.assertFalse(np.array_equal(net.output_weights[:, :2], net.output_weights[:, 2:]))
        self.assertLessEqual(np.abs(net.weights).max(), 1016)
        self.assertLessEqual(np.abs(net.output_weights).max(), 8192)


if __name__ == "__main__":
    unittest.main()
