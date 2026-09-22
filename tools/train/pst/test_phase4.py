"""段階9フェーズ4の交換形式と教師の分類の契約を検証する。"""

from contextlib import redirect_stdout
from io import StringIO
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

import numpy as np
import torch

from features import FEATURE_COUNT, INITIAL_BOARD
from mnsd import Dataset, HEADER, RECORD_DTYPE, provenance_path, write_mnsd
from test_train_pst import write_provenance
from train_pst import build_targets, estimate_generation_ks, make_model, validation_loss


def write_rescore(path, source, rows, *, nodes=200000, commit="a" * 40):
    """共有契約第1節のオフセットと16バイトの記録から直接構築する。"""
    header = bytearray(240)
    struct.pack_into("<4sI", header, 0, b"MNRS", 1)
    header[8:40] = hashlib.sha256(source.read_bytes()).digest()
    struct.pack_into("<Q", header, 40, len(rows))
    targets = b"".join(struct.pack("<Q", i) for i, row in enumerate(rows) if row[0])
    header[48:80] = hashlib.sha256(targets).digest()
    struct.pack_into("<Q", header, 80, len(targets) // 8)
    header[88:120] = b"a" * 32
    struct.pack_into("<I", header, 120, nodes)
    header[124:156] = b"L0,P0,R1,E0".ljust(32, b"\0")
    struct.pack_into("<I", header, 156, 16)
    header[164:204] = commit.encode("ascii")
    header[204:236] = b"b" * 32
    path.write_bytes(header + b"".join(struct.pack("<BBhIQ", *row) for row in rows))


class Phase4Test(unittest.TestCase):
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
        write_mnsd(self.source, self.rows, seed=11, network_checksum=b"a" * 32, teacher_nodes=100000)
        self.provenance = write_provenance(self.source)

    def metadata(self, **changes):
        value = dict(self.provenance, **changes)
        provenance_path(self.source).write_text(json.dumps(value))

    def test_required_provenance_and_checksum(self):
        provenance_path(self.source).unlink()
        with self.assertRaises(OSError):
            Dataset([self.source])
        self.metadata(mnsd_sha256="0" * 64)
        with self.assertRaises(ValueError):
            Dataset([self.source])

    def test_provenance_domains(self):
        cases = [
            {"format": "wrong"}, {"version": 2}, {"version": True},
            {"result_origin": "unknown"}, {"start_origin": "unknown"},
            {"lambda": -0.01}, {"lambda": 1.01}, {"lambda": float("nan")}, {"lambda": True},
            {"games": []}, {"teacher": {}},
        ]
        for key, value in (("generation_commit", "bad"), ("network_checksum", "bad"),
                           ("nodes", -1), ("nodes", True), ("nodes", 2**32),
                           ("rule_set", ""), ("search_condition", "unknown")):
            cases.append({"teacher": dict(self.provenance["teacher"], **{key: value})})
        for change in cases:
            with self.subTest(change=change):
                self.metadata(**change)
                with self.assertRaises(ValueError):
                    Dataset([self.source])
        for key in self.provenance:
            value = dict(self.provenance)
            del value[key]
            provenance_path(self.source).write_text(json.dumps(value))
            with self.assertRaises(ValueError):
                Dataset([self.source])

    def test_game_mapping_requirements(self):
        for games in (None, [{"game": 1, "id": "abc"}],
                      [{"game": 1, "id": "abc", "ply": -1}],
                      [{"game": 0, "id": "abc", "ply": 0}],
                      [{"game": 1, "id": "", "ply": 0}],
                      [{"game": 1, "id": "abc", "ply": 0}] * 2):
            with self.subTest(games=games):
                self.metadata(start_origin="human-game", games=games)
                with self.assertRaises(ValueError):
                    Dataset([self.source])

    def human_games(self):
        return [{"game": i, "id": f"game-{i}", "ply": 40} for i in range(1, 101)]

    def test_id_split_agrees_across_origins_files_and_seeds(self):
        games = self.human_games()
        self.metadata(result_origin="human", start_origin="human-game", games=games, **{"lambda": 0})
        other = self.root / "other.bin"
        write_mnsd(other, self.rows, seed=11, network_checksum=b"a" * 32)
        write_provenance(other, start_origin="human-game", games=games)
        dataset = Dataset([self.source, other])
        expected = [int.from_bytes(hashlib.sha256(g["id"].encode()).digest()[:8], "little") % 20 == 0 for g in games]
        np.testing.assert_array_equal(dataset.validation_masks[0], expected)
        np.testing.assert_array_equal(dataset.validation_masks[1], expected)
        self.assertEqual(dataset.game_keys(np.array([0, 100])), [("human", "game-1")] * 2)
        self.assertEqual(dataset.generation_count, 2)
        # 対局結果だけを教師とする実戦棋譜の平均に、実戦開始の自己対局を混ぜない。
        groups = {}
        model = make_model(torch.zeros((FEATURE_COUNT, 1)), torch.device("cpu"), "single")
        validation_loss(model, dataset, np.array([np.nan, 1000.]), 1., 100, torch.device("cpu"),
                        indices=np.arange(100, 200), breakdown=groups)
        self.assertEqual(groups["origins"]["human-start-selfplay"]["positions"], 100)
        self.assertIsNone(groups["human_game_mean"]["loss"])

    def test_all_seven_fields_define_classes(self):
        variants = [("generation_commit", "a" * 40), ("network_checksum", "b" * 64),
                    ("nodes", 200000), ("rule_set", "another-rule"), ("search_condition", "standalone"),
                    ("result_origin", "human"), ("start_origin", "human-game")]
        for field, value in variants:
            with self.subTest(field=field):
                other = self.root / "other.bin"
                write_mnsd(other, self.rows, seed=22, network_checksum=b"a" * 32)
                changes = {"teacher": dict(self.provenance["teacher"])}
                if field in changes["teacher"]:
                    changes["teacher"][field] = value
                else:
                    changes[field] = value
                    changes["games"] = self.human_games()
                write_provenance(other, **changes)
                self.assertEqual(Dataset([self.source, other]).generation_count, 2)

    def test_rejects_duplicate_content_and_conflicting_mix(self):
        other = self.root / "other.bin"
        other.write_bytes(self.source.read_bytes())
        write_provenance(other)
        with self.assertRaises(ValueError):
            Dataset([self.source, other])
        write_mnsd(other, self.rows, seed=22, network_checksum=b"a" * 32, teacher_nodes=100000)
        write_provenance(other, **{"lambda": 0.5})
        with self.assertRaises(ValueError):
            Dataset([self.source, other])

    def test_rescore_rejects_partial_and_invalid_contract_fields(self):
        rows = [(1, 0, 42, 1, 100)] * 100
        write_rescore(self.sidecar, self.source, rows)
        original = self.sidecar.read_bytes()
        mutations = [(0, b"WRNG"), (4, struct.pack("<I", 2)), (8, bytes(32)),
                     (40, struct.pack("<Q", 99)), (48, bytes(32)), (80, struct.pack("<Q", 99)),
                     (160, b"\1"), (161, b"\1"), (236, b"\1"), (164, b"z"),
                     (240, b"\3"), (241, b"\2"), (244, bytes(4))]
        for offset, value in mutations:
            changed = bytearray(original)
            changed[offset:offset + len(value)] = value
            self.sidecar.write_bytes(changed)
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                Dataset([self.source], rescore=[self.sidecar])
        for value in (original[:-1], original[:-16], original + bytes(16)):
            self.sidecar.write_bytes(value)
            with self.assertRaises(ValueError):
                Dataset([self.source], rescore=[self.sidecar])
        with self.assertRaises(ValueError):
            Dataset([self.source], rescore=[])

    def test_rescore_direct_input_equivalence_and_original_feature_rows(self):
        # 共有契約の境界: ±29000、捕獲/成り、未完了。重なる理由も含む。
        rows = [(1, 0, 100 if i % 2 else -100, 3, 200) for i in range(100)]
        rows[:7] = [(0, 0, 0, 0, 0), (1, 0, 28999, 1, 50), (1, 0, 29000, 1, 50),
                    (1, 0, -29000, 1, 50), (1, 1, 10, 1, 50), (2, 0, 0, 0, 20),
                    (1, 1, -32768, 1, 50)]
        before = self.source.read_bytes()
        write_rescore(self.sidecar, self.source, rows)
        features = self.root / "features.bin"
        features.write_bytes(HEADER.pack(b"MNKF", 1, 1, 1, 100, hashlib.sha256(before).digest()) + bytes(range(100)))
        replaced = Dataset([self.source], rescore=[self.sidecar], king_features=[features], extra_columns=[0])
        expected_kept = np.array([0, 1, *range(7, 100)])
        self.assertEqual(replaced.exclusions, {"mate_band": 3, "tactical": 2, "depth_incomplete": 1, "total": 5})
        kept = np.sort(np.concatenate((replaced.training_indices, replaced.validation_indices)))
        np.testing.assert_array_equal(kept, expected_kept)
        np.testing.assert_array_equal(replaced.gather_extra(kept)[:, 0], expected_kept)
        direct_rows = self.rows[expected_kept].copy()
        direct_rows["score"][1:] = [rows[i][2] for i in expected_kept[1:]]
        direct = self.root / "direct.bin"
        write_mnsd(direct, direct_rows, seed=11, network_checksum=b"a" * 32)
        write_provenance(direct)
        dataset = Dataset([direct])
        for split in ("training_indices", "validation_indices"):
            left_indices, right_indices = getattr(replaced, split), getattr(dataset, split)
            left, right = replaced.gather(left_indices), dataset.gather(right_indices)
            np.testing.assert_array_equal(left, right)
            targets = build_targets(left, np.full(replaced.generation_count, 400.), replaced.generations(left_indices), replaced.teacher_lambdas)
            expected = 0.75 / (1 + np.exp(-right["score"].astype(float) / 400)) + 0.25 * right["result"] / 2
            np.testing.assert_allclose(targets, expected, rtol=1e-6)
        self.assertEqual(self.source.read_bytes(), before)
        classes = replaced.teacher_classes
        self.assertEqual(classes[1].generation_commit, "a" * 40)
        self.assertEqual(classes[1].nodes, 200000)
        self.assertEqual(classes[1].search_condition, "standalone")
        self.assertEqual(classes[1].result_origin, "selfplay")

    def test_direct_saved_scores_reproduce_fitted_k_and_targets(self):
        rows = [(1, 0, -300 if i % 2 == 0 else 700, 2, 500) for i in range(100)]
        rows[10] = (1, 0, 29000, 2, 500)
        rows[20] = (1, 1, 0, 2, 500)
        rows[30] = (2, 0, 0, 0, 500)
        write_rescore(self.sidecar, self.source, rows)
        replaced = Dataset([self.source], rescore=[self.sidecar])
        kept = np.array([i for i in range(100) if i not in (10, 20, 30)])
        direct_rows = self.rows[kept].copy()
        direct_rows["score"] = [rows[i][2] for i in kept]
        path = self.root / "direct-complete.bin"
        write_mnsd(path, direct_rows, seed=11, network_checksum=b"a" * 32,
                   teacher_nodes=200000, generation_commit="a" * 40)
        metadata = write_provenance(path)
        metadata["teacher"]["search_condition"] = "standalone"
        provenance_path(path).write_text(json.dumps(metadata))
        direct = Dataset([path])
        self.assertEqual(replaced.class_metadata(), direct.class_metadata())
        left_k, _ = estimate_generation_ks(replaced, indices=replaced.training_indices)
        right_k, _ = estimate_generation_ks(direct, indices=direct.training_indices)
        np.testing.assert_array_equal(left_k, right_k)
        for split in ("training_indices", "validation_indices"):
            left_indices, right_indices = getattr(replaced, split), getattr(direct, split)
            left, right = replaced.gather(left_indices), direct.gather(right_indices)
            np.testing.assert_array_equal(left, right)
            np.testing.assert_array_equal(
                build_targets(left, left_k, replaced.generations(left_indices), replaced.teacher_lambdas),
                build_targets(right, right_k, direct.generations(right_indices), direct.teacher_lambdas))
        self.assertEqual(replaced.exclusions["total"], len(self.rows) - direct.record_count)

    def test_independent_k_with_same_checksum_and_training_only(self):
        rows = [(1, 0, 700, 1, 20) if i % 2 else (0, 0, 0, 0, 0) for i in range(100)]
        write_rescore(self.sidecar, self.source, rows)
        dataset = Dataset([self.source], rescore=[self.sidecar])
        calls = []
        def fit(scores, results):
            calls.append((scores.copy(), results.copy()))
            return [100., 200.][len(calls)-1]
        with patch("train_pst.estimate_k", side_effect=fit):
            ks, counts = estimate_generation_ks(dataset, indices=dataset.training_indices)
        np.testing.assert_array_equal(ks, [100, 200])
        self.assertEqual(sum(counts), len(dataset.training_indices))
        for c, (scores, results) in enumerate(calls):
            records = dataset.gather(dataset.generation_training_indices(c))
            np.testing.assert_array_equal(scores, records["score"])
            np.testing.assert_array_equal(results, records["result"])
        np.testing.assert_array_equal(calls[1][0], 700)

    def test_zero_lambda_never_estimates_or_uses_teacher_k(self):
        self.metadata(result_origin="human", start_origin="human-game", games=self.human_games(), **{"lambda": 0})
        dataset = Dataset([self.source])
        with patch("train_pst.estimate_k", side_effect=AssertionError("K must not be estimated")):
            ks, _ = estimate_generation_ks(dataset, indices=dataset.training_indices)
        self.assertTrue(np.isnan(ks[0]))
        records = dataset.gather(dataset.training_indices)
        np.testing.assert_array_equal(build_targets(records, ks, dataset.generations(dataset.training_indices), dataset.teacher_lambdas), records["result"] / 2)
        records["score"] = -32768
        np.testing.assert_array_equal(build_targets(records, ks, np.zeros(len(records), dtype=int), dataset.teacher_lambdas), records["result"] / 2)

    def test_class_specific_mixing(self):
        records = self.rows[:3].copy()
        records["score"] = 400
        records["result"] = [2, 0, 1]
        actual = build_targets(records, np.array([np.nan, 400., 800.]), np.arange(3), np.array([0., 0.75, 1.]))
        np.testing.assert_allclose(actual, [1., 0.75 / (1 + np.exp(-1)), 1 / (1 + np.exp(-0.5))], rtol=1e-6)

    def test_cpu_training_cli_records_initial_metrics_and_exclusions(self):
        from train_pst import initial_piece_values, main, write_mnpt
        self.metadata(**{"lambda": 0})
        rows = [(1, 0, 100, 1, 20)] * 100
        rows[0] = (2, 0, 0, 0, 10)
        write_rescore(self.sidecar, self.source, rows)
        base = self.root / "base.bin"
        output = self.root / "trained.bin"
        weights = np.zeros(FEATURE_COUNT, dtype=np.int16)
        write_mnpt(base, weights, weights, initial_piece_values(), 400.)
        stdout = StringIO()
        with patch("train_pst.estimate_k", side_effect=AssertionError("unexpected K estimate")), redirect_stdout(stdout):
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
        write_mnsd(self.source, self.rows, seed=11, network_checksum=b"a" * 32)
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
        write_mnsd(self.source, rows, seed=11, network_checksum=b"a" * 32)
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


if __name__ == "__main__":
    unittest.main()
