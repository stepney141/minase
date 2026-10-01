"""関係補正項の事前登録に従う採取・教師・追跡・分類資料の CLI。

判定の正は ../relational-correction-prep.md。実行ファイルは再ビルドしない。
record は0起算、ply は初期局面から指した手数。値の単位はセンチポーン。
短い読み筋では末端の駒の損得を以降の全手数の値として S/H/P を判定する。
missing_depth_material は短い読み筋の有無だけを記録する。
--all の比較不能な根は、追跡規則の内部値で経路だけを選び直し、
trace_a_star に記録する。通常の Q と教師判定は変更しない。
読み筋と追跡経路は終局手まで残し、それより後の手を捨てる。
pv_truncated は削除手数を教師では候補手別、追跡では経路別に記録する。
sample は generate.log の完了要約から対局数と手数上限の破棄数を読む。
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from contextlib import contextmanager
from decimal import Decimal
import importlib.util
import itertools
import json
import math
from pathlib import Path
import re
import selectors
import statistics
import struct
import subprocess
import sys
import threading
import time

import numpy as np

WT = Path(__file__).resolve().parents[3]
DATA = Path('/home/stepney141/board-games/minase/data/relational-correction')
SA = Path('/home/stepney141/board-games/minase/data/search-aware-evaluation')
M = Decimal('117.7')
BUDGETS = (1_000_000, 10_000_000)
# Load a private module: changing m must not alter the original module's tests.
_spec = importlib.util.spec_from_file_location(
    'phase0_trace', WT / 'docs/measurements/qsearch-output-search-trace/trace_roots.py')
trace = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(trace)
trace.M = M
sys.path.insert(0, str(WT / 'tools/train/pst'))
import mnsd  # noqa: E402

KIND = dict(zip('PILAMVBRHDQKEFTCSGOXN', range(21)))
PROMOTE = {0: 17, 1: 12, 2: 22, 3: 23, 4: 25, 5: 24, 6: 8, 7: 9,
           8: 27, 9: 28, 12: 21, 13: 6, 14: 26, 15: 4, 16: 5,
           17: 7, 18: 20, 19: 10}
UNPROMOTED_STATE = {k: 29 + i for i, k in enumerate(PROMOTE)}
KANJI = list('歩仲香反横竪角飛馬龍奔王醉猛盲銅銀金麒鳳獅太白鯨牛猪鹿鷹鷲')
SQUARE = r'(?:1[0-2]|[1-9])[a-l]'
FORBIDDEN = ('型S', '型H', '型P', '型 S', '型 H', '型 P', '補正項', '表',
             'NNUE', '着手の基準', '計画の目的')


def read_jsonl(path):
    if not path.exists():
        return []
    rows = trace.read_jsonl(path)
    if rows and 'root_id' in rows[0]:
        ids = [r['root_id'] for r in rows]
        if len(ids) != len(set(ids)):
            raise ValueError(f'{path}: duplicate root_id')
    return rows


def read_output(path):
    """Recover only an interrupted final append; reject malformed complete lines."""
    if path.exists():
        raw = path.read_bytes()
        if raw and not raw.endswith(b'\n'):
            start = raw.rfind(b'\n') + 1
            try:
                json.loads(raw[start:])
            except (ValueError, UnicodeDecodeError):
                with path.open('r+b') as stream:
                    stream.truncate(start)
                print(f'{path}: removed incomplete final JSONL append', file=sys.stderr)
            else:
                with path.open('ab') as stream:
                    stream.write(b'\n')
    return read_jsonl(path)


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    trace.write_json(path, value)


class TimeCap(Exception):
    pass


class Control:
    """One batch owns all live engines; fatal errors cancel every worker."""
    def __init__(self, batch_deadline=None):
        self.batch_deadline = batch_deadline
        self.cancelled = threading.Event()
        self.lock = threading.Lock()
        self.processes = set()

    def check(self, deadline=None):
        if self.cancelled.is_set():
            raise RuntimeError('another root failed; batch cancelled')
        now = time.monotonic()
        # If both limits have elapsed, record whichever was reached first.
        limits = [(d, name) for d, name in (
            (self.batch_deadline, 'batch_time_cap'), (deadline, 'root_time_cap'))
            if d is not None]
        if limits and now >= min(limits)[0]:
            raise TimeCap(min(limits)[1])

    def timeout(self, deadline):
        self.check(deadline)
        limits = [d for d in (deadline, self.batch_deadline) if d is not None]
        return max(0, min(limits) - time.monotonic()) if limits else None

    def cancel(self):
        self.cancelled.set()
        with self.lock:
            for process in self.processes:
                process.kill()


class Engine:
    def __init__(self, binary, control, deadline=None):
        self.control, self.deadline = control, deadline
        control.check(deadline)
        self.p = subprocess.Popen([str(binary), '--protocol', 'usi', '--rules', 'engine-default'],
                                  cwd=WT, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.p.stdout, selectors.EVENT_READ)
        self.pending = b''
        with control.lock:
            control.processes.add(self.p)
        try:
            self.send('usi')
            handshake = self.receive('usiok')
            for option in ('TracePath', 'TraceOut', 'TraceDisable'):
                if not any(s.startswith(f'option name {option} ') for s in handshake):
                    raise ValueError(f'missing engine option: {option}')
            for name, value in (('USI_Hash', 64), ('Threads', 1), ('ResignValue', 99999)):
                self.send(f'setoption name {name} value {value}')
            self.send('isready')
            self.receive('readyok')
            self.send('usinewgame')
        except BaseException:
            self.close()
            raise

    def send(self, command):
        self.control.check(self.deadline)
        self.p.stdin.write((command + '\n').encode())
        self.p.stdin.flush()

    def receive(self, prefix):
        lines = []
        while True:
            self.control.check(self.deadline)
            if b'\n' not in self.pending:
                if not self.selector.select(self.control.timeout(self.deadline)):
                    self.control.check(self.deadline)
                    continue
                block = self.p.stdout.read1(65536)
                if not block:
                    self.control.check(self.deadline)
                    raise RuntimeError('engine exited: ' + '\n'.join(lines[-5:]))
                self.pending += block
                continue
            raw, self.pending = self.pending.split(b'\n', 1)
            line = raw.decode().rstrip('\r')
            if line.startswith(('error', 'info string error')):
                raise RuntimeError(line)
            lines.append(line)
            if line == prefix or line.startswith(prefix + ' '):
                return lines

    def position(self, moves):
        self.send('position startpos' + (' moves ' + ' '.join(moves) if moves else ''))

    def board(self, moves):
        self.position(moves)
        self.send('d')
        return self.receive('sfen')[-1].removeprefix('sfen ')

    def close(self):
        # Kill also on errors/caps; reap to prevent leaked engine processes.
        self.p.kill()
        self.p.communicate()
        self.selector.close()
        with self.control.lock:
            self.control.processes.remove(self.p)


@contextmanager
def engine(binary, control, deadline=None):
    instance = Engine(binary, control, deadline)
    try:
        yield instance
    finally:
        instance.close()


def run_search(binary, control, moves, limit, disabled=(), path=None, trace_out=None,
               deadline=None):
    with engine(binary, control, deadline) as e:
        e.position(moves)
        # Finished games must never enter go (which can emit bestmove resign).
        e.send('state')
        state_line = e.receive('state')[-1]
        state = state_line.split(' status ', 1)[1]
        if state != 'ongoing':
            if state.startswith('draw '):
                trace_score = 0
            elif state.startswith('win '):
                side = state_line.split(' board ', 1)[1].split(' status ', 1)[0].split()[1]
                winner = state.split()[1]
                trace_score = 30000 if winner == {'b': 'black', 'w': 'white'}[side] else -30000
            else:
                raise ValueError(f'unknown terminal state: {state}')
            return {'kind': 'terminal', 'status': state, 'score': None, 'score_usi': None,
                    'pv': [], 'bestmove': None, 'depth': 0, 'nodes': 0, 'trace_score': trace_score}
        if path is not None:
            trace_out.parent.mkdir(parents=True, exist_ok=True)
            trace_out.write_text('', encoding='utf-8')
            e.send('setoption name TracePath value ' + ' '.join(path))
            e.send('setoption name TraceOut value ' + str(trace_out.resolve()))
        if path is not None or disabled:
            e.send('setoption name TraceDisable value ' + ','.join(disabled))
        e.send('go ' + limit)
        lines = e.receive('bestmove')
        if lines[-1].split()[1] == 'resign':
            raise ValueError('bestmove resign with ResignValue 99999')
        result = trace.parse_search(lines)
        result['kind'] = result['score_usi'][0]
        return result


def internal_score(search):
    return search['trace_score'] if search['kind'] == 'terminal' else search['score']


def pair_class(a, b):
    if any(v is None for v in (*a, *b)):
        return {'class': 'incomparable', 'differences': None}
    d1, d10 = (Decimal(str(x)) - Decimal(str(y)) for x, y in zip(a, b))
    if abs(d1) < M and abs(d10) < M:
        category = 'small'
    elif abs(d1) >= M and abs(d10) >= M and d1 * d10 > 0 and abs(d10 - d1) <= M:
        category = 'supports_a' if d10 > 0 else 'supports_b'
    else:
        category = 'unstable'
    return {'class': category, 'differences': [float(d1), float(d10)]}


def judge(values, s0):
    candidates = sorted(values)
    pairs = [dict(a=a, b=b, **pair_class(values[a], values[b]))
             for a, b in itertools.combinations(candidates, 2)]
    comparable = all(v is not None for vs in values.values() for v in vs)
    stable = comparable and all(p['class'] != 'unstable' for p in pairs)
    a = min(candidates, key=lambda mv: (-values[mv][1], mv)) if comparable else None
    loss = values[a][1] - values[s0][1] if comparable else None
    return {'C': candidates, 'pairs': pairs, 'comparable': comparable, 'stable': stable,
            'a_star': a, 'R': loss, 'large_error': stable and Decimal(str(loss)) >= M}


def selection_resolved(a, b, values):
    pair = pair_class(values[a], values[b])
    return {'resolved': a == b or pair['class'] in ('small', 'supports_b'), 'pair': pair}


def resolving_interventions(involved, interventions):
    return sorted(name for name, item in interventions.items()
                  if involved and (name == 'combined' or name in involved)
                  and all(item[mode]['resolved'] for mode in ('depth', 'nodes')))


def board(sfen):
    result = {}
    rows = sfen.split()[0].split('/')
    if len(rows) != 12:
        raise ValueError('SFEN must have 12 ranks')
    for rank, row in enumerate(rows):
        file = 12
        tokens = re.findall(r'\d+|\+?[A-Za-z]', row)
        if ''.join(tokens) != row:
            raise ValueError('invalid SFEN')
        for token in tokens:
            if token.isdigit():
                file -= int(token)
            else:
                if not 1 <= file <= 12:
                    raise ValueError('invalid SFEN width')
                result[f'{file}{chr(97 + rank)}'] = token
                file -= 1
        if file != 0:
            raise ValueError('invalid SFEN width')
    return result


def piece_kind(token):
    kind = KIND[token[-1].upper()]
    return PROMOTE[kind] if token.startswith('+') else kind


def material(sfen, black_view, values):
    total = 0
    for token in board(sfen).values():
        kind = piece_kind(token)
        if kind in (11, 21):
            continue
        state = kind if token.startswith('+') else UNPROMOTED_STATE.get(kind, kind)
        total += values[state] * (1 if token[-1].isupper() == black_view else -1)
    return total


def material_values():
    body = (WT / 'nets/pst.bin').read_bytes()[80:]
    return [struct.unpack_from('<i', body, 13680 * 4 + s * 4)[0] for s in range(47)]


def record_sfen(record):
    letters = {v: k for k, v in KIND.items()}
    bases = {v: k for k, v in PROMOTE.items()}
    rows = []
    for rank in reversed(range(12)):
        row, empty = '', 0
        for file in range(12):
            code = int(record['board'][rank * 12 + file])
            if not code:
                empty += 1
                continue
            if empty:
                row += str(empty)
                empty = 0
            white = code >= 65
            payload = code - (65 if white else 1)
            promoted, kind = payload >= 29, payload % 29
            letter = letters[bases[kind] if promoted else kind]
            row += ('+' if promoted else '') + (letter.lower() if white else letter)
        rows.append(row + (str(empty) if empty else ''))
    lion = int(record['lion'])
    square = '-' if lion == 255 else f'{12 - lion % 12}{chr(108 - lion // 12)}'
    return '/'.join(rows) + f" {'bw'[int(record['stm'])]} {square} {int(record['ply']) + 1} -"


def choose_records(records, batch):
    rng = np.random.default_rng(20261001 + batch)
    games = {}
    for index, game in enumerate(records['game']):
        games.setdefault(int(game), []).append(index)
    return [(game, indices[int(rng.integers(len(indices)))])
            for game, indices in sorted(games.items())]


def parallel_rows(roots, out, jobs, work, control):
    """Append completed roots immediately, preserving completed work on failure."""
    saved = read_output(out)
    done = {r['root_id'] for r in saved}
    ids = [r['root_id'] for r in roots]
    if len(ids) != len(set(ids)) or not done <= set(ids):
        raise ValueError('duplicate input roots or output roots absent from input')
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open('a', encoding='utf-8') as stream, ThreadPoolExecutor(max_workers=jobs) as pool:
        futures = []
        try:
            futures = [pool.submit(work, root) for root in roots if root['root_id'] not in done]
            for future in as_completed(futures):
                row = future.result()
                stream.write(json.dumps(row, ensure_ascii=False) + '\n')
                stream.flush()
                saved.append(row)
        except BaseException:
            control.cancel()
            for future in futures:
                future.cancel()
            raise
    return saved


def sample(data, batch, jobs):
    directory = data / f'phase0/batch{batch}'
    # The generator writes the final header and summary only after all games finish.
    log = (directory / 'generate.log').read_text()
    if 'summary:\n' not in log:
        raise ValueError('generation is incomplete: generate.log has no final summary')
    summary = dict(line.split(': ', 1) for line in log.split('summary:\n', 1)[1].splitlines()
                   if ': ' in line)
    games, discarded = int(summary['games_completed']), int(summary['games_discarded_max_ply'])
    if games != 1024:
        raise ValueError('batch must contain 1024 completed/discarded games')
    path = directory / 'generated.bin'
    header = mnsd.read_header(path)
    if (header.seed != 99_000_000 + 100_000 * (batch - 1)
            or not header.generation_commit.startswith('bf897dd')
            or header.teacher_nodes != 100_000 or header.rule_set != 'L0,P0,R1,E0'):
        raise ValueError('generation conditions differ from preregistration')
    records = mnsd.map_records(path)
    kinds = (directory / 'generated.kinds').read_text().splitlines()
    if len(kinds) != len(records) or set(kinds) - {'quiet', 'capture', 'promotion'}:
        raise ValueError('kinds do not match records')
    histories = {}
    for h in read_jsonl(directory / 'generated.history.jsonl'):
        if h['game'] in histories:
            raise ValueError('duplicate history game')
        histories[h['game']] = h['moves']
    roots = []
    for game, index in choose_records(records, batch):
        r = records[index]
        ply = int(r['ply'])
        moves = histories[game][:ply]
        if len(moves) != ply:
            raise ValueError('history ends before root')
        roots.append({'root_id': f'b{batch}-g{game}-p{ply}', 'batch': batch, 'game': game,
                      'ply': ply, 'record': index, 'kind': kinds[index], 'moves': moves,
                      'sfen': record_sfen(r), 'lion_by_kirin_promotion': bool(r['kirin'])})
    if set(histories) != {r['game'] for r in roots}:
        raise ValueError('history and record games differ')
    no_records = games - discarded - len(histories)
    if no_records < 0 or len(roots) != games - discarded - no_records:
        raise ValueError('sample counts do not match non-discarded games')
    control = Control()
    def verify(root):
        with engine(data / 'bin/minase', control) as e:
            actual = e.board(root['moves'])
        if actual != root['sfen']:
            raise ValueError(f"{root['root_id']}: replay mismatch: {actual} != {root['sfen']}")
        lion = root['sfen'].split()[2]
        kirin = lion != '-' and board(actual).get(lion) in ('+O', '+o')
        if kirin != root['lion_by_kirin_promotion']:
            raise ValueError('lion-promotion flag mismatch')
        return root
    # Preserve game order in roots.jsonl, while parallelizing replay checks.
    old = {r['root_id']: r for r in read_output(directory / 'roots.jsonl')}
    if not set(old) <= {r['root_id'] for r in roots}:
        raise ValueError('sample output contains roots absent from deterministic sample')
    with ThreadPoolExecutor(max_workers=jobs) as pool, (directory / 'roots.jsonl').open('a') as stream:
        try:
            pending = [r for r in roots if r['root_id'] not in old]
            for r in roots:
                if r['root_id'] in old and old[r['root_id']] != r:
                    raise ValueError('saved sample differs')
            for root in pool.map(verify, pending):
                stream.write(json.dumps(root, ensure_ascii=False) + '\n')
                stream.flush()
        except BaseException:
            control.cancel()
            raise
    result = {'batch': batch, 'games': games, 'discarded': discarded,
              'no_records': no_records, 'roots': len(roots)}
    write_json(directory / 'sample-summary.json', result)
    return result


class Evaluations:
    def __init__(self, root, binary, control, deadline=None):
        self.root, self.binary, self.control, self.deadline = root, binary, control, deadline
        self.cache = root['evaluation_cache']

    def search(self, moves, nodes):
        key = f'{nodes}:' + ' '.join(moves)
        if key not in self.cache:
            self.cache[key] = run_search(self.binary, self.control, moves, f'nodes {nodes}',
                                         deadline=self.deadline)
        return self.cache[key]

    def child(self, move):
        results = [self.search(self.root['moves'] + [move], n) for n in BUDGETS]
        return {'searches': results,
                'Q': [-s['score'] if s['kind'] == 'cp' else None for s in results]}


def replay_path(e, moves, path):
    """Replay through the first terminal position, including the move reaching it."""
    sfens = []
    for k in range(1, len(path) + 1):
        sfens.append(e.board(moves + path[:k]))
        e.send('state')
        if e.receive('state')[-1].split(' status ', 1)[1] != 'ongoing':
            break
    return path[:len(sfens)], sfens


def line_material(root, binary, control):
    values = material_values()
    lines = {}
    root['pv_truncated'] = {}
    for move in dict.fromkeys((root['a_star'], root['s0'])):
        with engine(binary, control) as e:
            path = [move] + root['children'][move]['searches'][1]['pv']
            kept, sfens = replay_path(e, root['moves'], path)
            root['pv_truncated'][move] = len(path) - len(kept)
            path, sfens = kept, [root['sfen'], *sfens]
            lines[move] = {'path': path, 'sfens': sfens,
                           'mu': [material(s, root['sfen'].split()[1] == 'b', values) for s in sfens]}
    return lines


def error_type(lines, a, s, depth):
    ma, ms = lines[a]['mu'], lines[s]['mu']
    missing = min(len(ma), len(ms)) <= depth
    if Decimal(abs(ma[min(depth, len(ma) - 1)] - ms[min(depth, len(ms) - 1)])) >= M:
        kind = 'S'
    elif Decimal(abs(ma[-1] - ms[-1])) >= M:
        kind = 'H'
    else:
        kind = 'P'
    return {'error_type': kind, 'missing_depth_material': missing}


def teacher_root(root, binary, control, engine_hash):
    root = dict(root, evaluation_cache={}, engine_sha256=engine_hash)
    ev = Evaluations(root, binary, control)
    proposals = [ev.search(root['moves'], n) for n in (100_000, *BUDGETS)]
    if any(p['bestmove'] in (None, 'win', 'none', '0000') for p in proposals):
        raise ValueError(f"{root['root_id']}: root has no playable proposal")
    s0 = proposals[0]['bestmove']
    children = {m: ev.child(m) for m in sorted({p['bestmove'] for p in proposals})}
    root.update(proposals=proposals, s0=s0, d0=proposals[0]['depth'], children=children)
    root.update(judge({m: c['Q'] for m, c in children.items()}, s0))
    if root['large_error']:
        root['lines'] = line_material(root, binary, control)
        root.update(error_type(root['lines'], root['a_star'], s0, root['d0']))
    return root


def teacher(roots, out, binary, jobs):
    control = Control()
    engine_hash = trace.sha256(binary)
    for saved in read_output(out):
        if saved['engine_sha256'] != engine_hash:
            raise ValueError('saved teacher uses a different engine')
    return parallel_rows(roots, out, jobs,
                         lambda r: teacher_root(r, binary, control, engine_hash), control)


def check_proposal(root, result):
    fields = ('bestmove', 'depth', 'score', 'score_usi', 'kind')
    if any(result[k] != root['proposals'][0][k] for k in fields):
        raise ValueError(f"{root['root_id']}: prerequisite mismatch: {result}")


def trace_paths(root, paths, disabled, name, directory, binary, control, deadline):
    traced = {}
    for route, path in paths.items():
        output = directory / f'{route}-{name}.jsonl'
        search = run_search(binary, control, root['moves'], f"depth {root['d0']}",
                            disabled, path, output, deadline)
        events = trace.read_jsonl(output)
        starts = [e for e in events if e['type'] == 'search_start']
        expected = {'path': path, 'disabled': sorted(disabled), 'depth_limit': root['d0'],
                    'node_limit': None}
        if len(starts) != 1:
            raise ValueError('trace search_start is not unique')
        actual = {k: starts[0][k] for k in expected}
        actual['disabled'] = sorted(actual['disabled'])
        if actual != expected:
            raise ValueError('trace conditions differ')
        final, _, _ = trace.final_pass(events, root['d0'])
        if (search['depth'], search['bestmove'], search['score']) != (
                root['d0'], final['best_move'], final['score']):
            raise ValueError('trace and USI result differ')
        traced[route] = {'search': search, 'events': events, 'file': str(output),
                         'root_move': trace.root_move_value(events, root['d0'])}
    results = [t['search'] for t in traced.values()]
    if any(r != results[0] for r in results):
        raise ValueError('traced routes produce different searches')
    return traced


def compact_traces(traced):
    return {route: {k: v for k, v in t.items() if k != 'events'} for route, t in traced.items()}


def trace_root(root, directory, binary, control, root_cap):
    # Copy nested caches: teacher records remain immutable to callers.
    root = json.loads(json.dumps(root))
    started = time.monotonic()
    deadline = started + root_cap if root_cap is not None else None
    result = {'root_id': root['root_id'], 'status': 'complete', 'resolved': False,
              'resolving_interventions': [], 'prerequisites': {}, 'analyses': {},
              'baseline': {}, 'interventions': {}, 'involved': [],
              'evaluation_cache': root['evaluation_cache'], 'children': root['children'],
              'engine_sha256': root['engine_sha256']}
    ev = Evaluations(root, binary, control, deadline)
    try:
        control.check(deadline)
        for limit in ('nodes 100000', f"depth {root['d0']}"):
            search = run_search(binary, control, root['moves'], limit, deadline=deadline)
            check_proposal(root, search)
            result['prerequisites'][limit] = search
        a = root['a_star']
        if a is None:
            # --all also traces incomparable roots. This ordering is only for
            # the route; it never enters ordinary Q comparisons or error counts.
            a = min(root['children'], key=lambda move: (
                internal_score(root['children'][move]['searches'][1]), move))
        result['trace_a_star'] = a
        result['teacher_comparable'] = root['comparable']
        paths = {route: [move] + root['children'][move]['searches'][1]['pv']
                 for route, move in (('s', root['s0']), ('a', a))}
        if paths['s'] == paths['a']:
            del paths['a']
        result['paths'] = paths
        result['pv_truncated'] = {}
        for route, path in paths.items():
            with engine(binary, control, deadline) as e:
                kept, _ = replay_path(e, root['moves'], path)
                result['pv_truncated'][route] = len(path) - len(kept)
                paths[route] = kept
        baseline = trace_paths(root, paths, (), 'none', directory, binary, control, deadline)
        result['baseline'] = compact_traces(baseline)
        for route, traced in baseline.items():
            check_proposal(root, traced['search'])
            child = root['children'][paths[route][0]]['searches'][1]
            value = -internal_score(child)
            analysis = trace.analyze_trace(traced['events'], root['d0'], value)
            result['analyses'][route] = analysis
            if analysis['D'] is not None:
                confirmation = ev.search(root['moves'] + paths[route][:analysis['D']], BUDGETS[1])
                analysis['confirmation'] = confirmation
                analysis = trace.confirm_divergence(analysis, internal_score(confirmation))
                result['analyses'][route] = analysis
        involved = sorted({m for a0 in result['analyses'].values() for m in a0['mechanisms']})
        result['involved'] = involved
        interventions = [(m, [m]) for m in trace.MECHANISMS]
        if involved:
            interventions.append(('combined', involved))
        for name, disabled in interventions:
            item = {'disabled': disabled}
            result['interventions'][name] = item
            traced = trace_paths(root, paths, disabled, name, directory, binary, control, deadline)
            item['traces'] = compact_traces(traced)
            for mode in ('depth', 'nodes'):
                search = (next(iter(traced.values()))['search'] if mode == 'depth' else
                          run_search(binary, control, root['moves'], 'nodes 100000',
                                     disabled=disabled, deadline=deadline))
                item[mode] = {'search': search}
                b = search['bestmove']
                if b in (None, 'win', 'none', '0000'):
                    raise ValueError('intervention selected no playable move')
                if b not in root['children']:
                    root['children'][b] = ev.child(b)
                values = {m: c['Q'] for m, c in root['children'].items()}
                item[mode] = {'search': search, 'selected': b, 'Q': values[b],
                              **selection_resolved(a, b, values)}
        control.check(deadline)
        result['resolving_interventions'] = resolving_interventions(involved, result['interventions'])
        result['resolved'] = bool(result['resolving_interventions'])
    except TimeCap as error:
        result['status'] = str(error)
    result['trace_elapsed_s'] = time.monotonic() - started
    return result


def trace_roots(roots, out, binary, jobs, root_cap=None, batch_cap=None, all_roots=False,
                started=None):
    if started is None:
        started = time.monotonic()
    control = Control(started + batch_cap if batch_cap is not None else None)
    selected = [r for r in roots if all_roots or r['large_error']]
    engine_hash = trace.sha256(binary)
    for r in [*selected, *read_output(out)]:
        if r['engine_sha256'] != engine_hash:
            raise ValueError('teacher/trace uses a different engine')
    rows = parallel_rows(selected, out, jobs,
                         lambda r: trace_root(r, out.parent / 'traces' / r['root_id'], binary,
                                              control, root_cap), control)
    incomplete = any(r['status'] == 'batch_time_cap' for r in rows)
    write_json(out.parent / 'trace-summary.json',
               {'incomplete': incomplete, 'roots': len(selected), 'recorded': len(rows),
                'root_cap_s': root_cap, 'batch_cap_s': batch_cap,
                'elapsed_s': time.monotonic() - started})
    return rows


def timing_roots(binary):
    targets = trace.ROOT_IDS
    source = [r['record']['data'] for r in read_jsonl(SA / 'phase3/roots.jsonl')]
    source = [r for r in source if r['id'].removesuffix('-r') in targets and not r['path']]
    if len(source) != 6:
        raise ValueError('timing requires exactly the six registered roots')
    games = {}
    seeds = {r['game_seed'] for r in source}
    for path in sorted(SA.glob('phase3/games-*.jsonl')):
        for event in read_jsonl(path):
            game = event['record']['data']
            if game['seed'] in seeds:
                if game['seed'] in games or game['moves'][:len(game['opening'])] != game['opening']:
                    raise ValueError('timing history duplicate or opening mismatch')
                games[game['seed']] = game['moves']
    roots = []
    with engine(binary, Control()) as e:
        for r in sorted(source, key=lambda r: r['id']):
            moves = games[r['game_seed']][:r['ply']]
            if len(moves) != r['ply']:
                raise ValueError('timing history too short')
            # No comparison with the historical teacher/search reference values.
            roots.append({'root_id': r['id'].removesuffix('-r'), 'moves': moves,
                          'sfen': e.board(moves), 'ply': r['ply']})
    return roots


def timing(data):
    directory = data / 'phase0/timing'
    binary = data / 'bin/minase'
    roots = timing_roots(binary)
    teachers = teacher(roots, directory / 'teacher.jsonl', binary, 16)
    rows = trace_roots(teachers, directory / 'trace.jsonl', binary, 16, all_roots=True)
    elapsed = {r['root_id']: r['trace_elapsed_s'] for r in rows}
    median = statistics.median(elapsed.values())
    result = {'trace_elapsed_s': elapsed, 'median_s': median, 'root_cap_s': 3 * median}
    write_json(directory / 'timing.json', result)
    return result


COUNT_KEYS = ('games', 'discarded', 'no_records', 'roots', 'comparable', 'stable', 'large_error',
              'S', 'H', 'P', 'resolved', 'unresolved', 'root_time_cap', 'batch_time_cap')


def batch_counts(sample_summary, teachers, traces):
    counts = dict.fromkeys(COUNT_KEYS, 0)
    counts.update({k: sample_summary[k] for k in ('games', 'discarded', 'no_records', 'roots')})
    traced = {r['root_id']: r for r in traces}
    for r in teachers:
        for key in ('comparable', 'stable', 'large_error'):
            counts[key] += int(r[key])
        if r['large_error']:
            counts[r['error_type']] += 1
            t = traced.get(r['root_id'])
            if t is not None:
                resolved = t['status'] == 'complete' and t['resolved']
                counts['resolved' if resolved else 'unresolved'] += 1
                if t['status'] in ('root_time_cap', 'batch_time_cap'):
                    counts[t['status']] += 1
    return counts


def stop_decision(batches):
    total = {key: sum(b['counts'][key] for b in batches) for key in COUNT_KEYS}
    n = total['unresolved']
    incomplete = any(b['incomplete'] for b in batches)
    boundary = bool(batches) and all(b['complete'] for b in batches)
    reasons = []
    if incomplete:
        reasons.append('batch_incomplete')
    if boundary and n >= 30:
        reasons.append('N>=30')
    if boundary and len(batches) == 4:
        reasons.append('four_batches')
    return {'batches': batches, 'total': total, 'N': n, 'stop': bool(reasons),
            'stop_reasons': reasons, 'incomplete': incomplete}


def status(data, save=True):
    batches = []
    for batch in range(1, 5):
        directory = data / f'phase0/batch{batch}'
        path = directory / 'sample-summary.json'
        if not path.exists():
            continue
        summary = json.loads(path.read_text())
        teachers = read_jsonl(directory / 'teacher.jsonl')
        traces = read_jsonl(directory / 'trace.jsonl')
        trace_summary = directory / 'trace-summary.json'
        incomplete = any(t['status'] == 'batch_time_cap' for t in traces)
        if trace_summary.exists():
            incomplete |= json.loads(trace_summary.read_text())['incomplete']
        counts = batch_counts(summary, teachers, traces)
        complete = (len(teachers) == summary['roots'] and len(traces) == counts['large_error']
                    and trace_summary.exists())
        batches.append({'batch': batch, 'counts': counts, 'complete': complete,
                        'incomplete': incomplete})
    result = stop_decision(batches)
    if save:
        write_json(data / 'phase0/status.json', result)
    return result


def piece_description(token, square):
    kind = piece_kind(token)
    name = '玉' if kind == 11 and token[-1].islower() else KANJI[kind]
    return f"{'先手' if token[-1].isupper() else '後手'}{name} {square}"


def text_board(sfen):
    pieces = board(sfen)
    lines = ['各升は筋段と駒を記す。先は先手、後は後手、・は空升を示す。', '', '```text']
    for rank in 'abcdefghijkl':
        cells = []
        for file in reversed(range(1, 13)):
            square = f'{file}{rank}'
            token = pieces.get(square)
            if token is None:
                piece = '・'
            else:
                kind = piece_kind(token)
                name = '玉' if kind == 11 and token[-1].islower() else KANJI[kind]
                piece = ('先' if token[-1].isupper() else '後') + name
            cells.append(f'{square}:{piece}')
        lines.append(' '.join(cells))
    return '\n'.join([*lines, '```', ''])


def move_pieces(pieces, identities, move):
    """Track physical pieces through captures/promotion, including lion two-step moves."""
    squares = re.findall(SQUARE, move)
    if len(squares) not in (2, 3) or ''.join(squares) + ('+' if move.endswith('+') else '') != move:
        raise ValueError(f'invalid move: {move}')
    start, end = squares[0], squares[-1]
    token, identity = pieces.pop(start), identities.pop(start)
    captures = []
    for square in dict.fromkeys(squares[1:]):
        if square in pieces:
            captured = pieces.pop(square)
            if captured[-1].isupper() == token[-1].isupper():
                raise ValueError('PV captures own piece')
            captures.append((identities.pop(square), captured, square))
    after = '+' + token if move.endswith('+') else token
    pieces[end], identities[end] = after, identity
    return identity, token, start, end, captures


def coords(square):
    return int(square[:-1]), ord(square[-1]) - 97


def local_context(sfen, identities, involved):
    pieces = board(sfen)
    lines = []
    inverse = {identity: square for square, identity in identities.items()}
    for identity in sorted(involved):
        if identity not in inverse:
            lines.append(f'根の{identity}の駒は、この局面では盤上にない。')
            continue
        square = inverse[identity]
        x, y = coords(square)
        neighbors = [piece_description(token, sq) for sq, token in pieces.items()
                     if sq != square and max(abs(coords(sq)[0] - x), abs(coords(sq)[1] - y)) <= 2]
        lines.append(f"{piece_description(pieces[square], square)}の距離2以内には、"
                     + ('、'.join(neighbors) if neighbors else '該当する駒なし') + '。')
    for square, token in pieces.items():
        if piece_kind(token) not in (11, 21):
            continue
        x, y = coords(square)
        for dx, dy in itertools.product((-1, 0, 1), repeat=2):
            if (dx, dy) == (0, 0):
                continue
            ray = []
            xx, yy = x + dx, y + dy
            while 1 <= xx <= 12 and 0 <= yy < 12:
                sq = f'{xx}{chr(97 + yy)}'
                if sq in pieces:
                    ray.append(piece_description(pieces[sq], sq))
                    if len(ray) == 2:
                        break
                xx, yy = xx + dx, yy + dy
            lines.append(f"{piece_description(token, square)}から筋差{dx}、段差{dy}の方向は、"
                         + ('、'.join(ray) if ray else '駒なし') + '。')
    return '\n\n'.join(lines)


def packet(root, traced):
    a, s = root['a_star'], root['s0']
    lines = root['lines']
    initial = board(root['sfen'])
    pieces, identities = dict(initial), {sq: sq for sq in initial}
    involved, actions = set(), []
    for k, move in enumerate(lines[a]['path'][:6], 1):
        identity, token, start, end, captures = move_pieces(pieces, identities, move)
        involved.add(identity)
        involved.update(i for i, _, _ in captures)
        capture_text = '、'.join(piece_description(t, sq) for _, t, sq in captures) or 'なし'
        actions.append(f'{k}手目の{move}では、{piece_description(token, start)}が{end}へ動く。'
                       f'取られる駒は{capture_text}。')
        if pieces != board(lines[a]['sfens'][k]):
            raise ValueError('piece tracking differs from engine replay')
    text = [f"# 根 {root['root_id']}", '', '## 根の局面', '', text_board(root['sfen']),
            '## 最初の6手', '', '\n\n'.join(actions), '', '## 関与駒', '',
            '関与駒は根の升で識別する。' + '、'.join(piece_description(initial[i], i)
                                                   for i in sorted(involved)) + '。', '']
    for label, move in (('a*', a), ('s0', s)):
        child, ids = dict(initial), {sq: sq for sq in initial}
        move_pieces(child, ids, move)
        child_sfen = lines[move]['sfens'][1]
        if child != board(child_sfen):
            raise ValueError('child identity tracking differs from replay')
        text.extend([f'## {label}の子局面', '', f'根から{move}を指した局面である。', '',
                     text_board(child_sfen), local_context(child_sfen, ids, involved), ''])
    text.extend(['## a*の読み筋の末端', '', text_board(lines[a]['sfens'][-1]),
                 '## 読み筋と駒の損得', '', 'μは根の手番側から見た値で、単位はセンチポーンである。', ''])
    for label, move in (('a*', a), ('s0', s)):
        text.extend([f"{label}の読み筋は {' '.join(lines[move]['path'])}。", '',
                     f"{label}のμの推移は " + '、'.join(f'{k}手後 {mu}' for k, mu in
                                                       enumerate(lines[move]['mu'])) + '。', ''])
    text.extend(['## 追跡の結果', '', f"追跡の終了状態は {traced['status']}。", ''])
    for route, analysis in traced['analyses'].items():
        point = 'なし' if analysis['D'] is None else str(analysis['D'])
        mechanisms = '、'.join(analysis['mechanisms']) or 'なし'
        text.extend([f"経路{route}の乖離点は{point}、停止の理由は{analysis['reason']}、"
                     f'関与した機構は{mechanisms}。', ''])
    if not traced['analyses']:
        text.extend(['追跡の解析は時間内に完了していない。', ''])
    result = '\n'.join(text)
    check_packet(result)
    return result


def check_packet(text):
    if any(word in text for word in FORBIDDEN) or re.search(r'\b[ＳＳSHP]（', text):
        raise ValueError('classification packet contains a forbidden term')


def classification_instructions():
    source = (WT / 'docs/measurements/relational-correction-prep.md').read_text()
    section = source.split('### 関係の分類\n', 1)[1].split('### 着手の基準', 1)[0]
    definitions = [line for line in section.splitlines()
                   if line.startswith(('- 近接2駒は', '- 王駒射線は', '- 多駒は', '- 不明は'))]
    if len(definitions) != 4:
        raise ValueError('the four registered definitions are missing')
    context = section.split('関与駒は、', 1)[1].split('\n\n', 1)[0]
    text = ('# 分類の指示\n\n根ごとに次の4種類から1つを選び、理由を記す。\n\n関与駒は、'
            + context + '\n\n' + '\n\n'.join(definitions)
            + '\n\n読み筋の末端の局面は分類の対象にせず、関係の帰結を確かめる証拠として使う。'
            + '\n\n回答は根ごとに次のJSON形式とする。\n\n```json\n'
            + '{"root_id": "...", "category": "近接2駒|王駒射線|多駒|不明", "reason": "..."}\n```\n')
    check_packet(text)
    return text


def packets(data, out):
    current = status(data, save=False)
    if not current['stop']:
        raise ValueError('sampling has not stopped')
    texts = []
    out.mkdir(parents=True, exist_ok=True)
    for b in current['batches']:
        directory = data / f"phase0/batch{b['batch']}"
        teachers = {r['root_id']: r for r in read_jsonl(directory / 'teacher.jsonl')}
        for t in sorted(read_jsonl(directory / 'trace.jsonl'), key=lambda r: r['root_id']):
            r = teachers[t['root_id']]
            if r['large_error'] and not (t['status'] == 'complete' and t['resolved']):
                text = packet(r, t)
                (out / f"{r['root_id']}.md").write_text(text, encoding='utf-8')
                texts.append(text)
    (out / 'all.md').write_text('\n\n'.join(texts), encoding='utf-8')
    (out / 'instructions.md').write_text(classification_instructions(), encoding='utf-8')
    return {'packets': len(texts), 'out': str(out)}


def positive(value):
    value = float(value)
    if not math.isfinite(value) or value <= 0:
        raise argparse.ArgumentTypeError('must be finite and positive')
    return value


def main(argv=None):
    started = time.monotonic()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--data', type=Path, default=DATA, help='data directory (bin/ and phase0/)')
    parser.add_argument('--jobs', type=int, default=16)
    sub = parser.add_subparsers(dest='command', required=True)
    for command in ('sample', 'teacher', 'trace', 'timing', 'status', 'packets'):
        p = sub.add_parser(command)
        p.add_argument('--jobs', type=int, default=argparse.SUPPRESS)
        if command in ('sample', 'teacher', 'trace'):
            p.add_argument('--batch', type=int, choices=range(1, 5), required=command == 'sample')
        if command == 'teacher':
            p.add_argument('--roots', type=Path)
            p.add_argument('--out', type=Path)
        elif command == 'trace':
            p.add_argument('--roots-teacher', type=Path)
            p.add_argument('--out', type=Path)
            p.add_argument('--all', action='store_true')
            p.add_argument('--root-cap', type=positive)
            p.add_argument('--batch-cap', type=positive)
        elif command == 'packets':
            p.add_argument('--out', type=Path, required=True)
    args = parser.parse_args(argv)
    if args.jobs < 1:
        parser.error('--jobs must be positive')
    binary = args.data / 'bin/minase'
    if args.command == 'sample':
        result = sample(args.data, args.batch, args.jobs)
    elif args.command in ('teacher', 'trace'):
        inputs = args.roots if args.command == 'teacher' else args.roots_teacher
        if args.batch is not None:
            if inputs is not None or args.out is not None:
                parser.error('--batch cannot be combined with explicit input/output files')
            directory = args.data / f'phase0/batch{args.batch}'
            inputs = directory / ('roots.jsonl' if args.command == 'teacher' else 'teacher.jsonl')
            out = directory / f'{args.command}.jsonl'
            if args.command == 'trace' and (args.root_cap is None or args.batch_cap is None):
                parser.error('batch trace requires --root-cap and --batch-cap')
        else:
            if inputs is None or args.out is None:
                parser.error('provide --batch or both input and --out')
            out = args.out
        if not inputs.is_file():
            parser.error(f'input does not exist: {inputs}')
        roots = read_jsonl(inputs)
        if args.command == 'teacher':
            rows = teacher(roots, out, binary, args.jobs)
        else:
            rows = trace_roots(roots, out, binary, args.jobs, args.root_cap,
                               args.batch_cap, args.all, started)
        result = {'roots': len(rows), 'out': str(out)}
    elif args.command == 'timing':
        if args.jobs != 16:
            parser.error('timing requires --jobs 16')
        result = timing(args.data)
    elif args.command == 'status':
        result = status(args.data)
    else:
        result = packets(args.data, args.out)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    main()
