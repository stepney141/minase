"""事前登録の表と式から手で定めた合成例による検証。

実行: python3 -m unittest discover -s docs/measurements/qsearch-output-search-trace -v
全テストが合成入力だけを使い、エンジンや測定データを読み出さない。

仕様と入力区分:
* 乖離の3式: 各上下界の両方向、m未満、mと等しい場合を検査する。
* 最後の訪問: 窓の再探索、3回の子探索、古い親の子、祖先の下限を検査する。
* 停止表: 各 outcome、LMR の採用と再探索、子の乖離の有無を検査する。
* 機構集合: 根からDまでの機構と、Dより下の機構の除外を検査する。
* 分類表: 単独1件、単独2件、同時だけ、特定不能と件数を検査する。
* 入出力契約: mateの符号、前提条件の不一致、履歴を含むキャッシュ、
  USIの固定条件、介入の厳密不等号を合成入力で検査する。

SPEC_UNCLEAR: N1が存在しない根の停止はD=0とする。追跡外の単独成功が
ある場合、同時成功による分類に進まない。解釈は trace_roots.py 冒頭参照。
"""

from contextlib import ExitStack, redirect_stdout
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import Mock, patch

import trace_roots as trace


def search(reduction=0, score=0, alpha=-1000, beta=1000):
    return {"depth": 3, "reduction": reduction, "score": score, "alpha": alpha, "beta": beta}


def node(seq, ply, outcome="path_end", result=0, **changes):
    event = {"type": "node", "iter": 6, "pass": 0, "seq": seq, "ply": ply,
             "kind": "root" if ply == 0 else "main", "depth_req": 4,
             "depth_after_iir": 4, "alpha": -1000, "beta": 1000,
             "null": None, "result": result, "bound": "exact", "aborted": False,
             "path_move": "1a1b" if outcome != "path_end" else None,
             "path_move_outcome": outcome,
             "path_move_searches": [search()] if outcome == "searched" else []}
    event.update(changes)
    return event


def window(number=0, aborted=False):
    return {"type": "pass", "iter": 6, "pass": number, "aborted": aborted,
            "alpha": -1000, "beta": 1000, "best_move": "1a1b", "score": 0}


def events(*nodes):
    """JSONL を経由し、実際の入出力形式と同じイベント列を作る。"""
    lines = [json.dumps(n) for n in nodes] + [json.dumps(window())]
    return [json.loads(line) for line in lines]


def analyze(*nodes, value=200):
    return trace.analyze_trace(events(*nodes), 6, value)


class DivergenceTests(unittest.TestCase):
    def test_upper_bound(self):
        n = {"result": 100, "alpha": 100, "beta": 200}
        self.assertTrue(trace.diverges(n, 242.1))
        self.assertFalse(trace.diverges(n, 242.09))
        self.assertFalse(trace.diverges(n, -1000))

    def test_lower_bound(self):
        n = {"result": 100, "alpha": 0, "beta": 100}
        self.assertTrue(trace.diverges(n, -42.1))
        self.assertFalse(trace.diverges(n, -42.09))
        self.assertFalse(trace.diverges(n, 1000))

    def test_exact_value_both_directions(self):
        n = {"result": 100, "alpha": 0, "beta": 200}
        for t in (242.1, -42.1):
            with self.subTest(t=t):
                self.assertTrue(trace.diverges(n, t))
        for t in (242.09, -42.09, 100):
            with self.subTest(t=t):
                self.assertFalse(trace.diverges(n, t))

    def test_window_boundaries(self):
        self.assertEqual(trace.bound_kind(100, 100, 200), "upper")
        self.assertEqual(trace.bound_kind(200, 100, 200), "lower")
        self.assertEqual(trace.bound_kind(150, 100, 200), "exact")
        with self.assertRaises(ValueError):
            trace.bound_kind(1, 2, 2)


class VisitTests(unittest.TestCase):
    def test_last_completed_window_only(self):
        old = node(1, 1, "qs_see")
        old_root = node(2, 0, "searched")
        chosen = node(3, 1, "qs_not_generated", **{"pass": 1})
        root = node(4, 0, "searched", **{"pass": 1})
        aborted = node(5, 1, "aborted", aborted=True, **{"pass": 2})
        a = trace.analyze_trace([old, old_root, window(), chosen, root,
                                 window(1), aborted, window(2, True)], 6, 200)
        self.assertEqual(a["pass"]["pass"], 1)
        self.assertEqual(a["node"]["seq"], 3)
        self.assertEqual(a["mechanisms"], ["aspiration"])

    def test_nested_reduced_zero_full_zero_full_window_visits(self):
        # N1を3回訪問。最後のN1にあるN2も3回訪問し、その最後だけが採用される。
        ns = [node(1, 2, "qs_see"), node(2, 1, "searched"),
              node(3, 2, "qs_delta_move"), node(4, 1, "searched"),
              node(5, 2, "qs_see"), node(6, 2, "qs_delta_move"),
              node(7, 2, "qs_not_generated"),
              node(8, 1, "searched", path_move_searches=[search(2), search(0), search(0)]),
              node(9, 0, "searched", path_move_searches=[search(2), search(0), search(0)])]
        a = analyze(*ns)
        self.assertEqual([n["seq"] for n in a["visits"]], [9, 8, 7])
        self.assertEqual(a["D"], 2)
        self.assertEqual(a["reason"], "地平線")
        self.assertEqual(a["mechanisms"], [])

    def test_old_parent_child_is_not_reused(self):
        a = analyze(node(1, 2, "qs_see"), node(2, 1, "searched"),
                    node(3, 1, "tt_cutoff"), node(4, 0, "searched"))
        self.assertEqual([n["seq"] for n in a["visits"]], [4, 3])
        self.assertEqual(a["mechanisms"], ["tt_cutoff"])

    def test_ancestor_lower_bound_excludes_stale_grandchild(self):
        ns = [node(1, 3, "qs_see"), node(2, 2, "searched"), node(3, 1, "searched"),
              node(4, 2, "tt_cutoff"), node(5, 1, "searched"), node(6, 0, "searched")]
        self.assertEqual([n["seq"] for n in trace.last_visits(ns)], [6, 5, 4])

    def test_missing_child_after_new_parent_is_error(self):
        with self.assertRaises(ValueError):
            analyze(node(1, 2), node(2, 1, "searched"),
                    node(3, 1, "searched"), node(4, 0, "searched"))

    def test_invalid_outcomes_in_final_window_fail_even_on_old_visit(self):
        for outcome in ("not_reached", "aborted"):
            with self.subTest(outcome=outcome), self.assertRaises(ValueError):
                analyze(node(1, 1, outcome), node(2, 1), node(3, 0, "searched"))

    def test_invalid_bound_or_duplicate_root_is_error(self):
        for ns in ([node(1, 1, bound="upper"), node(2, 0, "searched")],
                   [node(1, 0), node(2, 0)]):
            with self.subTest(ns=ns), self.assertRaises(ValueError):
                analyze(*ns)


class StopTests(unittest.TestCase):
    def test_all_path_mechanisms(self):
        cases = {"tt_cutoff": ["tt_cutoff"], "null_cutoff": ["null_move"],
                 "futility": ["futility"], "see": ["see"], "qs_stand_pat": [],
                 "qs_delta_node": ["qs_delta"], "qs_delta_move": ["qs_delta"],
                 "qs_see": ["qs_see"], "qs_tt_cutoff": ["qs_tt"]}
        for outcome, mechanisms in cases.items():
            with self.subTest(outcome=outcome):
                a = analyze(node(1, 1, outcome), node(2, 0, "searched"))
                self.assertEqual((a["D"], a["reason"], a["outcome"]), (1, "経路上の機構", outcome))
                self.assertEqual(a["mechanisms"], mechanisms)

    def test_other_stop_reasons(self):
        cases = {"sibling_beta_cutoff": "兄弟手による打ち切り",
                 "qs_not_generated": "地平線", "max_ply": "地平線",
                 "path_end": "読み筋の末端", "repetition": "反復",
                 "last_royal_capture": "王駒の捕獲"}
        for outcome, reason in cases.items():
            with self.subTest(outcome=outcome):
                self.assertEqual(analyze(node(1, 1, outcome), node(2, 0, "searched"))["reason"], reason)

    def test_reduction_takes_precedence_over_child_divergence(self):
        for child_score in (0, 200):
            with self.subTest(child_score=child_score):
                a = analyze(node(1, 2, result=child_score),
                            node(2, 1, "searched", path_move_searches=[search(1)]),
                            node(3, 0, "searched"))
                self.assertEqual((a["D"], a["reason"]), (1, "減深"))
                self.assertEqual(a["mechanisms"], ["lmr"])

    def test_full_depth_research_overrides_earlier_reduction(self):
        a = analyze(node(1, 2, "qs_see"),
                    node(2, 1, "searched", path_move_searches=[search(2), search(0)]),
                    node(3, 0, "searched"))
        self.assertEqual(a["D"], 2)
        self.assertEqual(a["mechanisms"], ["qs_see"])

    def test_sibling_error_when_child_is_not_divergent(self):
        a = analyze(node(1, 2, result=200), node(2, 1, "searched"), node(3, 0, "searched"))
        self.assertEqual((a["D"], a["reason"], a["t_D"]), (1, "兄弟手の誤り", -200))

    def test_no_divergence_at_first_child(self):
        a = analyze(node(1, 1, result=-200), node(2, 0, "searched", depth_after_iir=3))
        self.assertIsNone(a["D"])
        self.assertEqual(a["reason"], "乖離なし")
        self.assertEqual(a["mechanisms"], [])

    def test_path_not_searched_at_root_stops_at_zero(self):
        a = analyze(node(1, 0, "sibling_beta_cutoff"))
        self.assertEqual((a["D"], a["reason"], a["t_D"]), (0, "兄弟手による打ち切り", 200))

    def test_mechanisms_include_ancestors_and_stop_but_not_descendants(self):
        ns = [node(1, 2, "qs_see", depth_after_iir=1),
              node(2, 1, "searched", path_move_searches=[search(2)],
                   null={"tried": True, "reduction": 2, "score": 0, "cutoff": False}),
              node(3, 0, "searched", depth_after_iir=3)]
        for n in ns:
            n["pass"] = 1
        a = trace.analyze_trace([window(), *ns, window(1)], 6, 200)
        self.assertEqual(a["mechanisms"], ["aspiration", "iir", "lmr", "null_move"])

    def test_confirmation_uses_node_perspective_and_clears_unconfirmed(self):
        a = analyze(node(1, 1, "qs_see"), node(2, 0, "searched"))
        confirmed = trace.confirm_divergence(a, -180)
        self.assertEqual(confirmed["mechanisms"], ["qs_see"])
        self.assertEqual(confirmed["t_prime_D"], -180)
        unconfirmed = trace.confirm_divergence(a, -100)
        self.assertEqual(unconfirmed["reason"], "乖離点の未確認")
        self.assertEqual(unconfirmed["mechanisms"], [])
        self.assertEqual(a["reason"], "経路上の機構")


class ClassificationTests(unittest.TestCase):
    def classify(self, involved, successful, combined=False):
        singles = {m: {"success": m in successful} for m in trace.MECHANISMS}
        return trace.classify(involved, singles, {"success": combined})

    def test_one_supported_success_and_unrelated_success(self):
        c = self.classify(["lmr"], ["lmr", "correction"], True)
        self.assertEqual(c["category"], "1機構の特定")
        self.assertEqual(c["supported"], ["lmr"])
        self.assertEqual(c["intervention_only"], ["correction"])

    def test_two_supported_successes(self):
        c = self.classify(["lmr", "qs_see"], ["lmr", "qs_see"])
        self.assertEqual(c["category"], "複数機構の疑い")
        self.assertEqual(c["supported"], ["lmr", "qs_see"])

    def test_combined_only_success(self):
        c = self.classify(["lmr", "qs_delta"], [], True)
        self.assertEqual(c["category"], "複数機構の疑い")
        self.assertEqual(c["supported"], [])

    def test_unidentified_including_intervention_only(self):
        for involved, successes, combined in ((["lmr"], [], False),
                                               ([], ["correction"], False),
                                               (["lmr"], ["correction"], True)):
            with self.subTest(involved=involved, successes=successes):
                c = self.classify(involved, successes, combined)
                self.assertEqual(c["category"], "特定できない")

    def test_counts_exclude_combined_and_unrelated_success(self):
        cs = [self.classify(["lmr"], ["lmr", "correction"]),
              self.classify(["lmr", "qs_see"], ["lmr", "qs_see"]),
              self.classify(["lmr"], ["lmr"]),
              self.classify(["qs_delta", "iir"], [], True),
              self.classify([], ["correction"])]
        a = trace.aggregate(cs)
        self.assertEqual(a["counts"]["lmr"], 3)
        self.assertEqual(a["counts"]["qs_see"], 1)
        for m in ("correction", "qs_delta", "iir"):
            self.assertEqual(a["counts"][m], 0)
        self.assertEqual(a["three_or_more"], ["lmr"])
        self.assertTrue(a["qsearch_note_required"])
        self.assertFalse(trace.aggregate(cs[:1])["qsearch_note_required"])


class InputOutputTests(unittest.TestCase):
    def test_mate_score_conversion(self):
        for text, expected in (("3", 29997), ("-2", -29998), ("-0", -30000)):
            with self.subTest(text=text):
                self.assertEqual(trace.score_value("mate", text), expected)

    def test_last_search_info(self):
        lines = ["info depth 2 score cp 45 nodes 500 pv 1a1b",
                 "info depth 6 score mate -0 nodes 100000 pv 2a2b 3a3b",
                 "info string done", "bestmove 2a2b ponder 3a3b"]
        self.assertEqual(trace.parse_search(lines), {
            "depth": 6, "nodes": 100000, "score": -30000,
            "score_usi": ["mate", "-0"], "pv": ["2a2b", "3a3b"], "bestmove": "2a2b"})

    def test_prerequisite_a_checks_both_value_and_pv(self):
        root = {"id": "synthetic", "reference": {"lines": {
            "1a1b": {"score_root_view": -500, "pv": ["2a2b"]}}}}
        trace.check_child(root, "1a1b", {"score": 500, "pv": ["2a2b"]})
        for result in ({"score": -500, "pv": ["2a2b"]}, {"score": 500, "pv": ["3a3b"]}):
            with self.subTest(result=result), self.assertRaises(ValueError):
                trace.check_child(root, "1a1b", result)

    def test_prerequisite_b_checks_move_depth_and_score(self):
        root = {"id": "synthetic", "proposal": {
            "best_move": "1a1b", "depth": 6, "score": {"kind": "cp", "value": -632}}}
        result = {"bestmove": "1a1b", "depth": 6, "score": -632}
        trace.check_proposal(root, result)
        for field, value in (("bestmove", "2a2b"), ("depth", 5), ("score", -633)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                trace.check_proposal(root, dict(result, **{field: value}))

    def test_root_values_use_last_search_parent_window(self):
        es = events(node(1, 1), node(2, 0, "searched", path_move_searches=[
            search(1, -10), search(0, 200, 100, 200)]))
        r = trace.root_move_value(es, 6)
        self.assertEqual(r["last_search"]["score"], 200)
        self.assertEqual(r["bound"], "lower")
        absent = trace.root_move_value(events(node(1, 0, "sibling_beta_cutoff")), 6)
        self.assertIsNone(absent["last_search"])
        self.assertIsNone(absent["bound"])

    def test_loss_threshold_is_strict_and_negative_is_success(self):
        pair = {p: {"search": {"bestmove": "1a1b"}, "events": [], "file": "synthetic"}
                for p in ("s", "a")}
        for loss, expected in ((142.1, False), (142.09, True), (-10, True)):
            with self.subTest(loss=loss), patch.object(trace, "trace_pair", return_value=pair), \
                    patch.object(trace, "move_loss", return_value=loss), \
                    patch.object(trace, "root_move_value", return_value={}):
                r = trace.intervention({"d0": 6}, {}, None, ("lmr",), "lmr")
                self.assertEqual(r["success"], expected)

    def test_cache_uses_history_and_rejects_other_binary(self):
        with tempfile.TemporaryDirectory(dir=Path(__file__).parent) as directory:
            path = Path(directory) / "searches.json"
            with patch.object(trace, "run_search", return_value={"score": 50}) as run:
                cache = trace.SearchCache(path, "synthetic_hash")
                cache.search(["1a1b", "2a2b"])
                cache.search(["1a1b", "2a2b"])
                self.assertEqual(run.call_count, 1)
                cache.search(["2a2b", "1a1b"])
                self.assertEqual(run.call_count, 2)
                cache.search(["1a1b", "2a2b"], fresh=True)
                self.assertEqual(run.call_count, 3)
                reloaded = trace.SearchCache(path, "synthetic_hash")
                self.assertEqual(reloaded.search(["1a1b", "2a2b"]), {"score": 50})
                self.assertEqual(run.call_count, 3)
            with self.assertRaises(ValueError):
                trace.SearchCache(path, "different_hash")

    def test_usi_fixed_conditions_fresh_process_and_options(self):
        replies = ("option name TracePath type string default\n"
                   "option name TraceOut type string default\n"
                   "option name TraceDisable type string default\nusiok\nreadyok\n"
                   "info depth 6 score cp 50 nodes 4321 pv 1a1b\nbestmove 1a1b\n")
        processes = []

        def make_process(*args, **kwargs):
            p = Mock(stdin=io.StringIO(), stdout=io.StringIO(replies), returncode=0)
            context = Mock()
            context.__enter__ = Mock(return_value=p)
            context.__exit__ = Mock(return_value=False)
            processes.append(p)
            return context

        with tempfile.TemporaryDirectory(dir=Path(__file__).parent) as directory, \
                patch.object(trace.subprocess, "Popen", side_effect=make_process) as popen:
            out = Path(directory) / "trace.jsonl"
            out.write_text("old search\n")
            trace.run_search(["1a1b"], "depth 6", ("lmr", "iir"), ["2a2b"], out)
            trace.run_search(["1a1b"], "nodes 100000", ("lmr",))
            self.assertEqual(popen.call_count, 2)
            self.assertEqual(popen.call_args.args[0][1:], ["--protocol", "usi", "--rules", "engine-default"])
            commands = processes[0].stdin.getvalue().splitlines()
            self.assertEqual(commands, [
                "usi", "setoption name USI_Hash value 64", "setoption name Threads value 1",
                "isready", "usinewgame", "position startpos moves 1a1b",
                "setoption name TracePath value 2a2b", f"setoption name TraceOut value {out}",
                "setoption name TraceDisable value lmr,iir", "go depth 6", "quit"])
            self.assertEqual(out.read_text(), "")
            self.assertIn("setoption name TraceDisable value lmr\ngo nodes 100000", processes[1].stdin.getvalue())
            self.assertNotIn("TracePath", processes[1].stdin.getvalue())


class DriverTests(unittest.TestCase):
    """探索の代わりに固定の合成応答を返し、手順全体を検査する。"""

    def run_driver(self, directory, failure=None):
        calls = []
        output = Path(directory)
        root = {"id": "synthetic", "prefix": ["opening"], "a": "a", "s": "s", "d0": 6,
                "proposal": {"best_move": "s", "depth": 6, "score": {"kind": "cp", "value": 0}},
                "values": {"a": 200, "s": 0}, "reference": {"lines": {
                    "a": {"score_root_view": 200, "pv": ["next"]},
                    "s": {"score_root_view": 0, "pv": ["next"]}}}}

        def fake_search(moves, limit, disabled=(), path=None, trace_out=None):
            calls.append((list(moves), limit, tuple(disabled), path))
            if limit == "nodes 10000000":
                score = -200 if moves[-1] == "a" else 0
                return {"score": score, "pv": ["wrong" if failure == "A" else "next"],
                        "depth": 14, "nodes": 10000000, "bestmove": "next"}
            move = "a" if limit == "depth 6" and set(disabled) & {"qs_see", "correction"} else "s"
            result = {"score": 0, "pv": [move], "depth": 5 if failure == "B" else 6,
                      "nodes": 100000, "bestmove": move}
            if path is not None:
                start = {"type": "search_start", "seq": 0, "path": path, "disabled": list(disabled),
                         "depth_limit": 6, "node_limit": None}
                es = [start, node(1, 1, "qs_see", path_move="next"),
                      node(2, 0, "searched", path_move=path[0]), dict(window(), best_move=move)]
                trace_out.parent.mkdir(exist_ok=True)
                trace_out.write_text("".join(json.dumps(e) + "\n" for e in es))
            return result

        with ExitStack() as stack:
            stack.enter_context(patch.object(trace, "OUT", output))
            stack.enter_context(patch.object(trace, "BIN", Path(__file__)))
            stack.enter_context(patch.object(trace, "REFERENCE", Path(__file__)))
            stack.enter_context(patch.object(trace, "load_roots", return_value=[root]))
            stack.enter_context(patch.object(trace, "run_search", side_effect=fake_search))
            stack.enter_context(patch("sys.argv", ["trace_roots.py", "--jobs", "2"]))
            stdout = stack.enter_context(redirect_stdout(io.StringIO()))
            if failure:
                with self.assertRaises(ValueError):
                    trace.main()
                self.assertFalse((output / "result.json").exists())
            else:
                trace.main()
        return calls, stdout.getvalue()

    def test_complete_flow_and_fixed_nodes_do_not_change_classification(self):
        with tempfile.TemporaryDirectory(dir=Path(__file__).parent) as directory:
            calls, stdout = self.run_driver(directory)
            output = Path(directory)
            result = json.loads((output / "result.json").read_text())
            root = result["roots"][0]
            self.assertEqual(root["classification"]["category"], "1機構の特定")
            self.assertEqual(root["classification"]["supported"], ["qs_see"])
            self.assertEqual(root["classification"]["intervention_only"], ["correction"])
            self.assertEqual(root["analyses"]["a"]["t_prime_D"], -200)
            self.assertEqual(root["analyses"]["s"]["reason"], "乖離なし")
            self.assertEqual(root["combined"]["disabled"], ["qs_see"])
            self.assertEqual(root["fixed_nodes"]["qs_see"]["loss"], 200)
            self.assertEqual(root["fixed_nodes"]["correction"]["loss"], 200)
            self.assertEqual(result["aggregate"]["counts"]["qs_see"], 1)
            self.assertEqual(result["aggregate"]["counts"]["correction"], 0)
            self.assertEqual(len(root["interventions"]), 11)
            self.assertEqual(len(list((output / "traces").glob("*.jsonl"))), 26)
            self.assertEqual([c[1] for c in calls[:2]], ["nodes 10000000"] * 2)
            self.assertEqual({c[1] for c in calls[2:4]}, {"nodes 100000", "depth 6"})
            self.assertTrue(all(c[3] is None for c in calls[:4]))
            # D=1の確認に前提条件Aのキャッシュを使い、追加の教師探索をしない。
            self.assertEqual(sum(c[1] == "nodes 10000000" for c in calls), 2)
            self.assertIn(trace.sha256(output / "result.json"), stdout)
            summary = (output / "summary.md").read_text()
            self.assertIn("N_D手番側", summary)
            self.assertIn("根手番側", summary)
            self.assertIn("注記する必要がある", summary)

    def test_prerequisite_failure_prevents_tracing(self):
        for failure in ("A", "B"):
            with self.subTest(failure=failure), \
                    tempfile.TemporaryDirectory(dir=Path(__file__).parent) as directory:
                calls, _ = self.run_driver(directory, failure)
                self.assertTrue(all(c[3] is None and not c[2] for c in calls))
                if failure == "A":
                    self.assertTrue(all(c[1] == "nodes 10000000" for c in calls))


if __name__ == "__main__":
    unittest.main()
