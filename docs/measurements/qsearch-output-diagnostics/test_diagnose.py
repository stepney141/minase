"""設計書の境界と指示書の受入条件。期待値は手計算した値を使う。"""

import copy
import io
import json
import math
from pathlib import Path
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import diagnose as d


def search(value, move='a', kind='cp'):
    return dict(best_move=move, score=dict(kind=kind, value=value))


class ClassificationTests(unittest.TestCase):
    def test_exact_margin_and_budget_difference(self):
        # mを含む支持、m未満だけの小差、予算差mを含む支持。
        for pair, expected in [
            ([142.1, 142.1], 'supports_s0'),
            ([142.1, 284.2], 'supports_s0'),
            ([-142.1, -284.2], 'supports_candidate'),
            ([142.1, 284.2001], 'unstable'),
            ([-142.1, -284.2001], 'unstable'),
            ([142.0999, -142.0999], 'small'),
            ([142.0999, 142.1], 'unstable'),
            ([142.1, -142.1], 'unstable'),
            ([0, 0], 'small'), (None, 'incomparable'),
        ]:
            with self.subTest(pair=pair):
                self.assertEqual(d.classify(pair), expected)

    def test_candidate_set_losses_and_identical_moves(self):
        proposals = {'S0': search(0, 'a'), 'Q': search(0, 'b'), 'Qc': search(0, 'a')}
        children = {'a': [search(100), search(120)], 'b': [search(0), search(20)],
                    'teacher': [search(140), search(160)]}
        result = d.decision(proposals, children)
        self.assertEqual(result['loss'], {'S0': 40, 'Q': 140, 'Qc': 40})
        self.assertEqual(result['category'], 'stable')
        proposals['Q'] = search(0, 'teacher')
        self.assertEqual(d.decision(proposals, children)['loss']['Q'], 0)
        children['b'][1] = search(-400)
        self.assertEqual(d.decision(proposals, children)['category'], 'unstable')

    def test_non_cp_children_are_incomparable_even_with_one_candidate(self):
        for kind in ('mate', 'terminal', 'incomplete'):
            with self.subTest(kind=kind):
                result = d.decision({'S0': search(0)}, {'a': [search(10), search(0, kind=kind)]})
                self.assertEqual(result['category'], 'incomparable')
                self.assertIsNone(result['loss'])


class StatisticsTests(unittest.TestCase):
    def test_group_sums_and_root_weighted_mean(self):
        # A=(1,3), B=(8): mean=4, centered sums=(-4,4).
        # cluster SE=sqrt(2*(16+16))/3=8/3; root SE=sqrt(13/3).
        result = d.paired_metric([('A', 1), ('A', 3), ('B', 8)])
        self.assertEqual(result['mean_cp'], 4)
        self.assertAlmostEqual(result['game_se_cp'], 8 / 3)
        self.assertAlmostEqual(result['independent_root_se_cp'], math.sqrt(13 / 3))
        self.assertEqual((result['roots'], result['games']), (3, 2))

    def test_150_roots_and_strict_lower_bound(self):
        insufficient = d.paired_metric([(i, 10) for i in range(149)])
        enough = d.paired_metric([(i, 10) for i in range(150)])
        self.assertEqual(insufficient['lower_cp'], 10)
        self.assertFalse(insufficient['eligible'])
        self.assertFalse(insufficient['significantly_positive'])
        self.assertTrue(enough['eligible'])
        self.assertTrue(enough['significantly_positive'])
        zero = d.paired_metric([(i, 0) for i in range(150)])
        self.assertFalse(zero['significantly_positive'])
        one_game = d.paired_metric([('same', 10)] * 150)
        self.assertIsNone(one_game['game_se_cp'])
        self.assertFalse(one_game['eligible'])

    def test_empty_metric(self):
        result = d.paired_metric([])
        self.assertIsNone(result['mean_cp'])
        self.assertFalse(result['eligible'])


class PerspectiveTests(unittest.TestCase):
    def test_mate_distance_including_negative_zero(self):
        # search.md: mate distance omits the two moves after mate; MAX_PLY=256.
        # Signed zero loses the exact distance and uses the documented ±MATE representative.
        for raw, expected in [
            ('1', 29997), ('-1', -29997), ('5', 29993), ('-5', -29993),
            ('254', 29744), ('-254', -29744),
            ('0', 30000), ('+0', 30000), ('-0', -30000),
        ]:
            with self.subTest(raw=raw):
                self.assertEqual(d.score_value('mate', raw), expected)
                lines = [f'info depth 3 score mate {raw} nodes 50 pv 1a1b', 'bestmove 1a1b']
                self.assertEqual(d.parse_search(lines, 50)['score'], dict(kind='mate', value=expected))
                self.assertEqual(d.parse_search(lines, 50, child=True)['score'],
                                 dict(kind='mate', value=-expected))

    def test_saved_five_mate_discrepancies(self):
        # Inputs: phase3/diagnostic-reproduction/roots.jsonl, root-side values.
        # Expected: search-aware-evaluation/phase4/final/roots.jsonl, independently recorded.
        # USI tokens are reconstructed from the saved values using the former parser.
        for rid, move, budget, recorded, raw, expected in [
            ('202610069-395-r', '2k1k2k', 1, 29992, '-8', 29990),
            ('202613302-422-r', '3k2j', 1, -29993, '7', -29991),
            ('202615380-917-r', '11f10f', 0, -29997, '3', -29995),
            ('202615380-917-r', '11f10f', 1, -29997, '3', -29995),
            ('202615380-917-r', '11f12f', 1, -29995, '5', -29993),
        ]:
            with self.subTest(root=rid, move=move, budget=budget):
                self.assertEqual(abs(int(raw)), 30000 - abs(recorded))
                self.assertEqual(int(raw) < 0, recorded > 0)
                lines = [f'info depth 12 score mate {raw} nodes 100 pv a', 'bestmove a']
                self.assertEqual(d.parse_search(lines, 100, child=True)['score'],
                                 dict(kind='mate', value=expected))

    def test_static_is_already_in_root_side_to_move_perspective(self):
        # 後手手番でもeval=-30、教師=-50なら誤差+20。
        self.assertEqual(d.root_error(-30, search(-50)), 20)
        self.assertEqual(d.root_error(30, search(50)), -20)
        self.assertIsNone(d.root_error(30, search(29995, kind='mate')))
        self.assertIsNone(d.root_error(30, search(0, kind='terminal')))

    def test_child_sign_and_incomplete(self):
        lines = ['info depth 2 score cp -50 nodes 20 pv 1a1b', 'bestmove 1a1b']
        self.assertEqual(d.parse_search(lines, 20, child=True)['score']['value'], 50)
        lines[0] = 'info depth 0 score cp 10 nodes 20'
        self.assertEqual(d.parse_search(lines, 20)['score']['kind'], 'incomplete')


class JournalTests(unittest.TestCase):
    def test_changed_conditions_are_rejected_before_any_root_finishes(self):
        with tempfile.TemporaryDirectory() as directory:
            d.Journal(directory, dict(nodes=100)).close()
            with self.assertRaises(ValueError):
                d.Journal(directory, dict(nodes=200))

    def test_reuse_and_condition_change(self):
        with tempfile.TemporaryDirectory() as directory:
            conditions = dict(engine='sha256-example', nodes=100, history=['1a1b'])
            journal = d.Journal(directory, conditions)
            journal.append(dict(id='root1', loss=10))
            journal.close()
            journal = d.Journal(directory, conditions)
            self.assertEqual(journal.completed['root1']['loss'], 10)
            journal.append(dict(id='root2', loss=0))
            journal.close()
            self.assertEqual(len(list(d.read_jsonl(Path(directory) / 'roots.jsonl'))), 2)
            for field, value in [('nodes', 200), ('engine', 'different'), ('history', ['2a2b'])]:
                with self.subTest(field=field), self.assertRaises(ValueError):
                    d.Journal(directory, dict(conditions, **{field: value}))

    def test_corrupt_or_partial_row_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            journal = d.Journal(directory, {})
            journal.append(dict(id='root1', loss=10))
            journal.close()
            path = Path(directory) / 'roots.jsonl'
            original = path.read_text()
            wrapped = json.loads(original)
            wrapped['record']['loss'] = 0
            path.write_text(json.dumps(wrapped) + '\n')
            with self.assertRaises(ValueError):
                d.Journal(directory, {})
            path.write_text(original + '{')
            with self.assertRaises(ValueError):
                d.Journal(directory, {})


class RootWorkflowTests(unittest.TestCase):
    def test_archived_non_cp_roots_are_excluded_before_proposals(self):
        # pipeline.rs:842-855 exports diagnostic positions individually, only for cp.
        for kind in ('mate', 'terminal', 'incomplete'):
            with self.subTest(kind=kind), patch.object(
                    d, 'run_engine', return_value=search(0, move=None)) as engine:
                root = dict(id='202610634-285-r', game=202610634,
                            moves=['opening'], sfen='saved', teacher_score=dict(kind=kind))
                result = d.evaluate_root(root, {'S0': Path('/s0')}, d.DEFAULT_NODES, {}, None, None)
                self.assertEqual(result['exclusion'],
                                 dict(reason='source_teacher_non_cp', score=dict(kind=kind)))
                engine.assert_not_called()
                summary = d.aggregate([result], [], [], {}, True)
                self.assertEqual((summary['planned_roots'], summary['roots']), (1, 0))
                self.assertEqual(summary['excluded_roots'], [result])

    def test_selected_root_teacher_scores_are_required_individually(self):
        # Diagnostic roots stay eligible when their paired sample is non-cp.
        ids = sorted(d.PHASE0 | {f'1-{ply}-r' for ply in range(250)})
        selection = dict(groups=[dict(split='diagnostic', positions=[rid, rid + '-sample'])
                                 for rid in ids])
        positions = [dict(id=rid, game_seed=int(rid.split('-')[0]), ply=0, path=[],
                          game_sha256='game', extended_sfen='saved') for rid in ids]
        teachers = [dict(request=dict(id=rid), search=search(0)) for rid in ids]
        teachers += [dict(request=dict(id=rid + '-sample'), search=search(1, kind='mate'))
                     for rid in ids]

        def wrapped(data):
            return dict(record=dict(data=data))

        with tempfile.TemporaryDirectory() as directory:
            data = Path(directory)
            phase3 = data / 'phase3'
            phase3.mkdir()
            d.write_json(phase3 / 'selection.json', wrapped(selection))
            (phase3 / 'roots.jsonl').write_text(''.join(json.dumps(wrapped(p)) + '\n'
                                                       for p in positions))
            (phase3 / 'games-0.jsonl').write_text(''.join(
                json.dumps(dict(wrapped(dict(seed=seed, moves=[], opening=[])), sha256='game')) + '\n'
                for seed in {p['game_seed'] for p in positions}))
            teacher_path = phase3 / 'teacher.jsonl'

            def save_teachers(values):
                teacher_path.write_text(''.join(json.dumps(wrapped(t)) + '\n' for t in values))

            save_teachers(teachers)
            roots, sources = d.load_roots(data, include_phase0=True)
            self.assertEqual(len(roots), 256)
            self.assertTrue(all(r['teacher_score']['kind'] == 'cp' for r in roots))
            self.assertIn(teacher_path, sources)
            teachers[0]['search']['score'] = dict(kind='mate', value=-29994)
            save_teachers(teachers)
            roots, _ = d.load_roots(data, include_phase0=True)
            self.assertEqual([r['id'] for r in roots if r['teacher_score']['kind'] != 'cp'], [ids[0]])
            for invalid in (teachers[1:], teachers + [teachers[0]]):
                save_teachers(invalid)
                with self.assertRaises(ValueError):
                    d.load_roots(data, include_phase0=True)

    def test_cp_root_with_missing_proposal_is_still_an_error(self):
        # diagnostic.rs:71-82 rejects absent proposals after export filtering.
        root = dict(id='g-1-r', game=1, moves=['opening'], sfen='saved',
                    teacher_score=dict(kind='cp', value=0))
        with patch.object(d, 'run_engine', return_value=search(0, move=None)):
            with self.assertRaisesRegex(ValueError, 'missing proposal'):
                d.evaluate_root(root, {'S0': Path('/s0')}, d.DEFAULT_NODES, {}, None, None)

    def test_usi_contract_static_perspective_and_new_process(self):
        # USIのdは素の行、evalだけはinfo string。後手の値は反転しない。
        transcript = ('option name USI_Hash type spin default 64 min 1 max 1024\n'
                      'option name Threads type spin default 1 min 1 max 20\n'
                      'option name ResignValue type spin default 20000 min 1 max 99999\nusiok\n'
                      'readyok\nsfen saved\nstatus ongoing\nreadyok\n'
                      'info string evaluation -30\ninfo string end\nreadyok\n')
        with patch.object(d.subprocess, 'Popen') as popen:
            contexts = []
            for _ in range(2):
                process = MagicMock()
                process.__enter__.return_value = process
                process.stdin = io.StringIO()
                process.stdout = io.StringIO(transcript)
                process.returncode = 0
                contexts.append(process)
            popen.side_effect = contexts
            for _ in range(2):
                self.assertEqual(d.run_engine(Path('/engine'), ['1a1b'], expected_sfen='saved'), -30)
            self.assertEqual(popen.call_count, 2)
            for process in contexts:
                commands = process.stdin.getvalue().splitlines()
                self.assertIn('setoption name USI_Hash value 64', commands)
                self.assertIn('setoption name Threads value 1', commands)
                self.assertLess(commands.index('setoption name ResignValue value 99999'),
                                commands.index('isready'))
                self.assertIn('position startpos moves 1a1b', commands)
                self.assertEqual(commands[-1], 'quit')
            self.assertEqual(popen.call_args.args[0][-2:], ['--rules', 'engine-default'])

    def test_missing_resign_option_stops_before_ready_or_search(self):
        for advertised in ('', 'option name ResignValueExtra type spin\n'):
            with self.subTest(advertised=advertised):
                process = MagicMock()
                process.__enter__.return_value = process
                process.stdin = io.StringIO()
                process.stdout = io.StringIO(
                    'option name USI_Hash type spin\noption name Threads type spin\n'
                    + advertised + 'usiok\n')
                process.returncode = 0
                with patch.object(d.subprocess, 'Popen', return_value=process):
                    with self.assertRaisesRegex(ValueError, 'missing option ResignValue'):
                        d.run_engine(Path('/engine'), [], nodes=100)
                self.assertNotIn('isready', process.stdin.getvalue())
                self.assertNotIn('go nodes', process.stdin.getvalue())
                process.kill.assert_called_once()

    def test_search_disables_resignation_before_ready(self):
        process = MagicMock()
        process.__enter__.return_value = process
        process.stdin = io.StringIO()
        process.stdout = io.StringIO(
            'option name USI_Hash type spin\noption name Threads type spin\n'
            'option name ResignValue type spin\nusiok\n'
            'readyok\nsfen saved\nstatus ongoing\nreadyok\n'
            'info depth 12 score mate -8 nodes 4078459 pv 2j1k\nbestmove 2j1k\n')
        process.returncode = 0
        with patch.object(d.subprocess, 'Popen', return_value=process):
            result = d.run_engine(Path('/engine'), ['2k1k2k'], nodes=10000000, child=True)
        commands = process.stdin.getvalue().splitlines()
        self.assertLess(commands.index('setoption name ResignValue value 99999'),
                        commands.index('isready'))
        self.assertEqual(result['best_move'], '2j1k')
        self.assertEqual(result['score'], dict(kind='mate', value=29990))

    def test_terminal_draw_cannot_become_ordinary_cp_zero(self):
        process = MagicMock()
        process.__enter__.return_value = process
        process.stdin = io.StringIO()
        process.stdout = io.StringIO(
            'option name USI_Hash type spin\noption name Threads type spin\n'
            'option name ResignValue type spin\nusiok\n'
            'readyok\nsfen saved\nstatus draw repetition\nreadyok\n')
        process.returncode = 0
        with patch.object(d.subprocess, 'Popen', return_value=process):
            result = d.run_engine(Path('/engine'), ['1a1b'], nodes=100, child=True)
        self.assertEqual(result['score']['kind'], 'terminal')
        self.assertNotIn('go nodes', process.stdin.getvalue())

    def test_union_history_fresh_searches_and_normal_root_exclusion(self):
        root = dict(id='g-1-r', game=1, moves=['opening'], sfen='saved',
                    teacher_score=dict(kind='cp', value=0))
        engines = dict(S0=Path('/s0'), R=Path('/r'))
        calls = []

        def engine(binary, moves, nodes=None, child=False, expected_sfen=None):
            calls.append((binary, moves, nodes, child))
            if nodes is None:
                return -30
            if child:
                return search(10 if moves[-1] == 'a' else 20)
            return search(29995, 'a' if nodes == 100 else 'b',
                          kind='mate' if nodes == 10000 else 'cp')

        with patch.object(d, 'run_engine', side_effect=engine):
            result = d.evaluate_root(root, engines, (100, 1000, 10000), {}, None, None)
        self.assertEqual(set(result['children']), {'a', 'b'})
        self.assertEqual(len([c for c in calls if c[3]]), 4)
        self.assertTrue(all(c[1][0] == 'opening' for c in calls))
        self.assertFalse(result['normal'])
        self.assertEqual(result['errors'], dict(S0=None, R=None))

    def test_mnsd_square_order_and_promotion(self):
        # SFEN a段左端=12aはdense132。+Pは金将の成駒コード47。
        root = dict(ply=5, sfen='+P11/12/12/12/12/12/12/12/12/12/12/11p w 12a 6 -')
        r = d.sfen_record(root)
        self.assertEqual(r['board'][132], 47)
        self.assertEqual(r['board'][11], 65)
        self.assertEqual((r['stm'], r['lion']), (1, 132))
        self.assertEqual(sum(x != 0 for x in r['board']), 2)

    def test_k_extractor_receives_matching_root_and_weights(self):
        # 保存形式の契約から、12aの成麒麟は盤面コード114、先獅子升132。
        modules = d.training_modules()
        root = dict(id='7-2-r', game=7, ply=2,
                    sfen='+o11/12/12/12/12/12/12/12/12/12/12/12 b 12a 3 -')

        def extractor(command, **kwargs):
            from mnsd import map_records, read_header
            path = Path(command[command.index('--input') + 1])
            record = map_records(path)[0]
            self.assertEqual(int(record['board'][132]), 114)
            self.assertEqual((int(record['lion']), int(record['kirin'])), (132, 1))
            self.assertEqual((int(record['game']), int(record['ply'])), (7, 2))
            self.assertEqual(read_header(path).rule_set, 'L0,P0,R1,E0')
            self.assertEqual(command[-2:], ['--jobs', '1'])
            info = Path(command[command.index('--info') + 1])
            info.write_text(json.dumps(dict(k=2, moves=['a', 'b'], kind='static')) + '\n')

        with patch.object(d.subprocess, 'run', side_effect=extractor), \
                patch.object(d.subprocess, 'check_output', return_value='1' * 40):
            result = d.extract_k(root, Path('/extractor'), d.WT / 'nets/pst.bin', modules)
        self.assertEqual(result['k'], 2)


class ArchiveComparisonTests(unittest.TestCase):
    def test_search_fields_and_node_exclusion(self):
        # The instruction requires exact search results and budgets, but nodes have
        # different meanings: total search vs. last completed USI iteration.
        s = dict(search(10), depth=1, nodes=100, requested_nodes=100)
        saved = dict(id='kept', proposals=[s, s, s], teacher_proposals=[s, s],
                     children=[dict(mv='a', searches=[s, s])])
        current = dict(id='kept', exclusion=None,
                       proposals={name: copy.deepcopy(s) for name in ('S0', 'A', 'B')},
                       teacher_proposals=[copy.deepcopy(s) for _ in range(2)],
                       children={'a': [copy.deepcopy(s) for _ in range(2)]},
                       normal=True, errors=dict(S0=0, A=0, B=0))
        with tempfile.TemporaryDirectory() as directory:
            final = Path(directory)
            (final / 'roots.jsonl').write_text(json.dumps(dict(record=saved)) + '\n')
            d.write_json(final / 'report.json', dict(record=dict(
                roots_sha256=d.sha256(final / 'roots.jsonl'), planned_roots=1, exported_roots=1,
                static=dict(diagnostic_root={name: dict(overall=dict(mean_error_cp=0, mae_cp=0))
                                             for name in ('S0', 'A', 'B')}))))
            for path, label in [
                (('proposals', 'S0'), 'S0'), (('proposals', 'A'), 'A'),
                (('proposals', 'B'), 'B'),
                (('teacher_proposals', 0), 'teacher-0'),
                (('teacher_proposals', 1), 'teacher-1'),
                (('children', 'a', 0), 'a-0'), (('children', 'a', 1), 'a-1'),
            ]:
                for field, value in [('nodes', 80), ('best_move', 'b'), ('depth', 2),
                                     ('score', dict(kind='cp', value=11)), ('requested_nodes', 200)]:
                    with self.subTest(search=label, field=field):
                        changed = copy.deepcopy(current)
                        target = changed
                        for part in path:
                            target = target[part]
                        target[field] = value
                        result = d.compare_archive([changed], final)
                        self.assertEqual(result['passed'], field == 'nodes')
                        if field == 'nodes':
                            self.assertEqual(result['differences'], [])
                        else:
                            self.assertEqual([(r['search'], r['field']) for r in result['differences']],
                                             [(label, field)])
            result = d.compare_archive([current], final)
            self.assertEqual(result['search_fields'], ['best_move', 'depth', 'score', 'requested_nodes'])
            self.assertEqual(set(result['excluded_search_fields']), {'nodes'})
            self.assertIn('last completed', result['excluded_search_fields']['nodes'])
            self.assertIn('total', result['excluded_search_fields']['nodes'])

    def test_exclusion_identity_and_complete_partition_are_checked(self):
        # A planned root absent from the archive must be explicitly excluded;
        # equal counts alone do not establish equal sets.
        s = dict(search(10), depth=1, nodes=100, requested_nodes=100)
        saved = dict(id='kept', proposals=[s, s, s], teacher_proposals=[s, s],
                     children=[dict(mv='a', searches=[s, s])])
        current = dict(id='kept', exclusion=None,
                       proposals={name: s for name in ('S0', 'A', 'B')},
                       teacher_proposals=[s, s], children={'a': [s, s]}, normal=True,
                       errors=dict(S0=0, A=0, B=0))
        excluded = dict(id='omitted', exclusion=dict(reason='source_teacher_non_cp',
                                                    score=dict(kind='mate', value=-29994)))
        with tempfile.TemporaryDirectory() as directory:
            final = Path(directory)
            (final / 'roots.jsonl').write_text(json.dumps(dict(record=saved)) + '\n')
            d.write_json(final / 'report.json', dict(record=dict(
                roots_sha256=d.sha256(final / 'roots.jsonl'), planned_roots=2, exported_roots=1,
                static=dict(diagnostic_root={name: dict(overall=dict(mean_error_cp=0, mae_cp=0))
                                             for name in ('S0', 'A', 'B')}))))
            result = d.compare_archive([current, excluded], final)
            self.assertTrue(result['passed'])
            self.assertEqual(result['expected_excluded_roots'], ['omitted'])
            self.assertEqual(result['excluded_roots'], ['omitted'])
            wrong = [dict(current, id='omitted'), dict(excluded, id='kept')]
            result = d.compare_archive(wrong, final)
            self.assertFalse(result['passed'])
            self.assertIn('excluded_roots', {r['field'] for r in result['differences']})
            for rows in ([current], [current, current, excluded],
                         [current, dict(current, id='omitted')]):
                self.assertFalse(d.compare_archive(rows, final)['passed'])
            changed = copy.deepcopy(current)
            changed['proposals']['A']['best_move'] = 'b'
            self.assertFalse(d.compare_archive([changed, excluded], final)['passed'])


if __name__ == '__main__':
    unittest.main()
