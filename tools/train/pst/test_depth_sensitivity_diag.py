"""指示書の契約に基づく診断Aの合成データテスト。

plan: 駒数47、王駒数、探索値の境界、同じ検査和の世代、母集団の重み。
sample: 最大剰余、層内の同数抽出、不足、固定シード、対局分割、対象一覧。
estimate: 手計算のD、通常値の境界、層除外、群・層をまたぐ対局再標本化。
交換形式: ヘッダ・長さ・検査和・対象状態・探索条件の不一致を拒否。
"""

import copy
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import unittest

import numpy as np

import depth_sensitivity_diag as diag
from mnsd import RECORD_DTYPE, hash64, write_mnsd


class DepthSensitivityTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.seed = 71
        games = np.arange(1, 200, dtype=np.uint32)
        hashes = hash64(self.seed, games)
        self.pilot = games[(hashes % 20 != 0) & (hashes % 2 == 0)].tolist()
        self.main = games[hashes % 2 == 1].tolist()
        self.validation = int(games[hashes % 20 == 0][0])

    def row(self, *, game=None, score=0, stm=0, pieces=47, own=1, opponent=1):
        row = np.zeros(1, dtype=RECORD_DTYPE)
        row["board"][0, :pieces] = 1
        # 王将は12/76、太子（成り）は51/115。醉象13/77とは区別する。
        kings = []
        for color, count in ((stm, own), (1 - stm, opponent)):
            kings.extend(([12 + color * 64, 51 + color * 64, 12 + color * 64])[:count])
        row["board"][0, :len(kings)] = kings
        row["stm"], row["lion"], row["score"] = stm, 255, score
        row["game"] = self.pilot[0] if game is None else game
        return row[0]

    def source(self, rows, columns, *, name="source.mnsd", network=1, seed=None):
        path = self.root / name
        records = np.array(rows, dtype=RECORD_DTYPE)
        write_mnsd(path, records, seed=self.seed if seed is None else seed,
                   network_checksum=bytes([network]) * 32)
        features = np.zeros((len(rows), 118), dtype=np.uint8)
        for i, active in enumerate(columns):
            features[i, active] = 1
        feature_path = self.root / (name + ".mnkf")
        header = struct.pack("<4sIIIQ32s", b"MNKF", 1, 2, 118, len(rows),
                             hashlib.sha256(path.read_bytes()).digest())
        feature_path.write_bytes(header + features.tobytes())
        return path, feature_path

    def sample(self, data, features, n=100, split="pilot", seed=1, directory="sample"):
        plan = diag.make_plan(data, features)
        return diag.sample_positions(plan, data, features, split, n, seed, self.root / directory)

    def rescore(self, source, sample, scores, *, nodes, statuses=None):
        """交換形式の明記されたバイト位置から、実装に依存せずMNRSを作る。"""
        count = len(scores)
        targets = sorted(row["index"] for row in sample["positions"] if row["file"] == str(source))
        header = bytearray(240)
        struct.pack_into("<4sI", header, 0, b"MNRS", 1)
        header[8:40] = hashlib.sha256(source.read_bytes()).digest()
        struct.pack_into("<Q", header, 40, count)
        header[48:80] = hashlib.sha256(b"".join(struct.pack("<Q", i) for i in targets)).digest()
        struct.pack_into("<Q", header, 80, len(targets))
        header[88:120] = bytes([9]) * 32
        struct.pack_into("<I", header, 120, nodes)
        header[124:156] = b"L0,P0,R1,E0".ljust(32, b"\0")
        struct.pack_into("<I", header, 156, 16)
        header[164:204] = b"a" * 40
        header[204:236] = bytes([7]) * 32
        body = bytearray()
        for i, score in enumerate(scores):
            status = (1 if statuses is None else statuses[i]) if i in targets else 0
            body.extend(struct.pack("<BBhIQ", status, 0, score if status == 1 else 0,
                                    4 if status == 1 else 0, nodes - 1 if status else 0))
        path = self.root / f"{source.name}.{nodes}.mnrs"
        path.write_bytes(header + body)
        return path

    def test_plan_boundaries_royals_exposure_and_exclusions(self):
        scores = [-501, -500, -101, -100, 99, 100, 499, 500]
        rows = [self.row(score=score) for score in scores]
        columns = [[6], [7], [8], [9], [10], [11], [], [0, 5, 12, 18]]
        rows += [self.row(stm=1, opponent=2), self.row(pieces=46), self.row(own=2),
                 self.row(game=self.validation), self.row(opponent=0), self.row(own=0)]
        columns += [[6]] * 6
        data, features = self.source(rows, columns)
        plan = diag.make_plan([data], [features])
        self.assertEqual(plan["exposed_total"], 7)
        self.assertEqual([plan["strata"][i]["counts"]["exposed"]["positions"] for i in range(5)], [1, 2, 2, 1, 0])
        self.assertEqual(plan["strata"][4]["counts"]["control"]["positions"], 1)
        self.assertEqual(plan["strata"][3]["counts"]["control"]["positions"], 1)
        self.assertAlmostEqual(plan["strata"][1]["w_h"], 2 / 7)
        self.assertEqual(plan["strata"][17]["counts"]["exposed"]["positions"], 1)
        self.assertEqual(plan["strata"][1]["counts"]["exposed"]["games"], 1)
        self.assertEqual(plan["files"][0]["eligible_count"], 9)
        self.assertEqual(plan["exclusions"], dict(validation=1, below_47_pieces=1,
                                                own_royals_not_one=2, opponent_royals_not_one_or_two=1))
        # 太子だけを持つ側も王駒1枚として対象になる。
        rows[0]["board"][0] = 51
        prince, prince_features = self.source(rows[:1], [[6]], name="prince.mnsd")
        self.assertEqual(diag.make_plan([prince], [prince_features])["exposed_total"], 1)

    def test_generation_uses_checksum_and_counts_games_across_files(self):
        paths = [self.source([self.row(score=i)], [[6]], name=f"g{i}.mnsd", network=network)
                 for i, network in enumerate([1, 2, 1, 3])]
        data, features = zip(*paths)
        plan = diag.make_plan(data, features)
        self.assertEqual([f["generation"] for f in plan["files"]], [0, 1, 0, 2])
        self.assertEqual(len(plan["strata"]), 60)
        self.assertEqual(plan["strata"][2]["counts"]["exposed"], dict(positions=2, games=1))
        self.assertEqual(plan["strata"][2]["w_h"], .5)

    def test_sample_reproducibility_splits_largest_remainder_and_shortages(self):
        rows, columns = [], []
        for game in (self.pilot[0], self.pilot[1], self.main[0]):
            for score, number in ((-600, 6), (0, 3), (600, 1)):
                for group_columns in ([6], []):
                    for _ in range(number):
                        rows.append(self.row(game=game, score=score))
                        columns.append(group_columns)
        data, features = self.source(rows, columns)
        first = self.sample([data], [features], n=7)
        second = self.sample([data], [features], n=7, directory="repeat")
        self.assertEqual(first, second)
        # 7*(.6,.3,.1)=(4.2,2.1,.7) -> (4,2,1)。
        self.assertEqual([first["allocations"][h]["requested_per_group"] for h in (0, 2, 4)], [4, 2, 1])
        target = self.root / "sample" / (data.name + ".targets.txt")
        indices = [row["index"] for row in first["positions"]]
        self.assertEqual(target.read_text(), "".join(f"{i}\n" for i in sorted(set(indices))))
        self.assertEqual(len(indices), 14)
        main = self.sample([data], [features], n=7, split="main", directory="main")
        self.assertTrue({r["game"] for r in first["positions"]}.isdisjoint({r["game"] for r in main["positions"]}))
        shortage = self.sample([data], [features], n=100, directory="short")
        self.assertEqual(shortage["selected_per_group"], 20)
        self.assertEqual(sum(a["shortage_per_group"] for a in shortage["allocations"]), 80)
        self.assertEqual(first["strata"], main["strata"])

    def test_shortage_preserves_equal_groups(self):
        data, features = self.source([self.row() for _ in range(5)], [[6], [6], [6], [6], []])
        sample = self.sample([data], [features], n=3)
        self.assertEqual(sample["selected_per_group"], 1)
        self.assertEqual(sample["allocations"][2]["shortage_per_group"], 2)

    def test_hand_computed_D_and_game_bootstrap_cross_groups_and_strata(self):
        # 各対局で両層・両群を持つ。群に共通の対局効果は常に相殺する。
        rows, columns, low, high = [], [], [], []
        for game, offset in zip(self.pilot[:3], [-1000, 0, 1000]):
            for score, delta, repeats in ((-600, -100, 1), (0, -20, 3)):
                for exposed in (True, False):
                    for _ in range(repeats):
                        rows.append(self.row(game=game, score=score))
                        columns.append([6] if exposed else [])
                        low.append(100)
                        high.append(100 + offset + (delta if exposed else 0))
        data, features = self.source(rows, columns)
        sample = self.sample([data], [features])
        shallow = self.rescore(data, sample, low, nodes=100_000)
        deep = self.rescore(data, sample, high, nodes=10_000_000)
        result = diag.estimate(sample, [shallow], [deep], [data], bootstrap=200)
        self.assertEqual(result, diag.estimate(sample, [shallow], [deep], [data], bootstrap=200))
        self.assertEqual(result["D"], -40)  # .25*(-100) + .75*(-20)
        self.assertAlmostEqual(result["standard_error"], 0)
        np.testing.assert_allclose(result["interval_two_sided_90"], [-40, -40], rtol=0, atol=1e-12)
        self.assertTrue(result["criterion_met"])
        self.assertEqual(result["n_pilot"], 12)
        self.assertEqual(result["n_main"], int(np.ceil(12 * (result["standard_error"] / 12) ** 2)))
        self.assertEqual(result["search"]["deep"]["nodes"]["min"], 9_999_999)
        self.assertEqual(result["search"]["shallow"]["depth"]["median"], 4)

    def test_mate_incomplete_exclusion_and_renormalization(self):
        rows = [self.row(score=score) for score in (-600, -600, 0, 0, 0, 0)]
        data, features = self.source(rows, [[6], [], [6], [], [6], []])
        sample = self.sample([data], [features])
        shallow = self.rescore(data, sample, [0, 0, 0, 0, -29000, 0], nodes=100_000)
        deep = self.rescore(data, sample, [29000, 0, -80, -20, 0, 0],
                            nodes=10_000_000, statuses=[1, 1, 1, 1, 1, 2])
        result = diag.estimate(sample, [shallow], [deep], [data], bootstrap=10)
        self.assertEqual(result["D"], -60)
        self.assertAlmostEqual(result["dropped_weight"], 1 / 3)
        self.assertEqual(result["strata"][2]["normalized_weight"], 1)
        self.assertEqual(result["groups"]["exposed"]["mate"]["win"]["either"]["fraction"], 1 / 3)
        self.assertEqual(result["groups"]["exposed"]["mate"]["loss"]["either"]["count"], 1)
        self.assertEqual(result["groups"]["control"]["incomplete"]["deep"], 1)
        self.assertEqual(result["groups"]["exposed"]["ordinary"], 1)

    def test_bootstrap_recomputes_missing_strata_and_nonzero_SE(self):
        rows = [self.row(game=game, score=score) for game, score in
                ((self.pilot[0], -600), (self.pilot[0], -600), (self.pilot[1], 0), (self.pilot[1], 0))]
        data, features = self.source(rows, [[6], [], [6], []])
        sample = self.sample([data], [features])
        shallow = self.rescore(data, sample, [0] * 4, nodes=100_000)
        deep = self.rescore(data, sample, [-100, 0, -20, 0], nodes=10_000_000)
        result = diag.estimate(sample, [shallow], [deep], [data], bootstrap=2000)
        self.assertEqual(result["D"], -60)
        self.assertEqual(result["interval_two_sided_90"], [-100, -20])
        self.assertFalse(result["criterion_met"])
        self.assertEqual(result["bootstrap"]["dropped_weight_max"], .5)
        # 対局2つを復元抽出: -100,-60,-20の確率は1/4,1/2,1/4。
        self.assertAlmostEqual(result["standard_error"], np.sqrt(800), delta=1.5)
        self.assertEqual(result["n_main"], int(np.ceil(2 * (result["standard_error"] / 12) ** 2)))

    def test_undefined_bootstrap_replicates_are_reported(self):
        data, features = self.source([self.row(game=g) for g in self.pilot[:2]], [[6], []])
        sample = self.sample([data], [features])
        shallow = self.rescore(data, sample, [0, 0], nodes=100_000)
        deep = self.rescore(data, sample, [-30, 0], nodes=10_000_000)
        result = diag.estimate(sample, [shallow], [deep], [data], bootstrap=200)
        self.assertGreater(result["bootstrap"]["undefined"], 0)
        self.assertEqual(result["bootstrap"]["valid"] + result["bootstrap"]["undefined"], 200)
        self.assertTrue(result["criterion_met"])  # 閾値ちょうど-30を含む。

    def test_input_contract_rejects_mismatches(self):
        data, features = self.source([self.row(), self.row()], [[6], []])
        sample = self.sample([data], [features])
        shallow = self.rescore(data, sample, [0, 0], nodes=100_000)
        deep = self.rescore(data, sample, [-50, 0], nodes=10_000_000)
        original = deep.read_bytes()
        # 元SHA、記録数、対象SHA、対象数、ノード数、条件、予約、状態。
        for offset in (8, 40, 48, 80, 120, 156, 160, 161, 240):
            with self.subTest(offset=offset):
                broken = bytearray(original)
                broken[offset] ^= 1
                deep.write_bytes(broken)
                with self.assertRaises(ValueError):
                    diag.estimate(sample, [shallow], [deep], [data], bootstrap=10)
        deep.write_bytes(original[:-1])
        with self.assertRaises(ValueError):
            diag.estimate(sample, [shallow], [deep], [data], bootstrap=10)
        deep.write_bytes(original)
        for field, value in (("game", 999), ("stratum", 0), ("data_seed", 0), ("index", -1)):
            bad = copy.deepcopy(sample)
            bad["positions"][0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                diag.estimate(bad, [shallow], [deep], [data], bootstrap=10)
        bad = copy.deepcopy(sample)
        bad["positions"].append(bad["positions"][0])
        with self.assertRaises(ValueError):
            diag.estimate(bad, [shallow], [deep], [data], bootstrap=10)
        raw = features.read_bytes()
        for offset in (8, 12, 16, 24):
            broken = bytearray(raw)
            broken[offset] ^= 1
            features.write_bytes(broken)
            with self.subTest(mnkf_offset=offset), self.assertRaises(ValueError):
                diag.make_plan([data], [features])
        features.write_bytes(raw)
        plan = diag.make_plan([data], [features])
        broken = bytearray(raw)
        broken[-1] = 1
        features.write_bytes(broken)
        with self.assertRaises(ValueError):
            diag.sample_positions(plan, [data], [features], "pilot", 1, 1, self.root / "bad")

    def test_estimate_multiple_files_main_and_file_order(self):
        paths = [self.source([self.row(game=self.main[0]), self.row(game=self.main[0])],
                             [[6], []], name=f"source{i}.mnsd", network=i + 1) for i in range(2)]
        data, features = zip(*paths)
        sample = self.sample(data, features, split="main")
        shallow = [self.rescore(path, sample, [0, 0], nodes=100_000) for path in data]
        deep = [self.rescore(path, sample, [-40 - i * 40, 0], nodes=10_000_000)
                for i, path in enumerate(data)]
        result = diag.estimate(sample, shallow, deep, data, bootstrap=20)
        self.assertEqual(result["D"], -60)
        self.assertEqual(result["bootstrap"]["games"], 1)
        self.assertEqual(result["standard_error"], 0)
        self.assertNotIn("n_main", result)
        with self.assertRaises(ValueError):
            diag.estimate(sample, shallow, list(reversed(deep)), data, bootstrap=20)

    def test_ordinary_boundary_tactical_moves_and_no_estimable_stratum(self):
        data, features = self.source([self.row(), self.row()], [[6], []])
        sample = self.sample([data], [features])
        shallow = self.rescore(data, sample, [28999, -28999], nodes=100_000)
        deep = self.rescore(data, sample, [28959, -28999], nodes=10_000_000)
        raw = bytearray(deep.read_bytes())
        raw[241] = 1  # 捕獲・成りという理由で診断Aから除外してはならない。
        deep.write_bytes(raw)
        self.assertEqual(diag.estimate(sample, [shallow], [deep], [data], bootstrap=20)["D"], -40)
        deep = self.rescore(data, sample, [29000, -29000], nodes=10_000_000)
        with self.assertRaises(ValueError):
            diag.estimate(sample, [shallow], [deep], [data], bootstrap=20)

    def test_cli_plan_sample_estimate(self):
        data, features = self.source([self.row(), self.row()], [[6], []])
        plan_path, result_path = self.root / "plan.json", self.root / "result.json"
        diag.main(["plan", "--data", str(data), "--king-features", str(features), "--output", str(plan_path)])
        diag.main(["sample", "--plan", str(plan_path), "--data", str(data), "--king-features", str(features),
                   "--split", "pilot", "--per-group", "1", "--output-dir", str(self.root / "cli")])
        sample_path = self.root / "cli" / "sample.json"
        sample = json.loads(sample_path.read_text())
        shallow = self.rescore(data, sample, [0, 0], nodes=100_000)
        deep = self.rescore(data, sample, [-50, 0], nodes=10_000_000)
        diag.main(["estimate", "--sample", str(sample_path), "--data", str(data), "--shallow", str(shallow),
                   "--deep", str(deep), "--bootstrap", "10", "--output", str(result_path)])
        self.assertEqual(json.loads(result_path.read_text())["D"], -50)


if __name__ == "__main__":
    unittest.main()
