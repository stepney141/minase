"""固定抽出の制約と、親視点の教師差を手計算の契約から検証する。"""
import unittest

from fm_teacher_diagnostics import paired_sample, parent_metrics, summarize_parents, budget_stability, validate_result_inputs


def search(value, kind="cp", best_move=None):
    return {"score": {"kind": kind, "value": value}, "best_move": best_move}


def row(scores):
    return {"proposals": {"fm": search(0, best_move="fm"), "pst": search(0, best_move="pst")},
            "candidates": ["fm", "pst", "teacher"],
            "children": [{"move": move, "budget": budget, "search": search(value)}
                         for budget, values in scores.items()
                         for move, value in zip(("fm", "pst", "teacher"), values)]}


class SamplingTests(unittest.TestCase):
    def test_seed_pair_strata_unique_records_and_two_per_game(self):
        saved = [{"source_index": i, "stratum": [i % 2, 0], "game_key": f"file{i // 4}:game1"}
                 for i in range(24)]
        match = [{"source_index": i, "stratum": [i % 2, 0], "game_key": f"game{i // 4}"}
                 for i in range(16)]
        pairs = paired_sample(saved, match, 8, 1)
        self.assertEqual(pairs, paired_sample(saved, match, 8, 1))
        self.assertEqual(len(pairs), 8)
        self.assertTrue(all(left["stratum"] == right["stratum"] for left, right in pairs))
        for side in (0, 1):
            selected = [pair[side] for pair in pairs]
            self.assertEqual(len({item["source_index"] for item in selected}), 8)
            for game in {item["game_key"] for item in selected}:
                self.assertLessEqual(sum(item["game_key"] == game for item in selected), 2)
        self.assertEqual(len({right["game_key"] for _, right in pairs[:4]}), 4)

    def test_constraints_take_precedence_over_requested_count(self):
        saved = [{"source_index": i, "stratum": [0, 0], "game_key": "same-game"} for i in range(10)]
        match = [{"source_index": i, "stratum": [0, 0], "game_key": str(i)} for i in range(10)]
        self.assertEqual(len(paired_sample(saved, match, 10, 1)), 2)
        for item in match:
            item["stratum"] = [1, 0]
        self.assertEqual(paired_sample(saved, match, 10, 1), [])


class MetricTests(unittest.TestCase):
    def test_parent_scores_are_not_negated_twice_and_gap_is_loss_difference(self):
        # 教師の親視点: FM=-20、PST=50、集合内最善80。損失は100と30、差70。
        metric = parent_metrics(row({100: [-20, 50, 80]}), 100)
        self.assertEqual(metric["gap"], 70)
        self.assertEqual(metric["candidate_set_losses"], {"fm": 100, "pst": 30})

    def test_mate_is_excluded_from_cp_gap(self):
        data = row({100: [-20, 50, 80]})
        data["children"][0]["search"] = search(29999, "mate")
        metric = parent_metrics(data, 100)
        self.assertIsNone(metric["gap"])
        self.assertEqual(metric["child_score_kinds"], {"mate": 1, "cp": 2})

    def test_incomplete_proposal_is_not_a_valid_choice(self):
        data = row({100: [-20, 50, 80]})
        data["proposals"]["fm"] = search(0, "incomplete", "fm")
        self.assertFalse(parent_metrics(data, 100)["proposal_valid"])
        self.assertIsNone(parent_metrics(data, 100)["gap"])

    def test_same_proposal_has_zero_gap(self):
        data = row({100: [-20, 50, 80]})
        data["proposals"]["pst"]["best_move"] = "fm"
        metric = parent_metrics(data, 100)
        self.assertTrue(metric["proposal_same"])
        self.assertEqual(metric["gap"], 0)

    def test_game_equal_weight_differs_from_parent_equal_weight(self):
        # game Aの2親は0、game Bの1親は90。親平均30、対局平均45。
        items = [{"game_key": game, "gap": gap, "proposal_valid": True,
                  "proposal_same": False, "child_score_kinds": {"cp": 3}}
                 for game, gap in (("A", 0), ("A", 0), ("B", 90))]
        report = summarize_parents(items)
        self.assertEqual(report["parent_equal_gap"]["mean"], 30)
        self.assertEqual(report["game_equal_gap"]["mean"], 45)

    def test_budget_order_and_gap_sign_change(self):
        # 小予算ではPST>FM、大予算ではFM>PST。teacherの最上位は不変。
        report = budget_stability(row({100: [10, 20, 30], 1000: [25, 20, 30]}), 100, 1000)
        self.assertFalse(report["gap_sign_stable"])
        self.assertTrue(report["strict_gap_reversal"])
        self.assertFalse(report["zero_nonzero_change"])
        self.assertFalse(report["top_set_changed"])
        self.assertEqual(report["gap_change"], -15)
        self.assertFalse(report["all_candidate_ranks_stable"])
        self.assertEqual(report["rank_pair_agree"], 2)
        self.assertEqual(report["rank_pair_count"], 3)

    def test_ties_distinguish_zero_transition_from_strict_reversal(self):
        report = budget_stability(row({100: [20, 20, 20], 1000: [10, 20, 30]}), 100, 1000)
        self.assertFalse(report["strict_gap_reversal"])
        self.assertTrue(report["zero_nonzero_change"])
        self.assertTrue(report["top_set_changed"])
        self.assertEqual(report["top_sets"], [["fm", "pst", "teacher"], ["teacher"]])

    def test_every_result_matches_weights_and_selected_input(self):
        manifest = {"teacher": {"candidate_sha256": "fm", "teacher_sha256": "pst"},
                    "files": {"pilot.mnsd": "pilot", "positions.mnsd": "full"}}
        metadata = {"candidate_sha256": "fm", "teacher_sha256": "pst", "positions_sha256": "pilot"}
        validate_result_inputs(manifest, [{"metadata": metadata}], "pilot")
        for key in metadata:
            wrong = dict(metadata)
            wrong[key] = "different"
            with self.assertRaises(ValueError):
                validate_result_inputs(manifest, [{"metadata": metadata}, {"metadata": wrong}], "pilot")
        with self.assertRaises(ValueError):
            validate_result_inputs(manifest, [{"metadata": metadata}], "full")

    def test_pawn_sized_difference_includes_both_100cp_boundaries(self):
        items = [{"game_key": str(i), "gap": gap, "proposal_valid": True,
                  "proposal_same": False, "child_score_kinds": {"cp": 3}}
                 for i, gap in enumerate((-101, -100, -99, 99, 100, 101))]
        report = summarize_parents(items)
        self.assertEqual(report["gap_at_least_100cp_count"], 2)
        self.assertEqual(report["gap_at_most_minus100cp_count"], 2)


if __name__ == "__main__":
    unittest.main()
