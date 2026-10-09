"""独立した既知評価で、Rust探査の引数、完全一致、および報告を検証する。"""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import numpy as np

from helpers import records_with, write_mnsd, write_provenance
from minase_train.data import mnpt
from minase_train.data.mnpt import initial_piece_values
from minase_train.data.mnsd import HEADER_LENGTH, Dataset, map_records
import minase_train.diagnostics.nnue as diagnostics
from minase_train.nnue.model import NNUE
from minase_train.nnue.train import quantized_weights
import torch
from minase_train.diagnostics.nnue import compare, main, rust_probe
from minase_train.nnue.format import write_mnue


class DiagnosticsTest(unittest.TestCase):
    def test_probe_passes_net_positions_and_preserves_command_prefix(self):
        for command in (["nnue_probe"], ["minase", "dev", "nnue-probe"]):
            with patch("minase_train.diagnostics.nnue.subprocess.run", return_value=Mock(stdout="[]")) as run:
                self.assertEqual(rust_probe(command)(Path("net.bin"), Path("samples.bin")), [])
            run.assert_called_once_with(command + ["--net", "net.bin", "--positions", "samples.bin"],
                                        check=True, capture_output=True, text=True)
        with self.assertRaises(ValueError):
            rust_probe([])

    def test_scores_counts_order_and_report_checksums(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            net, data, output = [root / name for name in ("net.bin", "data.bin", "report.json")]
            linear = np.zeros((13680, 2), dtype=np.int16)
            linear[29 * 144:30 * 144] = 8
            write_mnue(net, linear, initial_piece_values(), 1000, np.zeros((13680, 2), dtype=np.int16),
                       np.zeros(2, dtype=np.int32), np.zeros((2, 2), dtype=np.int16), 0)
            board = np.zeros((3, 144), dtype=np.uint8)
            for i, row in enumerate(board):
                row[:i + 1] = 1
            write_mnsd(data, seed=0, checksum=b"a" * 32, games=[0, 1, 2], board=board)
            expected = [{"index": i, "eval": i + 1} for i in range(3)]
            probe = Mock(return_value=expected)
            self.assertEqual(compare(net, data, probe)["samples"], 3)
            probe.assert_called_once_with(net, data)
            for wrong in (expected[:-1], expected + [{"index": 3, "eval": 4}], expected[::-1],
                          [{"index": 0, "eval": 2}, *expected[1:]],
                          [{"index": 0, "eval": 1.0}, *expected[1:]],
                          [{"index": False, "eval": 1}, *expected[1:]],
                          [{"index": 0}, *expected[1:]], [None, *expected[1:]]):
                with self.subTest(rows=wrong), self.assertRaisesRegex(ValueError, "index [0-3]: Python=.*Rust="):
                    compare(net, data, Mock(return_value=wrong))
            with patch("minase_train.diagnostics.nnue.rust_probe", return_value=probe):
                main(["compare", "--net", str(net), "--positions", str(data), "--output", str(output),
                      "--probe-command", "/bin/true"])
            report = json.loads(output.read_text())
            for key, path in (("net_sha256", net), ("positions_sha256", data), ("probe_sha256", Path("/bin/true"))):
                self.assertEqual(report[key], hashlib.sha256(path.read_bytes()).hexdigest())
            self.assertEqual(report["probe_command"], ["/bin/true", "--net", str(net), "--positions", str(data)])
            with self.assertRaisesRegex(ValueError, "already exists"):
                main(["compare", "--net", str(net), "--positions", str(data), "--output", str(output),
                      "--probe-command", "/bin/true"])


class PhaseFourDiagnosticsTest(unittest.TestCase):
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
        self.net, self.base, self.data, self.output = [
            self.root / name for name in ("net.bin", "base.bin", "data.bin", "report.json")]

    def write_network(self, linear):
        write_mnue(self.net, linear, initial_piece_values(), 1000,
                   np.zeros((13680, 2), dtype=np.int16), np.zeros(2, dtype=np.int32),
                   np.zeros((2, 2), dtype=np.int16), 0)

    def distribution_fixture(self):
        # 50 cpの歩を0,1,2,3,6,7,20,21枚とし、全指定帯で分散を持たせる。
        pawns = np.tile([0, 1, 2, 3, 6, 7, 20, 21], 6)
        counts = np.repeat([27, 42, 72], 16)
        board = np.zeros((48, 144), dtype=np.uint8)
        for row, count, pawn in zip(board, counts, pawns):
            row[:pawn] = 1
            row[pawn:count - 2] = 18
            row[142:] = [12, 76]
        write_mnsd(self.data, seed=0, checksum=b"a" * 32, games=list(range(48)), board=board)
        linear = np.zeros((13680, 2), dtype=np.int16)
        linear[29 * 144:30 * 144] = 400
        self.write_network(linear)
        mnpt.write_mnpt_v3(self.base, linear[:, 0], linear[:, 1], initial_piece_values(), 1000,
                           np.zeros((13680, 1), dtype=np.int16), np.ones(1, dtype=np.int8), 0)
        return pawns * 50, counts - 2

    def distribution_arguments(self, sample=40):
        return ["distribution", "--data", str(self.data), "--sample", str(sample), "--seed", "7",
                "--pair", "same", str(self.base), str(self.net), "--output", str(self.output)]

    def test_distribution_identical_evaluations_sampling_checksums_and_all_intervals(self):
        scores, phases = self.distribution_fixture()
        main(self.distribution_arguments() + ["--pair", "second", str(self.base), str(self.net)])
        report = json.loads(self.output.read_text())
        selected = np.sort(np.random.default_rng(7).choice(48, 40, replace=False))
        self.assertEqual(report["sample_indices_sha256_le_i64"],
                         hashlib.sha256(selected.astype("<i8").tobytes()).hexdigest())
        self.assertEqual(report["population_count"], 48)
        self.assertEqual(report["input_files"][0]["sha256"], hashlib.sha256(self.data.read_bytes()).hexdigest())
        self.assertEqual(len(report["pairs"]), 2)
        pair = report["pairs"][0]
        for key, path in (("baseline", self.base), ("candidate", self.net)):
            self.assertEqual(pair[key]["sha256"], hashlib.sha256(path.read_bytes()).hexdigest())
            self.assertAlmostEqual(pair[key + "_statistics"]["mean_cp"], scores[selected].mean())
        for row in [pair, *pair["by_phase_q"], *pair["by_baseline_absolute_cp"]]:
            self.assertAlmostEqual(row["spearman_correlation"], 1)
            self.assertAlmostEqual(row["std_ratio_candidate_over_baseline"], 1)
            self.assertAlmostEqual(row["regression_slope"], 1)
            self.assertAlmostEqual(row["regression_intercept_cp"], 0)
            self.assertAlmostEqual(row["regression_residual_std_cp"], 0)
        self.assertEqual(pair["verdict"], "全体の尺度はほとんど変えていない")
        for rows, values, bounds in (
            (pair["by_phase_q"], phases[selected], [(0, 29), (30, 59), (60, 90)]),
            (pair["by_baseline_absolute_cp"], scores[selected], [(0, 99), (100, 299), (300, 999), (1000, None)]),
        ):
            for row, (lower, upper) in zip(rows, bounds):
                self.assertEqual((row["lower_inclusive"], row["upper_inclusive"]), (lower, upper))
                mask = values >= lower
                if upper is not None:
                    mask &= values <= upper
                self.assertEqual(row["sample_count"], np.count_nonzero(mask))

    def test_distribution_includes_fm_and_tied_ranks(self):
        scores, _ = self.distribution_fixture()
        linear, endgame, values, k, embeddings, signs, exponent = mnpt.read_mnpt_v3(self.base)
        # 最初の2升に歩があるときだけFMの寄与は(2² - 2)/2 = 1 cp。
        embeddings = embeddings.copy()
        embeddings[29 * 144:29 * 144 + 2] = 1
        mnpt.write_mnpt_v3(self.base, linear, endgame, values, k, embeddings, signs, exponent)
        main(self.distribution_arguments(sample=48))
        pair = json.loads(self.output.read_text())["pairs"][0]
        self.assertAlmostEqual(pair["baseline_statistics"]["mean_cp"], (scores + (scores >= 100)).mean())
        self.assertEqual(pair["absolute_difference_percentiles_cp"]["99"], 1)
        statistics = diagnostics.compare_scores(np.array([0, 0, 2, 4]), np.array([0, 2, 2, 4]))
        self.assertAlmostEqual(statistics["spearman_correlation"], 5 / 6)

    def test_distribution_rejects_invalid_samples_duplicate_pairs_and_constant_intervals(self):
        self.distribution_fixture()
        for sample in (1, 49):
            with self.subTest(sample=sample), self.assertRaises(ValueError):
                main(self.distribution_arguments(sample=sample))
        with self.assertRaisesRegex(ValueError, "unique"):
            main(self.distribution_arguments() + ["--pair", "same", str(self.base), str(self.net)])
        self.write_network(np.zeros((13680, 2), dtype=np.int16))
        with self.assertRaisesRegex(ValueError, "variance"):
            main(self.distribution_arguments())
        with self.assertRaisesRegex(ValueError, "interval"):
            diagnostics.compare_intervals(np.array([0, 1]), np.array([0, 1]),
                                          np.array([0, 0]), ((1, 2),))
        self.assertFalse(self.output.exists())

    def move_fixture(self):
        self.distribution_fixture()
        write_mnsd(self.data, seed=0, checksum=b"a" * 32, games=[0, 1, 2])
        candidate = [{"index": 2, "moves": []},
                     {"index": 1, "moves": [{"move": "a", "delta": 3}]},
                     {"index": 0, "moves": [{"move": "b", "delta": -2}, {"move": "a", "delta": 6}]}]
        baseline = [{"index": 0, "moves": [{"move": "a", "delta": 2, "delta_pst": 0},
                                            {"move": "b", "delta": 0, "delta_pst": -4}]},
                    {"index": 1, "moves": [{"move": "a", "delta": -1, "delta_pst": 1}]},
                    {"index": 2, "moves": []}]
        args = ["moves", "--net", str(self.net), "--base", str(self.base), "--positions", str(self.data),
                "--probe-command", "/bin/true", "dev", "nnue-probe",
                "--base-probe-command", "/bin/true", "dev", "pst-probe", "--output", str(self.output)]
        return candidate, baseline, args

    def test_moves_aligns_indices_and_move_strings_and_reports_statistics(self):
        candidate, baseline, args = self.move_fixture()
        with patch.object(diagnostics, "rust_probe", side_effect=[lambda *args: candidate, lambda *args: baseline]):
            main(args)
        report = json.loads(self.output.read_text())
        self.assertEqual(report["probe_command"][-1], "--moves")
        self.assertEqual(report["base_probe_command"][-1], "--moves")
        self.assertIn("--pst", report["base_probe_command"])
        row = report["by_position"][0]
        for name, expected in (("candidate", [2, 4, 4, 6]), ("baseline", [1, 1, 1, 2]),
                               ("candidate_minus_baseline", [1, 3, 3, 4]),
                               ("candidate_minus_baseline_pst", [4, 2, 4, 6])):
            self.assertEqual(row[name]["count"], 2)
            for field, value in zip(("mean_cp", "std_cp", "mean_absolute_cp", "max_absolute_cp"), expected):
                self.assertEqual(row[name][field], value)
        self.assertEqual(report["by_position"][1]["candidate_minus_baseline"]["mean_cp"], 4)
        self.assertEqual(report["by_position"][2]["candidate"]["count"], 0)
        self.assertIsNone(report["by_position"][2]["candidate"]["mean_cp"])
        for option in ("--net", "--pst"):
            with patch.object(diagnostics.subprocess, "run", return_value=Mock(stdout="[]")) as run:
                rust_probe(["minase", "dev", "probe"], weight_option=option, moves=True)(self.net, self.data)
            self.assertEqual(run.call_args.args[0], ["minase", "dev", "probe", option, str(self.net),
                                                     "--positions", str(self.data), "--moves"])

    def test_moves_missing_moves_write_counts_and_raise(self):
        candidate, baseline, args = self.move_fixture()
        candidate[2]["moves"].pop()  # 基準だけにaが残る。
        baseline[0]["moves"].pop()  # 候補だけにbが残る。
        with patch.object(diagnostics, "rust_probe", side_effect=[lambda *args: candidate, lambda *args: baseline]), \
                self.assertRaisesRegex(ValueError, "candidate_only=1, baseline_only=1"):
            main(args)
        report = json.loads(self.output.read_text())
        self.assertEqual(report["candidate_only_count"], 1)
        self.assertEqual(report["baseline_only_count"], 1)
        self.assertEqual(report["by_position"][0]["matched_count"], 0)
        self.assertEqual(report["by_position"][1]["matched_count"], 1)

    def removal_fixture(self, *, nonlinear):
        # 両手番、先獅子、成駒、および補間の上下端を含め、全局面を検証用の同一対局へ置く。
        counts = (1, 2, 3, 91, 92, 93)
        boards = [[(i, (1, 65, 47, 111)[i % 4]) for i in range(count)] for count in counts]
        records = records_with(boards * 2, lion=0)
        records["stm"][6:] = 1
        records["board"][-1, :2] = [12, 22]  # 王・太子は除去対象外。
        write_mnsd(self.data, seed=0, checksum=b"a" * 32, games=[0] * len(records), board=records["board"])
        mapped = map_records(self.data).copy()
        mapped["stm"], mapped["lion"] = records["stm"], records["lion"]
        header = self.data.read_bytes()[:HEADER_LENGTH]
        self.data.write_bytes(header + mapped.tobytes())
        write_provenance(self.data)
        # seed=0, game=0は検証集合。訓練集合への依存がないことも固定する。
        self.assertEqual(len(Dataset([self.data]).validation_indices), len(records))
        initial = np.tile(-mnpt.initial_weights().astype(np.float32)[:, None] / 8, (1, 2))
        if nonlinear:
            initial[:, 1] *= 0.9
        model = NNUE(initial, np.ones(13680, dtype=bool), 4, 1000, torch.device("cpu"), 18)
        with torch.no_grad():
            if nonlinear:
                model.biases.fill_(0.13)
                model.output_weights.copy_(torch.tensor([[0.2, -0.3], [0.4, 0.1]]))
                model.output_bias.fill_(0.125)
        weights = model.floating_weights()
        float_path = self.root / "net-float.npz"
        np.savez(float_path, **weights)
        linear, hidden, biases, output, bias = quantized_weights(weights)
        write_mnue(self.net, linear, initial_piece_values(), 1000, hidden, biases, output, bias)
        args = ["removal", "--float", str(float_path), "--net", str(self.net), "--data", str(self.data),
                "--sample", str(len(records)), "--seed", "8", "--output", str(self.output)]
        return records, args

    def test_removal_matches_recomputation_at_phase_boundaries_and_excludes_royals(self):
        records, args = self.removal_fixture(nonlinear=True)
        main(args)
        report = json.loads(self.output.read_text())
        self.assertEqual(report["split"], "validation")
        self.assertEqual(report["removal_count"], int(np.count_nonzero(records["board"])) - 2)
        self.assertTrue(report["float_batch_vs_recomputed"]["passed"])
        self.assertLessEqual(report["float_batch_vs_recomputed"]["max_absolute_cp"], 1e-6)
        self.assertGreater(report["float_vs_integer_recomputed"]["max_absolute_cp"], 0)
        self.assertNotIn("11", report["by_piece_state"])
        self.assertNotIn("21", report["by_piece_state"])

    def test_removal_reverse_counts_magnitudes_and_zero_deltas(self):
        records, args = self.removal_fixture(nonlinear=False)
        main(args)
        report = json.loads(self.output.read_text())
        total = int(np.count_nonzero(records["board"])) - 2
        golds = int(np.count_nonzero(np.isin(records["board"], [47, 111])))
        pawns = total - golds
        self.assertEqual(report["reversals"]["count"], total)
        self.assertEqual(report["reversals"]["total"], total)
        self.assertEqual(report["reversals"]["fraction"], 1)
        self.assertEqual(report["reversals"]["max_absolute_cp"], 378)
        self.assertAlmostEqual(report["reversals"]["mean_absolute_cp"], (100 * pawns + 378 * golds) / total)
        self.assertEqual(report["by_piece_state"]["29"]["count"], pawns)
        self.assertEqual(report["by_piece_state"]["17"]["count"], golds)
        self.assertEqual(report["float_vs_integer_recomputed"]["max_absolute_cp"], 0)
        # 整数差分0は逆向きに含めず、参照差分0は分母からも除く。
        statistics = diagnostics.reversal_statistics(np.array([0, -2, 4, 8]), np.array([1, 1, -1, 0]))
        self.assertEqual(statistics, {"count": 2, "total": 3, "fraction": 2 / 3,
                                     "mean_absolute_cp": 3, "max_absolute_cp": 4})

    def test_removal_corrupted_batch_is_reported_and_rejected(self):
        _, args = self.removal_fixture(nonlinear=True)
        original = diagnostics.float_removal_deltas
        with patch.object(diagnostics, "float_removal_deltas", side_effect=lambda *args: original(*args) + 0.01), \
                self.assertRaisesRegex(ValueError, "batched removal differs"):
            main(args)
        report = json.loads(self.output.read_text())
        self.assertFalse(report["float_batch_vs_recomputed"]["passed"])
        self.assertAlmostEqual(report["float_batch_vs_recomputed"]["max_absolute_cp"], 0.01)

    def test_removal_samples_only_validation_and_accepts_training_output(self):
        from contextlib import redirect_stdout
        from io import StringIO
        from test_train_nnue import arguments, fixture
        from minase_train.nnue import train

        data, initial, base, _ = fixture(self.root)
        with redirect_stdout(StringIO()):
            train.main(arguments(data, initial, base, self.net, epochs=1))
        dataset = Dataset([data])
        sample, seed = 3, 42
        indices = np.sort(np.random.default_rng(seed).choice(dataset.validation_indices, sample, replace=False))
        main(["removal", "--float", str(mnpt.float_weights_path(self.net)), "--net", str(self.net),
              "--data", str(data), "--sample", str(sample), "--seed", str(seed), "--output", str(self.output)])
        report = json.loads(self.output.read_text())
        self.assertEqual(report["sample_indices_sha256_le_i64"],
                         hashlib.sha256(indices.astype("<i8").tobytes()).hexdigest())
        self.assertEqual(report["sample_count"], sample)
        self.assertEqual(report["removal_count"],
                         int(np.count_nonzero(dataset.gather(indices)["board"])) - 2 * sample)
        self.assertEqual(report["population_count"], len(dataset.validation_indices))
        self.assertTrue(report["float_batch_vs_recomputed"]["passed"])
        self.assertEqual(report["float"]["sha256"],
                         hashlib.sha256(mnpt.float_weights_path(self.net).read_bytes()).hexdigest())
        for invalid in (0, len(dataset.validation_indices) + 1):
            with self.subTest(sample=invalid), self.assertRaisesRegex(ValueError, "sample"):
                diagnostics.removal(mnpt.float_weights_path(self.net), self.net, [data], invalid, seed)

    def test_moves_rejects_duplicate_or_missing_positions_and_duplicate_moves(self):
        candidate, baseline, _ = self.move_fixture()
        variants = (candidate[:-1], candidate + [candidate[0]],
                    [{"index": 0, "moves": [{"move": "a", "delta": 1}] * 2}, *candidate[:2]])
        for rows in variants:
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                diagnostics.moves(self.net, self.base, self.data, lambda *args: rows, lambda *args: baseline)


if __name__ == "__main__":
    unittest.main()
