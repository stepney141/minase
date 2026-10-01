"""事前登録から決めた期待値による検証。

入力区分: 組の閾値・符号・予算差、同値、非通常値、介入の関与と2条件、
標本の乱数順序、停止時点、分類資料、USI終了と時間上限、中断後の再開。
終局手を含む切り詰め、短い読み筋の末端値による深さd0の判定。
"""
import json
from contextlib import contextmanager
from pathlib import Path
import sys
import time

import numpy as np
import pytest

import phase0 as p


@pytest.mark.parametrize('a,b,expected', [
    ([117.7, 117.7], [0, 0], 'supports_a'),
    ([-117.7, -117.7], [0, 0], 'supports_b'),
    ([117.69, -117.69], [0, 0], 'small'),
    ([117.7, -117.7], [0, 0], 'unstable'),
    ([117.7, 235.4], [0, 0], 'supports_a'),
    ([117.7, 235.41], [0, 0], 'unstable'),
    ([117.69, 117.7], [0, 0], 'unstable'),
    ([0, None], [0, 0], 'incomparable'),
])
def test_pair_boundaries(a, b, expected):
    assert p.pair_class(a, b)['class'] == expected


def test_teacher_stability_and_tie():
    r = p.judge({'b': [200, 200], 'a': [200, 200], 's': [0, 0]}, 's')
    assert r['a_star'] == 'a'
    assert r['R'] == 200 and r['large_error'] and r['stable']
    assert r['C'] == ['a', 'b', 's']
    assert len(r['pairs']) == 3
    assert not p.judge({'a': [200, 400], 's': [0, 0]}, 's')['stable']
    single = p.judge({'s': [0, 0]}, 's')
    assert single['stable'] and not single['large_error'] and single['R'] == 0
    assert not p.judge({'a': [None, None], 's': [0, 0]}, 's')['comparable']


@pytest.mark.parametrize('b,values,expected,category', [
    ('a', {'a': [None, None]}, True, 'incomparable'),
    ('b', {'a': [100, 100], 'b': [0, 0]}, True, 'small'),
    ('b', {'a': [0, 0], 'b': [200, 200]}, True, 'supports_b'),
    ('b', {'a': [200, 200], 'b': [0, 0]}, False, 'supports_a'),
    ('b', {'a': [200, 400], 'b': [0, 0]}, False, 'unstable'),
    ('b', {'a': [200, 200], 'b': [None, 0]}, False, 'incomparable'),
])
def test_resolution(b, values, expected, category):
    result = p.selection_resolved('a', b, values)
    assert result['resolved'] is expected
    assert result['pair']['class'] == category


def test_both_modes_and_involved_required():
    yes = {'depth': {'resolved': True}, 'nodes': {'resolved': True}}
    one = {'depth': {'resolved': True}, 'nodes': {'resolved': False}}
    assert p.resolving_interventions(['lmr'], {'correction': yes, 'lmr': one}) == []
    assert p.resolving_interventions([], {'combined': yes}) == []
    assert p.resolving_interventions(['lmr'], {'lmr': yes, 'combined': yes}) == ['combined', 'lmr']
    assert p.resolving_interventions(['lmr'], {'correction': yes, 'combined': yes}) == ['combined']


def test_sample_uses_game_order_and_file_order():
    rows = np.zeros(8, dtype=p.mnsd.RECORD_DTYPE)
    rows['game'] = [3, 1, 3, 2, 1, 3, 2, 3]
    rng = np.random.default_rng(20261002)
    expected = [(g, indices[int(rng.integers(len(indices)))])
                for g, indices in [(1, [1, 4]), (2, [3, 6]), (3, [0, 2, 5, 7])]]
    assert p.choose_records(rows, 1) == expected
    assert p.choose_records(rows, 1) == p.choose_records(rows, 1)


def test_record_sfen_coordinates_and_promotions():
    row = np.zeros(1, dtype=p.mnsd.RECORD_DTYPE)[0]
    row['board'][132] = 65 + 11  # White king at 12a.
    row['board'][11] = 1 + 11    # Black king at 1l.
    row['board'][70] = 1 + 29 + 20  # Promoted kirin (lion) at 2g.
    row['lion'], row['kirin'], row['ply'] = 70, 1, 23
    sfen = p.record_sfen(row)
    assert sfen == 'k11/12/12/12/12/12/10+O1/12/12/12/12/11K b 2g 24 -'
    assert p.board(sfen) == {'12a': 'k', '2g': '+O', '1l': 'K'}


def test_material_excludes_royals_and_uses_promoted_state():
    values = list(range(47))
    sfen = 'k11/12/12/12/12/12/3+E+O+p6/12/12/12/12/11K b - 1 -'
    # Prince excluded; promoted kirin is state 20, promoted pawn is state 17.
    assert p.material(sfen, True, values) == 3
    assert p.material(sfen, False, values) == -3


def test_error_types_use_absolute_differences_and_root_first_move():
    lines = {'a': {'mu': [100, 0, 0, 0]}, 's': {'mu': [100, 200, 200, 0]}}
    assert p.error_type(lines, 'a', 's', 2)['error_type'] == 'S'
    lines['s']['mu'] = [100, 0, 0, 200]
    assert p.error_type(lines, 'a', 's', 2)['error_type'] == 'H'
    lines['s']['mu'][-1] = 100
    assert p.error_type(lines, 'a', 's', 2)['error_type'] == 'P'


@pytest.mark.parametrize('ma,ms,expected', [
    ([0, 200], [0, 0], 'S'),
    ([0, 0], [0, 200], 'S'),
    ([0, 200], [0, 0, 0, 200], 'S'),
    ([0, 0, 0, 200], [0, 200], 'S'),
    ([0, 0], [0, 0, 0, 200], 'H'),
    ([0, 100], [0, 0], 'P'),
])
def test_short_lines_extend_terminal_material_to_depth(ma, ms, expected):
    result = p.error_type({'a': {'mu': ma}, 's': {'mu': ms}}, 'a', 's', 2)
    assert result == {'error_type': expected, 'missing_depth_material': True}


@pytest.fixture
def replay_engine(monkeypatch):
    def install(terminal_at, status='win black royal-capture'):
        class Replay:
            ended = False

            def board(self, moves):
                assert not self.ended, 'position requires a new game after a terminal state'
                self.ply = len(moves) - 1  # One move precedes the root.
                assert terminal_at is None or self.ply <= terminal_at
                return 'k11/' + '12/' * 10 + f'11K b - {self.ply + 2} -'

            def send(self, command):
                assert command == 'state'

            def receive(self, prefix):
                assert prefix == 'state'
                state = status if self.ply == terminal_at else 'ongoing'
                self.ended = state != 'ongoing'
                return ['state rules L0,P0,R1,E0 board 12 b status ' + state]

        @contextmanager
        def replay(*args):
            yield Replay()
        monkeypatch.setattr(p, 'engine', replay)
    return install


@pytest.mark.parametrize('terminal_at,kept', [(1, 1), (2, 2), (3, 3), (None, 3)])
@pytest.mark.parametrize('status', ['win black royal-capture', 'draw repetition'])
def test_material_stops_after_terminal_move(replay_engine, terminal_at, kept, status):
    replay_engine(terminal_at, status)
    root = {'moves': ['opening'], 'sfen': 'k11/' + '12/' * 10 + '11K b - 2 -',
            'a_star': 'a', 's0': 's', 'children': {
                move: {'searches': [{}, {'pv': ['next', 'last']}]} for move in ('a', 's')}}
    lines = p.line_material(root, None, p.Control())
    for move in ('a', 's'):
        assert lines[move]['path'] == [move, 'next', 'last'][:kept]
        assert len(lines[move]['sfens']) == len(lines[move]['mu']) == kept + 1
        assert lines[move]['sfens'][-1].split()[3] == str(kept + 2)
    assert root['pv_truncated'] == {'a': 3 - kept, 's': 3 - kept}


def batch(n, complete=True, incomplete=False):
    counts = dict.fromkeys(p.COUNT_KEYS, 0)
    counts['unresolved'] = n
    return {'counts': counts, 'complete': complete, 'incomplete': incomplete}


def test_status_n_includes_caps_and_stops_only_at_boundary():
    teachers = [{'root_id': str(i), 'comparable': True, 'stable': True,
                 'large_error': True, 'error_type': 'S'} for i in range(4)]
    traces = [{'root_id': '0', 'resolved': True, 'status': 'complete'},
              {'root_id': '1', 'resolved': False, 'status': 'complete'},
              {'root_id': '2', 'resolved': False, 'status': 'root_time_cap'}]
    c = p.batch_counts({'games': 1024, 'discarded': 1, 'no_records': 3, 'roots': 1020},
                       teachers, traces)
    assert c['unresolved'] == 2 and c['resolved'] == 1 and c['root_time_cap'] == 1
    assert p.stop_decision([batch(30)])['stop']
    assert not p.stop_decision([batch(29)])['stop']
    assert not p.stop_decision([batch(30, complete=False)])['stop']
    assert p.stop_decision([batch(0)] * 4)['stop_reasons'] == ['four_batches']
    assert p.stop_decision([batch(0, False, True)])['stop_reasons'] == ['batch_incomplete']
    assert p.stop_decision([batch(20), batch(10)])['N'] == 30


def test_status_counts_games_without_records_per_batch_and_in_total(tmp_path):
    """Non-discarded games without records are counted separately from roots."""
    for batch_id, discarded, no_records in [(1, 40, 3), (2, 20, 5)]:
        p.write_json(tmp_path / f'phase0/batch{batch_id}/sample-summary.json',
                     {'batch': batch_id, 'games': 1024, 'discarded': discarded,
                      'no_records': no_records, 'roots': 1024 - discarded - no_records})
    result = p.status(tmp_path, save=False)
    assert [b['counts']['no_records'] for b in result['batches']] == [3, 5]
    assert result['total']['no_records'] == 8
    assert result['total']['roots'] == 1980
    assert result['total']['games'] == 2048
    assert result['total']['discarded'] == 60


def fixture_packet():
    sfen = 'k11/12/12/12/12/12/12/12/12/11P/12/11K b - 1 -'
    a = '1j1i'
    s = '1l2k'
    sa = 'k11/12/12/12/12/12/12/12/11P/12/12/11K w - 2 -'
    ss = 'k11/12/12/12/12/12/12/12/12/11P/10K1/12 w - 2 -'
    root = {'root_id': 'b1-g1-p2', 'sfen': sfen, 'a_star': a, 's0': s,
            'error_type': 'S', 'lines': {
                a: {'path': [a], 'sfens': [sfen, sa], 'mu': [100, 100]},
                s: {'path': [s], 'sfens': [sfen, ss], 'mu': [100, 100]}}}
    return root, {'status': 'root_time_cap', 'analyses': {}}


def test_packet_is_blind_and_contains_all_boards():
    root, traced = fixture_packet()
    result = p.packet(root, traced)
    assert all(word not in result for word in p.FORBIDDEN)
    assert result.count('```text') == 4
    assert '先手歩 1j' in result and '1i:先歩' in result
    assert '盤上にない' not in result
    assert len(result.split('```text\n')[1].split('```')[0].splitlines()) == 12
    assert '根から1j1i' in result and '0手後 100' in result
    instructions = p.classification_instructions()
    assert all(word not in instructions for word in p.FORBIDDEN)
    source = (p.WT / 'docs/measurements/relational-correction-prep.md').read_text()
    for line in source.splitlines():
        if line.startswith(('- 近接2駒は', '- 王駒射線は', '- 多駒は', '- 不明は')):
            assert line in instructions


def test_lion_capture_return_and_promotion_identity():
    pieces = {'6f': 'N', '5e': 'p', '4d': 'P'}
    ids = {sq: sq for sq in pieces}
    moved = p.move_pieces(pieces, ids, '6f5e6f')
    assert moved[4] == [('5e', 'p', '5e')]
    assert ids['6f'] == '6f' and pieces['6f'] == 'N'
    p.move_pieces(pieces, ids, '4d4c+')
    assert ids['4c'] == '4d' and pieces['4c'] == '+P'


def test_resume_skips_completed_and_preserves_other_rows(tmp_path):
    roots = [{'root_id': str(i)} for i in range(3)]
    out = tmp_path / 'rows.jsonl'
    calls = []
    def work(r):
        calls.append(r['root_id'])
        return r
    p.parallel_rows(roots[:1], out, 1, work, p.Control())
    p.parallel_rows(roots, out, 2, work, p.Control())
    assert sorted(calls) == ['0', '1', '2']
    assert len(p.read_jsonl(out)) == 3


@pytest.fixture
def fake_engine(tmp_path):
    script = tmp_path / 'minase'
    log = tmp_path / 'commands'
    script.write_text(f'''#!{sys.executable}
import os, sys, signal
from pathlib import Path
for raw in sys.stdin:
    line = raw.strip()
    with open({str(log)!r}, 'a') as f:
        f.write(line + '\\n')
    if line == 'usi':
        print('option name TracePath type string default')
        print('option name TraceOut type string default')
        print('option name TraceDisable type string default')
        print('usiok', flush=True)
    elif line == 'isready':
        print('readyok', flush=True)
    elif line == 'state':
        print('state rules L0,P0,R1,E0 board 12 b status ' + os.environ.get('TEST_STATE', 'ongoing'), flush=True)
    elif line.startswith('go '):
        if os.environ.get('TEST_HANG'):
            signal.pause()
        print('info depth 6 score ' + os.environ.get('TEST_SCORE', 'cp 50') + ' nodes 100000 pv 1a1b', flush=True)
        print('bestmove ' + os.environ.get('TEST_MOVE', '1a1b'), flush=True)
''')
    script.chmod(0o755)
    return script, log


def test_usi_conditions_mate_and_resign(fake_engine, monkeypatch):
    binary, log = fake_engine
    control = p.Control()
    result = p.run_search(binary, control, ['2a2b'], 'nodes 100000', disabled=['lmr'])
    assert result['kind'] == 'cp'
    commands = log.read_text()
    for command in ('setoption name ResignValue value 99999', 'setoption name Threads value 1',
                    'setoption name USI_Hash value 64', 'position startpos moves 2a2b',
                    'setoption name TraceDisable value lmr'):
        assert command in commands
    monkeypatch.setenv('TEST_SCORE', 'mate -0')
    result = p.run_search(binary, control, [], 'depth 6')
    assert result['kind'] == 'mate' and result['score_usi'] == ['mate', '-0']
    monkeypatch.setenv('TEST_MOVE', 'resign')
    with pytest.raises(ValueError, match='resign'):
        p.run_search(binary, control, [], 'depth 6')
    assert not control.processes


def test_terminal_does_not_search(fake_engine, monkeypatch):
    binary, log = fake_engine
    monkeypatch.setenv('TEST_STATE', 'draw repetition')
    result = p.run_search(binary, p.Control(), [], 'nodes 1000000')
    assert result['kind'] == 'terminal' and result['score'] is None
    assert 'go ' not in log.read_text()


def test_deadline_kills_running_engine(fake_engine, monkeypatch):
    binary, _ = fake_engine
    monkeypatch.setenv('TEST_HANG', '1')
    control = p.Control()
    start = time.monotonic()
    with pytest.raises(p.TimeCap, match='root_time_cap'):
        p.run_search(binary, control, [], 'depth 6', deadline=start + 0.3)
    assert not control.processes
    assert time.monotonic() - start < 2


def test_batch_cap_marks_queued_roots_and_returns_summary(tmp_path, monkeypatch):
    binary = tmp_path / 'binary'
    binary.write_bytes(b'fake')
    roots = [{'root_id': str(i), 'large_error': True, 'engine_sha256': p.trace.sha256(binary),
              'evaluation_cache': {}, 'children': {}} for i in range(4)]
    # Expiration at entry must mark all pending roots without starting engines.
    monkeypatch.setattr(p, 'run_search', lambda *a, **k: pytest.fail('search after batch deadline'))
    out = tmp_path / 'trace.jsonl'
    rows = p.trace_roots(roots, out, binary, 2, root_cap=10, batch_cap=1,
                         started=time.monotonic() - 2)
    assert len(rows) == 4 and all(r['status'] == 'batch_time_cap' for r in rows)
    assert json.loads((tmp_path / 'trace-summary.json').read_text())['incomplete']


def test_cache_reuses_both_budgets_and_rejects_nonordinary(monkeypatch):
    root = {'moves': ['opening'], 'evaluation_cache': {}}
    calls = []
    def search(binary, control, moves, limit, **kw):
        calls.append((moves, limit))
        return {'kind': 'mate' if limit.endswith('10000000') else 'cp', 'score': -200}
    monkeypatch.setattr(p, 'run_search', search)
    e = p.Evaluations(root, None, p.Control())
    assert e.child('a')['Q'] == [200, None]
    p.Evaluations(json.loads(json.dumps(root)), None, p.Control()).child('a')
    assert len(calls) == 2


def test_prerequisite_mismatch_aborts_before_trace(monkeypatch, tmp_path):
    proposal = {'bestmove': 'a', 'depth': 6, 'score': 0, 'score_usi': ['cp', '0'], 'kind': 'cp'}
    root = {'root_id': 'x', 'moves': [], 'd0': 6, 'proposals': [proposal], 'children': {},
            'evaluation_cache': {}, 'engine_sha256': 'hash'}
    monkeypatch.setattr(p, 'run_search', lambda *args, **kw: dict(proposal, depth=5))
    monkeypatch.setattr(p, 'trace_paths', lambda *args: pytest.fail('trace after mismatch'))
    with pytest.raises(ValueError, match='prerequisite'):
        p.trace_root(root, tmp_path, None, p.Control(), None)


@pytest.mark.parametrize('incomparable', [False, True])
@pytest.mark.parametrize('terminal_at,outcome,reason', [
    (None, 'path_end', '読み筋の末端'),
    (1, 'last_royal_capture', '王駒の捕獲'),
    (2, 'path_end', '読み筋の末端'),
])
def test_trace_full_flow_keeps_both_modes_all_mechanisms_and_cache(
        monkeypatch, tmp_path, incomparable, replay_engine, terminal_at, outcome, reason):
    replay_engine(terminal_at)
    proposal = {'bestmove': 's', 'depth': 6, 'score': 0, 'score_usi': ['cp', '0'],
                'kind': 'cp', 'nodes': 100000, 'pv': ['s']}
    child_a = dict(proposal, bestmove='next', score=-200, pv=['next', 'last'])
    child_s = dict(proposal, bestmove='next', score=0, pv=['next', 'last'])
    root = {'root_id': 'r', 'moves': ['opening'], 'd0': 6, 's0': 's', 'a_star': 'a', 'comparable': True,
            'proposals': [proposal], 'engine_sha256': 'hash',
            'children': {'a': {'Q': [200, 200], 'searches': [child_a, child_a]},
                         's': {'Q': [0, 0], 'searches': [child_s, child_s]}},
            'evaluation_cache': {}}
    if incomparable:
        root['comparable'], root['a_star'] = False, None
        child_s.update(kind='mate', score=29995, score_usi=['mate', '5'])
        root['children']['s']['Q'] = [None, None]
    for move, child in [('a', child_a), ('s', child_s)]:
        for budget in (1000000, 10000000):
            root['evaluation_cache'][f'{budget}:opening {move}'] = child
    calls = []
    def search(binary, control, moves, limit, disabled=(), path=None, trace_out=None, deadline=None):
        calls.append((moves, limit, list(disabled), path))
        if limit in ('nodes 1000000', 'nodes 10000000'):
            # New candidate b is equivalent to a at both budgets.
            assert moves[-1] == 'b'
            return dict(child_a, score=-250)
        result = dict(proposal, bestmove='b' if disabled else 's')
        if path is not None:
            expected_path = [path[0], 'next', 'last'][:terminal_at]
            assert path == expected_path
            stop = 'qs_see' if path[0] == 'a' else outcome
            node = {'type': 'node', 'iter': 6, 'pass': 0, 'seq': 1, 'ply': 1,
                    'kind': 'main', 'depth_req': 5, 'depth_after_iir': 5,
                    'alpha': -1000, 'beta': 1000, 'null': None, 'result': 0,
                    'bound': 'exact', 'aborted': False, 'path_move': 'next',
                    'path_move_outcome': stop, 'path_move_searches': []}
            top = dict(node, seq=2, ply=0, kind='root', path_move=path[0],
                       path_move_outcome='searched', path_move_searches=[
                           {'depth': 6, 'reduction': 0, 'score': 0, 'alpha': -1000, 'beta': 1000}])
            events = [{'type': 'search_start', 'path': path, 'disabled': list(disabled),
                       'depth_limit': 6, 'node_limit': None}, node, top,
                      {'type': 'pass', 'iter': 6, 'pass': 0, 'aborted': False,
                       'alpha': -1000, 'beta': 1000, 'best_move': result['bestmove'], 'score': 0}]
            trace_out.parent.mkdir(parents=True, exist_ok=True)
            trace_out.write_text(''.join(json.dumps(e)+'\n' for e in events))
        return result
    monkeypatch.setattr(p, 'run_search', search)
    result = p.trace_root(root, tmp_path, None, p.Control(), None)
    assert result['resolved']
    assert result['involved'] == ['qs_see']
    assert result['resolving_interventions'] == ['combined', 'qs_see']
    assert len(result['interventions']) == 12
    assert len(list(tmp_path.glob('*.jsonl'))) == 26
    if not incomparable:
        assert result['analyses']['s']['D'] is None
    assert result['trace_a_star'] == 'a'
    kept = 3 if terminal_at is None else terminal_at
    assert result['paths'] == {route: [move, 'next', 'last'][:kept]
                               for route, move in [('a', 'a'), ('s', 's')]}
    assert result['pv_truncated'] == {'a': 3 - kept, 's': 3 - kept}
    if incomparable:
        assert result['analyses']['s']['original_reason'] == reason
    assert result['teacher_comparable'] is not incomparable
    assert result['analyses']['a']['t_prime_D'] == -200
    assert all(item['depth']['resolved'] and item['nodes']['resolved']
               for item in result['interventions'].values())
    # All twelve fixed-node searches are required, regardless of depth success.
    assert sum(limit == 'nodes 100000' and bool(disabled) for _, limit, disabled, _ in calls) == 12
    # New b is evaluated once per budget; confirmation at D=1 reuses child a.
    assert sum(limit in ('nodes 1000000', 'nodes 10000000') for _, limit, _, _ in calls) == 2
    assert result['trace_elapsed_s'] >= 0


def test_root_cap_persists_partial_root(fake_engine, monkeypatch, tmp_path):
    binary, _ = fake_engine
    monkeypatch.setenv('TEST_HANG', '1')
    root = {'root_id': 'r', 'engine_sha256': p.trace.sha256(binary), 'evaluation_cache': {},
            'children': {}, 'd0': 6, 'moves': [], 'proposals': [], 'large_error': True}
    out = tmp_path / 'trace.jsonl'
    rows = p.trace_roots([root], out, binary, 1, root_cap=0.3, batch_cap=3)
    assert rows[0]['status'] == 'root_time_cap'
    assert not rows[0]['resolved']
    assert rows[0]['trace_elapsed_s'] >= 0.3
    assert not json.loads((tmp_path / 'trace-summary.json').read_text())['incomplete']


def test_sample_refuses_unfinished_generation(tmp_path):
    directory = tmp_path / 'phase0/batch1'
    directory.mkdir(parents=True)
    (directory / 'generate.log').write_text('progress: games=10/1024\n')
    with pytest.raises(ValueError, match='incomplete'):
        p.sample(tmp_path, 1, 2)
    assert not (directory / 'roots.jsonl').exists()


def test_resume_recovers_only_unfinished_last_append(tmp_path):
    out = tmp_path / 'roots.jsonl'
    out.write_bytes(b'{"root_id":"done"}\n{"root_id":')
    assert p.read_output(out) == [{'root_id': 'done'}]
    assert out.read_bytes() == b'{"root_id":"done"}\n'
    out.write_bytes(b'{"root_id":"done"}')
    assert p.read_output(out) == [{'root_id': 'done'}]
    assert out.read_bytes().endswith(b'\n')
    out.write_bytes(b'{broken}\n')
    with pytest.raises(ValueError):
        p.read_output(out)


@pytest.mark.parametrize('no_records', [0, 3])
def test_sample_complete_files_and_replay_failure(tmp_path, monkeypatch, no_records):
    """The sidecar contains moves[:ply], and each selected record is checked by USI."""
    import struct
    from contextlib import contextmanager
    directory = tmp_path / 'phase0/batch1'
    directory.mkdir(parents=True)
    rows = np.zeros(4, dtype=p.mnsd.RECORD_DTYPE)
    rows['game'] = [1, 1, 2, 2]
    rows['ply'] = [0, 2, 0, 2]
    rows['lion'] = 255
    rows['board'][:, 132] = 76
    rows['board'][:, 11] = 12
    header = bytearray(136)
    struct.pack_into('<4sII', header, 0, b'MNSD', 1, 160)
    header[12:23] = b'L0,P0,R1,E0'
    header[44:84] = b'bf897dd' + b'0' * 33
    struct.pack_into('<IQQ', header, 116, 100000, 99000000, len(rows))
    (directory / 'generated.bin').write_bytes(header + rows.tobytes())
    (directory / 'generated.kinds').write_text('quiet\ncapture\npromotion\nquiet\n')
    (directory / 'generated.history.jsonl').write_text(
        '\n'.join(json.dumps({'game': i, 'moves': ['first', 'second']}) for i in (1, 2)) + '\n')
    (directory / 'generate.log').write_text(
        f'summary:\ngames_completed: 1024\ngames_discarded_max_ply: {1022 - no_records}\n')
    checked = []
    fail = False
    class Replay:
        def board(self, moves):
            checked.append(list(moves))
            return ('k11/' + '12/' * 10 + f'11K b - {len(moves) + 1} -') if not fail else 'bad'
    @contextmanager
    def replay(*args):
        yield Replay()
    monkeypatch.setattr(p, 'engine', replay)
    result = p.sample(tmp_path, 1, 2)
    assert result == {'batch': 1, 'games': 1024, 'discarded': 1022 - no_records,
                      'no_records': no_records, 'roots': 2}
    assert json.loads((directory / 'sample-summary.json').read_text()) == result
    selected = p.read_jsonl(directory / 'roots.jsonl')
    assert [r['game'] for r in selected] == [1, 2]
    rng = np.random.default_rng(20261002)
    assert [r['record'] for r in selected] == [int(rng.integers(2)),
                                               2 + int(rng.integers(2))]
    assert all(r['moves'] == ['first', 'second'][:r['ply']] for r in selected)
    assert len(checked) == 2
    p.sample(tmp_path, 1, 2)
    assert len(checked) == 2
    (directory / 'roots.jsonl').unlink()
    fail = True
    with pytest.raises(ValueError, match='replay mismatch'):
        p.sample(tmp_path, 1, 2)
    fail = False
    with (directory / 'generated.history.jsonl').open('a') as stream:
        stream.write(json.dumps({'game': 3, 'moves': []}) + '\n')
    with pytest.raises(ValueError, match='history and record games differ'):
        p.sample(tmp_path, 1, 2)


def test_batch_cap_kills_running_and_marks_queued(fake_engine, monkeypatch, tmp_path):
    binary, _ = fake_engine
    monkeypatch.setenv('TEST_HANG', '1')
    roots = [{'root_id': str(i), 'engine_sha256': p.trace.sha256(binary), 'evaluation_cache': {},
              'children': {}, 'd0': 6, 'moves': [], 'proposals': [], 'large_error': True}
             for i in range(3)]
    rows = p.trace_roots(roots, tmp_path / 'trace.jsonl', binary, 1,
                         root_cap=5, batch_cap=0.3)
    assert len(rows) == 3
    assert all(r['status'] == 'batch_time_cap' and not r['resolved'] for r in rows)
    assert json.loads((tmp_path / 'trace-summary.json').read_text())['incomplete']


def test_engine_error_is_not_masked_by_nonordinary_handling(fake_engine, monkeypatch):
    binary, _ = fake_engine
    # Unexpected engine errors must propagate even with terminal-aware replay.
    source = binary.read_text().replace("print('bestmove '",
        "print('info string error: TracePath: the game is already over', flush=True)\n        print('bestmove '")
    binary.write_text(source)
    control = p.Control()
    with pytest.raises(RuntimeError, match='game is already over'):
        p.run_search(binary, control, [], 'depth 6')
    assert not control.processes
