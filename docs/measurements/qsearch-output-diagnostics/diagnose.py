"""履歴を保つUSI探索による候補集合の診断と、保存済みA/Bへの判定規則の適用。"""

import argparse
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from decimal import Decimal
import fcntl
import hashlib
from itertools import combinations
import json
import math
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import tempfile

WT = Path(__file__).resolve().parents[3]
DATA = Path('/home/stepney141/board-games/minase/data/search-aware-evaluation')
MARGIN = Decimal('142.1')
PHASE0 = {'202610280-278-r', '202610489-236-r', '202612388-210-r',
          '202612852-212-r', '202613669-264-r', '202614203-172-r'}
DEFAULT_NODES = (100_000, 1_000_000, 10_000_000)
NO_MOVE = {'resign', 'win', 'none', '0000'}


def sha256(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def json_hash(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':'),
                                     allow_nan=False).encode()).hexdigest()


def read_json(path):
    return json.loads(Path(path).read_text(encoding='utf-8'))


def read_jsonl(path):
    with Path(path).open(encoding='utf-8') as stream:
        for line in stream:
            yield json.loads(line)


def write_json(path, value):
    path = Path(path)
    temporary = path.with_suffix(path.suffix + '.tmp')
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2,
                                    allow_nan=False) + '\n', encoding='utf-8')
    temporary.replace(path)


def classify(difference, margin=MARGIN):
    """teacher_common::classify::category。正負の支持方向も保持する。"""
    if difference is None:
        return 'incomparable'
    low, high = map(lambda x: Decimal(str(x)), difference)
    margin = Decimal(str(margin))
    if low >= margin and high >= margin and abs(high - low) <= margin:
        return 'supports_s0'
    if low <= -margin and high <= -margin and abs(high - low) <= margin:
        return 'supports_candidate'
    if abs(low) < margin and abs(high) < margin:
        return 'small'
    return 'unstable'


def decision(proposals, children):
    """diagnostic.rs::decision。childrenの値はすでに根手番側。"""
    excluded = sorted({s['score']['kind'] for pair in children.values() for s in pair
                       if s['score']['kind'] != 'cp'})
    if excluded:
        return dict(category='incomparable', excluded=excluded, loss=None, pairs=[])
    scores = {move: [s['score']['value'] for s in pair] for move, pair in children.items()}
    pairs = []
    for a, b in combinations(sorted(scores), 2):
        diff = [scores[a][i] - scores[b][i] for i in (0, 1)]
        pairs.append(dict(a=a, b=b, difference=diff, category=classify(diff)))
    best = max(s[1] for s in scores.values())
    losses = {name: best - scores[s['best_move']][1] for name, s in proposals.items()}
    return dict(category='unstable' if any(p['category'] == 'unstable' for p in pairs)
                else 'stable', excluded=[], loss=losses, pairs=pairs)


def grouped_metric(groups, independent_se=None):
    """根加重平均のcluster SE: sqrt(G/(G-1) Σ(Sg-ng*mean)^2) / N。"""
    count = sum(n for n, total in groups.values())
    games = len(groups)
    mean = sum(total for n, total in groups.values()) / count if count else None
    se = (math.sqrt(games / (games - 1) * sum((total - n * mean) ** 2
          for n, total in groups.values())) / count) if games > 1 else None
    lower = mean - 1.96 * se if se is not None else None
    usable = count >= 150 and se is not None
    return dict(mean_cp=mean, game_se_cp=se, independent_root_se_cp=independent_se,
                lower_cp=lower, roots=count, games=games, eligible=usable,
                significantly_positive=bool(usable and lower > 0))


def paired_metric(rows):
    groups = defaultdict(list)
    for game, difference in rows:
        groups[game].append(difference)
    values = [d for _, d in rows]
    se = statistics.stdev(values) / math.sqrt(len(values)) if len(values) > 1 else None
    return grouped_metric({g: (len(v), sum(v)) for g, v in groups.items()}, se)


def root_error(static_stm, teacher):
    # USI eval と教師はともに根手番側。後手でも再度反転しない。
    return static_stm - teacher['score']['value'] if teacher['score']['kind'] == 'cp' else None


def score_value(kind, value):
    if kind == 'cp':
        return int(value)
    if kind == 'mate':
        distance = abs(int(value))
        # usi.rs::score_text: |n| = max(MATE - |score| - 2, 0).
        # Zero is lossy; ±MATE represents its sign, not an exact internal distance.
        magnitude = 30000 - distance - 2 if distance else 30000
        return (-1 if str(value).startswith('-') else 1) * magnitude
    raise ValueError(f'unknown score kind: {kind}')


def parse_search(lines, nodes, child=False):
    best = lines[-1].split()[1]
    infos = [line.split() for line in lines if line.startswith('info depth ') and ' score ' in line]
    if not infos:
        return dict(requested_nodes=nodes, best_move=None if best in NO_MOVE else best,
                    nodes=0, depth=0, score=dict(kind='incomplete'))
    fields = infos[-1]
    depth, consumed = int(fields[fields.index('depth') + 1]), int(fields[fields.index('nodes') + 1])
    if 'lowerbound' in fields or 'upperbound' in fields:
        raise ValueError('last completed search has only a bound')
    index = fields.index('score')
    kind, raw = fields[index + 1:index + 3]
    value = score_value(kind, raw) * (-1 if child else 1)
    score = dict(kind=kind, value=value) if depth > 0 else dict(kind='incomplete')
    return dict(requested_nodes=nodes, best_move=None if best in NO_MOVE else best,
                nodes=consumed, depth=depth, score=score)


def run_engine(binary, moves, nodes=None, child=False, expected_sfen=None):
    """1問い合わせ1プロセス。dで終局と履歴再生後の局面も確認する。"""
    with subprocess.Popen([str(binary), '--protocol', 'usi', '--rules', 'engine-default'],
                          cwd=WT, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          text=True, bufsize=1) as process:
        def send(command):
            process.stdin.write(command + '\n')
            process.stdin.flush()

        def receive(prefix):
            lines = []
            while True:
                line = process.stdout.readline()
                if not line:
                    raise RuntimeError(f'{binary}: unexpected EOF: {lines[-5:]}')
                line = line.rstrip('\n')
                if line.startswith(('error', 'info string error')):
                    raise RuntimeError(line)
                lines.append(line)
                if line == prefix or line.startswith(prefix + ' '):
                    return lines

        try:
            send('usi')
            handshake = receive('usiok')
            for name, value in [('USI_Hash', 64), ('Threads', 1), ('ResignValue', 99999)]:
                if not any(line.startswith(f'option name {name} ') for line in handshake):
                    raise ValueError(f'{binary}: missing option {name}')
                send(f'setoption name {name} value {value}')
            send('isready')
            receive('readyok')
            send('usinewgame')
            send('position startpos moves ' + ' '.join(moves))
            send('d')
            send('isready')
            position = receive('readyok')
            sfens = [line.removeprefix('sfen ') for line in position if line.startswith('sfen ')]
            statuses = [line.removeprefix('status ') for line in position if line.startswith('status ')]
            if len(sfens) != 1 or len(statuses) != 1:
                raise ValueError(f'missing position diagnostics: {position}')
            if expected_sfen is not None and sfens[0] != expected_sfen:
                raise ValueError(f'replayed position differs: {sfens[0]} != {expected_sfen}')
            if nodes is None:
                send('eval')
                send('isready')
                lines = receive('readyok')
                values = [int(line.split()[-1]) for line in lines
                          if line.startswith('info string evaluation ')]
                if len(values) != 1:
                    raise ValueError('missing or duplicate static evaluation')
                result = values[0]
            elif statuses[0] != 'ongoing':
                result = dict(requested_nodes=nodes, nodes=0, depth=0, best_move=None,
                              score=dict(kind='terminal', status=statuses[0]))
            else:
                send(f'go nodes {nodes}')
                result = parse_search(receive('bestmove'), nodes, child)
            send('quit')
        except BaseException:
            process.kill()
            raise
    if process.returncode:
        raise RuntimeError(f'{binary}: exit {process.returncode}')
    return result


def load_roots(data, include_phase0=False):
    selection_path = data / 'phase3/selection.json'
    selection = read_json(selection_path)['record']['data']
    ids = {g['positions'][0] for g in selection['groups'] if g['split'] == 'diagnostic'}
    if len(ids) != 256 or not PHASE0 <= ids:
        raise ValueError('diagnostic selection must contain 256 roots and the six phase0 roots')
    source_paths = [selection_path, data / 'phase3/roots.jsonl']
    positions = {}
    for entry in read_jsonl(source_paths[-1]):
        r = entry['record']['data']
        if r['id'] in ids:
            if r['id'] in positions:
                raise ValueError('duplicate root')
            positions[r['id']] = r
    if positions.keys() != ids:
        raise ValueError('missing selected root')
    teacher_path = data / 'phase3/teacher.jsonl'
    source_paths.append(teacher_path)
    teacher_scores = {}
    for entry in read_jsonl(teacher_path):
        response = entry['record']['data']
        rid = response['request']['id']
        if rid in ids:
            score = response['search']['score']
            if rid in teacher_scores or score['kind'] not in ('cp', 'mate', 'terminal', 'incomplete'):
                raise ValueError(f'{rid}: duplicate teacher or unknown score kind')
            teacher_scores[rid] = score
    if teacher_scores.keys() != ids:
        raise ValueError('missing selected root teacher response')
    seeds = {r['game_seed'] for r in positions.values()}
    games = {}
    for path in sorted((data / 'phase3').glob('games-*.jsonl')):
        source_paths.append(path)
        for entry in read_jsonl(path):
            g = entry['record']['data']
            if g['seed'] in seeds:
                if g['seed'] in games or g['moves'][:len(g['opening'])] != g['opening']:
                    raise ValueError('duplicate game or opening differs from moves')
                games[g['seed']] = entry
    roots = []
    for rid in sorted(ids):
        p = positions[rid]
        g = games[p['game_seed']]
        moves = g['record']['data']['moves'][:p['ply']]
        if (p['path'] or len(moves) != p['ply'] or g['sha256'] != p['game_sha256']
                or int(rid.split('-')[0]) != p['game_seed']):
            raise ValueError(f'{rid}: root history identity differs')
        if include_phase0 or rid not in PHASE0:
            roots.append(dict(id=rid, game=p['game_seed'], moves=moves,
                              sfen=p['extended_sfen'], ply=p['ply'], teacher_score=teacher_scores[rid]))
    return roots, source_paths


class Journal:
    """完了した根を追記し、条件または内容が異なる再開を拒否する。"""

    def __init__(self, directory, conditions):
        self.directory = Path(directory)
        self.directory.mkdir(parents=True, exist_ok=True)
        self.lock = (self.directory / '.lock').open('a')
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            manifest = self.directory / 'manifest.json'
            self.path = self.directory / 'roots.jsonl'
            if manifest.exists():
                if read_json(manifest) != conditions:
                    raise ValueError('diagnostic conditions changed; use another output directory')
            else:
                if self.path.exists():
                    raise ValueError('roots.jsonl exists without manifest')
                write_json(manifest, conditions)
            self.condition_hash = json_hash(conditions)
            self.completed = {}
            if self.path.exists():
                for wrapped in read_jsonl(self.path):
                    row = wrapped['record']
                    if (wrapped['sha256'] != json_hash(row)
                            or row['conditions_sha256'] != self.condition_hash
                            or row['id'] in self.completed):
                        raise ValueError('corrupt or duplicate saved root')
                    self.completed[row['id']] = row
        except BaseException:
            self.close()
            raise

    def append(self, row):
        if row['id'] in self.completed:
            raise ValueError('duplicate completed root')
        row = dict(row, conditions_sha256=self.condition_hash)
        with self.path.open('a', encoding='utf-8') as stream:
            stream.write(json.dumps(dict(sha256=json_hash(row), record=row), allow_nan=False) + '\n')
            stream.flush()
            os.fsync(stream.fileno())
        self.completed[row['id']] = row

    def close(self):
        self.lock.close()


def training_modules():
    # Only diagnostic evaluation needs numpy/torch; the rule tests use stdlib.
    sys.path.insert(0, str(WT / 'tools/train/pst'))
    import torch
    torch.set_num_threads(1)
    from pst_diagnostics import Weights
    from mnsd import RECORD_DTYPE, write_mnsd
    import numpy as np
    return np, Weights, RECORD_DTYPE, write_mnsd


def positions_array(rows, np, dtype):
    records = np.zeros(len(rows), dtype=dtype)
    for i, row in enumerate(rows):
        for name in ('board', 'stm', 'lion'):
            records[i][name] = row[name]
        # MNSDの復元は先獅子升上の成麒麟からこのビットを検証する。
        records[i]['kirin'] = row['lion'] != 255 and row['board'][row['lion']] in (50, 114)
    return records


def promotion_diagnostics(data, weights, modules):
    """元のcheck_promotionsと同じ262局面の合法成り368手を整数評価する。"""
    np, Weights, dtype, _ = modules
    sample = read_json(data / 'phase4/diagnostics/S0-positions.json')
    saved = read_json(data / 'phase4/diagnostics/S0-promotions.json')
    if len(sample) != len(saved) or [r['index'] for r in saved] != list(range(len(sample))):
        raise ValueError('promotion sample correspondence differs')
    after, origins, keys = [], [], []
    for i, row in enumerate(saved):
        for move in row['promotions']:
            after.append(move['after'])
            origins.append(i)
            keys.append((i, move['move']))
    if len(set(keys)) != len(keys):
        raise ValueError('duplicate promotion in saved sample')
    before_records, after_records = (positions_array(x, np, dtype) for x in (sample, after))
    result = {}
    for name, path in weights.items():
        model = Weights(path)
        before, following = model.evaluate(before_records), model.evaluate(after_records)
        delta = -following.astype(np.int64) - before[origins].astype(np.int64)
        if name == 'S0':
            expected = [m['delta'] for r in saved for m in r['promotions']]
            if (not np.array_equal(before, [r['eval'] for r in saved])
                    or not np.array_equal(following, [r['eval'] for r in after])
                    or not np.array_equal(delta, expected)):
                raise ValueError('S0 does not reproduce saved promotion evaluations')
        result[name] = dict(positions=len(sample), moves=len(keys),
                            negative=int((delta < 0).sum()),
                            median_delta_cp=float(np.median(delta)))
    return result


def sfen_record(root):
    """拡張SFENをMNSDの盤面に直す。升順はrank*12+file。"""
    board_text, side, lion, next_move, deferred = root['sfen'].split()
    if side not in ('b', 'w') or deferred != '-' or int(next_move) != root['ply'] + 1:
        raise ValueError('unsupported root state')
    letters = 'PILAMVBRHDQKEFTCSGOXN'
    promoted = dict(zip('PILAMVBRHDEFTCSGOX', [17, 12, 22, 23, 25, 24, 8, 9, 27, 28,
                                                        21, 6, 26, 4, 5, 7, 20, 10]))
    board = [0] * 144
    rows = board_text.split('/')
    if len(rows) != 12:
        raise ValueError('wrong SFEN rank count')
    for row_number, row in enumerate(rows):
        column = 0
        tokens = re.findall(r'\d+|\+?[A-Za-z]', row)
        if ''.join(tokens) != row:
            raise ValueError('invalid SFEN token')
        for token in tokens:
            if token.isdigit():
                column += int(token)
            else:
                letter = token[-1]
                kind = promoted[letter.upper()] if token.startswith('+') else letters.index(letter.upper())
                if column >= 12:
                    raise ValueError('SFEN rank overflow')
                board[(11 - row_number) * 12 + column] = (1 + kind + 64 * letter.islower()
                                                          + 29 * token.startswith('+'))
                column += 1
        if column != 12:
            raise ValueError('wrong SFEN rank width')
    lion_square = 255
    if lion != '-':
        match = re.fullmatch(r'(1[0-2]|[1-9])([a-l])', lion)
        if match is None:
            raise ValueError('invalid lion square')
        lion_square = (11 - (ord(match[2]) - ord('a'))) * 12 + 12 - int(match[1])
    return dict(board=board, stm=int(side == 'w'), lion=lion_square)


def extract_k(root, binary, weights, modules):
    np, _, dtype, write_mnsd = modules
    records = positions_array([sfen_record(root)], np, dtype)
    records['game'], records['ply'], records['result'] = root['game'], root['ply'], 1
    with tempfile.TemporaryDirectory(prefix='qsearch-diagnostic-') as directory:
        directory = Path(directory)
        input_path, output, info = [directory / p for p in ('roots.bin', 'leaves.bin', 'leaves.jsonl')]
        write_mnsd(input_path, records, seed=0, network_checksum=weights.read_bytes()[48:80],
                   generation_commit=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=WT,
                                                              text=True).strip())
        subprocess.run([str(binary), '--input', str(input_path), '--output', str(output),
                        '--info', str(info), '--weights', str(weights), '--jobs', '1'],
                       cwd=WT, capture_output=True, text=True, check=True)
        rows = list(read_jsonl(info))
        if len(rows) != 1 or rows[0]['k'] != len(rows[0]['moves']):
            raise ValueError('qsearch leaf correspondence differs')
        return rows[0]


def evaluate_root(root, engines, budgets, weights, leaf, modules):
    # pipeline.rs:842-855 filters each diagnostic position by its saved teacher score.
    # diagnostic.rs:473-488 requires exactly these cp exports, before any proposals.
    if root['teacher_score']['kind'] != 'cp':
        return dict(id=root['id'], game=root['game'],
                    exclusion=dict(reason='source_teacher_non_cp', score=root['teacher_score']))
    proposal_nodes, low, high = budgets
    proposals = {name: run_engine(binary, root['moves'], proposal_nodes, expected_sfen=root['sfen'])
                 for name, binary in engines.items()}
    teachers = [run_engine(engines['S0'], root['moves'], n, expected_sfen=root['sfen'])
                for n in (low, high)]
    candidates = sorted({s['best_move'] for s in [*proposals.values(), *teachers]
                         if s['best_move'] is not None})
    if any(s['best_move'] is None for s in [*proposals.values(), *teachers]):
        raise ValueError(f"{root['id']}: missing proposal; cannot form the prescribed candidate set")
    children = {move: [run_engine(engines['S0'], root['moves'] + [move], n, child=True)
                       for n in (low, high)] for move in candidates}
    # The high-budget teacher proposal already is an independent search of this root.
    teacher = teachers[1]
    static = {name: run_engine(binary, root['moves'], expected_sfen=root['sfen'])
              for name, binary in engines.items()}
    errors = {name: root_error(value, teacher) for name, value in static.items()}
    k = {name: extract_k(root, leaf, weights[name], modules) for name in engines if name in ('Q', 'Qc')}
    return dict(id=root['id'], game=root['game'], exclusion=None,
                proposals=proposals, teacher_proposals=teachers,
                children=children, decision=decision(proposals, children), static=static,
                errors=errors, normal=teacher['score']['kind'] == 'cp', k=k)


def aggregate(rows, names, judge, promotions, smoke):
    planned = len(rows)
    excluded_roots = [r for r in rows if r['exclusion'] is not None]
    rows = [r for r in rows if r['exclusion'] is None]
    result = dict(smoke=smoke, planned_roots=planned, roots=len(rows), excluded_roots=excluded_roots,
                  margin_cp=str(MARGIN),
                  counts=dict(Counter(r['decision']['category'] for r in rows)), engines={})
    for name in names:
        stable = [r for r in rows if r['decision']['category'] == 'stable']
        normal = [r for r in rows if r['normal']]
        losses = [r['decision']['loss'][name] for r in stable]
        errors = [r['errors'][name] for r in normal]
        loss_test = paired_metric([(r['game'], r['decision']['loss'][name] - r['decision']['loss']['S0'])
                                   for r in stable])
        error_test = paired_metric([(r['game'], abs(r['errors'][name]) - abs(r['errors']['S0']))
                                    for r in normal])
        excluded = any(t['significantly_positive'] for t in (loss_test, error_test))
        result['engines'][name] = dict(
            stable_roots=len(losses), mean_loss_cp=statistics.mean(losses) if losses else None,
            normal_roots=len(errors), mean_error_cp=statistics.mean(errors) if errors else None,
            mae_cp=statistics.mean(map(abs, errors)) if errors else None,
            promotions=promotions[name], loss_difference=loss_test, absolute_error_difference=error_test,
            judged=name in judge, excluded_from_s0=excluded if name in judge and not smoke else None,
            k_distribution=dict(sorted(Counter(str(r['k'][name]['k']) for r in rows
                                                if name in r['k']).items(), key=lambda x: int(x[0]))),
            leaf_kinds=dict(Counter(r['k'][name]['kind'] for r in rows if name in r['k'])))
    return result


def summary_markdown(result):
    lines = ['# 採否前の診断', '', f"対象は{result['roots']}根であり、値の単位はセンチポーンである。",
             f"選定した{result['planned_roots']}根のうち、保存済み教師値が通常評価でない"
             f"{len(result['excluded_roots'])}根を探索前に除外した。",
             '標準誤差は対局内の差を合計して求め、対象が150根未満の指標は判定に使わない。', '']
    if result['smoke']:
        lines += ['この実行は縮小検証であり、採否の判定には使わない。', '']
    lines += ['| エンジン | 安定根 | 平均選択損失 | 通常根 | 平均誤差 | 平均絶対誤差 | 評価を下げる成り手 |',
              '|---|---:|---:|---:|---:|---:|---:|']
    for name, r in result['engines'].items():
        p = r['promotions']
        lines.append(f"| {name} | {r['stable_roots']} | {r['mean_loss_cp']} | {r['normal_roots']} | "
                     f"{r['mean_error_cp']} | {r['mae_cp']} | {p['negative']}/{p['moves']} |")
    lines += ['', '| 候補 | 指標 | 平均差 | 対局単位SE | 根独立SE | 下限 | 根数 | 対局数 | 判定対象 |',
              '|---|---|---:|---:|---:|---:|---:|---:|---|']
    for name, r in result['engines'].items():
        for metric, label in [('loss_difference', '選択損失'), ('absolute_error_difference', '絶対誤差')]:
            m = r[metric]
            lines.append(f"| {name} | {label} | {m['mean_cp']} | {m['game_se_cp']} | "
                         f"{m['independent_root_se_cp']} | {m['lower_cp']} | {m['roots']} | "
                         f"{m['games']} | {m['eligible']} |")
    lines += ['', 'SEは標準誤差、下限は平均差から対局単位の標準誤差の1.96倍を引いた値である。', '']
    for name, r in result['engines'].items():
        if r['judged']:
            verdict = ('縮小実行のため判定しない' if result['smoke'] else
                       'S0との採否測定へ進めない' if r['excluded_from_s0'] else
                       'この2条件ではS0との採否測定から除外されない')
            lines += [f'{name}は{verdict}。', '']
        if r['k_distribution']:
            lines += [f"{name}のkの分布は{json.dumps(r['k_distribution'])}であり、記録だけに使う。",
                      f"末端の種類別件数は{json.dumps(r['leaf_kinds'])}である。", '']
    return '\n'.join(lines)


def archived_rule(final):
    """report/rootsだけから再計算する。静的誤差は保存された対局別合計を使う。"""
    report_path, roots_path = final / 'report.json', final / 'roots.jsonl'
    report = read_json(report_path)['record']
    if sha256(roots_path) != report['roots_sha256']:
        raise ValueError('archived roots checksum differs from report')
    rows = [e['record'] for e in read_jsonl(roots_path)]
    decisions = {d['id']: d for d in report['decisions']}
    if len(decisions) != len(rows) or {r['id'] for r in rows} != decisions.keys():
        raise ValueError('archived root set differs')
    for r in rows:
        computed = decision(dict(zip(('S0', 'A', 'B'), r['proposals'])),
                            {c['mv']: c['searches'] for c in r['children']})
        if dict(id=r['id'], **computed) != decisions[r['id']]:
            raise ValueError(f"archived decision does not reproduce: {r['id']}")
    static = report['static']['diagnostic_root']
    # In this archive game_group equals the actual game seed; verify rather than assume.
    seed_counts = Counter(r['id'].split('-')[0] for r in rows)
    if seed_counts != Counter({g: v['count'] for g, v in static['S0']['by_game_group'].items()}):
        raise ValueError('archived game groups differ from game seeds')
    output = dict(planned_roots=report['planned_roots'], evaluated_roots=len(rows),
                  include_phase0_roots=True, counts=report['counts'], engines={},
                  files={str(p): sha256(p) for p in (report_path, roots_path)},
                  se_formula='sqrt(G/(G-1) * sum((S_g - n_g * mean)^2)) / N')
    for name in ('A', 'B'):
        stable = [r for r in decisions.values() if r['category'] == 'stable']
        loss = paired_metric([(r['id'].split('-')[0], r['loss'][name] - r['loss']['S0']) for r in stable])
        groups = {}
        candidate = static[name]['by_game_group']
        baseline = static['S0']['by_game_group']
        if candidate.keys() != baseline.keys():
            raise ValueError('static comparison groups differ')
        for g, base in baseline.items():
            n = base['count']
            if candidate[g]['count'] != n:
                raise ValueError('static comparison counts differ')
            groups[g] = (n, n * (candidate[g]['mae_cp'] - base['mae_cp']))
        error = grouped_metric(groups)
        error['independent_root_se_unavailable_reason'] = (
            'report.json contains game sums but no within-game paired second moments; '
            'roots.jsonl contains no static scores')
        output['engines'][name] = dict(loss_difference=loss, absolute_error_difference=error,
            excluded_from_s0=loss['significantly_positive'] or error['significantly_positive'])
    a, b = (output['engines'][n]['loss_difference'] for n in ('A', 'B'))
    if round(a['mean_cp'], 1) != 1.7 or round(b['mean_cp'], 1) != 24.1 or round(b['independent_root_se_cp'], 2) != 8.57:
        raise ValueError('archived independent-root values do not reproduce the design')
    output['reference_values_reproduced'] = True
    return output


def compare_archive(rows, final):
    """本番予算のS0/A/B再実行を、保存された248根と照合する。"""
    report = read_json(final / 'report.json')['record']
    if sha256(final / 'roots.jsonl') != report['roots_sha256']:
        raise ValueError('archived root checksum differs')
    planned_ids = {r['id'] for r in rows}
    current = {r['id']: r for r in rows if r['exclusion'] is None}
    excluded_ids = {r['id'] for r in rows if r['exclusion'] is not None}
    saved = [entry['record'] for entry in read_jsonl(final / 'roots.jsonl')]
    saved_ids = {r['id'] for r in saved}
    expected_excluded = planned_ids - saved_ids
    differences = []
    for field, actual, expected in [
        ('planned_roots', len(rows), report['planned_roots']),
        ('unique_roots', len(planned_ids), len(rows)),
        ('exported_roots', len(saved), report['exported_roots']),
        ('evaluated_roots', sorted(current), sorted(saved_ids)),
        ('excluded_roots', sorted(excluded_ids), sorted(expected_excluded)),
        ('excluded_root_count', len(excluded_ids), report['planned_roots'] - report['exported_roots']),
    ]:
        if actual != expected:
            differences.append(dict(field=field, actual=actual, expected=expected))

    search_fields = ['best_move', 'depth', 'score', 'requested_nodes']

    def compare_search(rid, label, actual, expected):
        for key in search_fields:
            if actual[key] != expected[key]:
                differences.append(dict(id=rid, search=label, field=key,
                                        actual=actual[key], expected=expected[key]))

    for old in saved:
        rid = old['id']
        if rid not in current:
            continue
        new = current[rid]
        for name, expected in zip(('S0', 'A', 'B'), old['proposals']):
            compare_search(rid, name, new['proposals'][name], expected)
        for i, expected in enumerate(old['teacher_proposals']):
            compare_search(rid, f'teacher-{i}', new['teacher_proposals'][i], expected)
        children = {c['mv']: c['searches'] for c in old['children']}
        if children.keys() != new['children'].keys():
            differences.append(dict(id=rid, field='candidate_set', actual=sorted(new['children']),
                                    expected=sorted(children)))
        for move in children.keys() & new['children'].keys():
            for i, expected in enumerate(children[move]):
                compare_search(rid, f'{move}-{i}', new['children'][move][i], expected)
    normal_ids = {r['id'] for r in current.values() if r['normal']}
    if normal_ids != {r['id'] for r in saved}:
        differences.append(dict(field='normal_roots', actual=sorted(normal_ids),
                                expected=sorted(r['id'] for r in saved)))
    for name in ('S0', 'A', 'B'):
        if current.keys() != saved_ids:
            continue
        errors = [current[r['id']]['errors'][name] for r in saved]
        if any(e is None for e in errors):
            continue
        expected = report['static']['diagnostic_root'][name]['overall']
        for key, value in [('mean_error_cp', statistics.mean(errors)),
                           ('mae_cp', statistics.mean(map(abs, errors)))]:
            if not math.isclose(value, expected[key], abs_tol=1e-9, rel_tol=0):
                differences.append(dict(engine=name, field=key, actual=value, expected=expected[key]))
    return dict(compared_roots=len(saved_ids & current.keys()), planned_roots=report['planned_roots'],
                excluded_roots=sorted(excluded_ids), expected_excluded_roots=sorted(expected_excluded),
                search_fields=search_fields, excluded_search_fields=dict(nodes=(
                    'USI records nodes at the last completed iteration; the archive records '
                    'total search nodes, including the interrupted iteration. The final total '
                    'is not always emitted by USI. requested_nodes is still compared.')),
                passed=not differences, differences=differences)


def assignments(items):
    result = {}
    for item in items:
        name, separator, value = item.partition('=')
        if not separator or not re.fullmatch(r'[A-Za-z][A-Za-z0-9_]*', name) or not value or name in result:
            raise ValueError(f'invalid or duplicate NAME=PATH: {item}')
        result[name] = Path(value).resolve(strict=True)
    return result


def run(args):
    engines, weights = assignments(args.engine), assignments(args.weights)
    if 'S0' not in engines:
        raise ValueError('--engine S0=PATH is required')
    if 'S0' not in weights:
        weights['S0'] = WT / 'nets/pst.bin'
    if engines.keys() != weights.keys():
        raise ValueError('every engine requires matching --weights NAME=MNPT (S0 defaults to nets/pst.bin)')
    judge = ([n for n in ('Q', 'Qc') if n in engines] if args.judge is None else
             [] if args.judge == '' else args.judge.split(','))
    if not set(judge) <= engines.keys() or 'S0' in judge or len(judge) != len(set(judge)):
        raise ValueError('--judge must name distinct supplied candidates, excluding S0')
    budgets = (args.proposal_nodes, args.teacher_low_nodes, args.teacher_high_nodes)
    if min(budgets) < 1 or budgets[1] >= budgets[2] or args.jobs < 1 or (args.limit is not None and args.limit < 1):
        raise ValueError('invalid node budgets, jobs, or limit')
    modules = training_modules()
    for name, binary in engines.items():
        modules[1](weights[name])
        if weights[name].read_bytes() not in binary.read_bytes():
            raise ValueError(f'{name}: MNPT is not embedded in the supplied executable')
    roots, sources = load_roots(args.data, args.include_phase0_roots)
    if args.root_id is not None:
        roots = [r for r in roots if r['id'] == args.root_id]
        if not roots:
            raise ValueError('--root-id must name a selected root')
    if args.limit is not None:
        roots = roots[:args.limit]
    for name in ('S0-positions.json', 'S0-promotions.json'):
        sources.append(args.data / 'phase4/diagnostics' / name)
    sources += [WT / 'tools/train/pst' / name for name in
                ('train_pst.py', 'pst_diagnostics.py', 'features.py', 'taper.py', 'mnsd.py')]
    leaf = args.qsearch_leaf.resolve()
    if any(name in engines for name in ('Q', 'Qc')):
        sources.append(leaf)
    smoke = budgets != DEFAULT_NODES or args.limit is not None or args.root_id is not None
    if args.out.resolve().is_relative_to(args.data.resolve()):
        raise ValueError('output must be outside the archived input directory')
    if args.compare_final is not None:
        if smoke or not args.include_phase0_roots or engines.keys() != {'S0', 'A', 'B'}:
            raise ValueError('--compare-final requires S0,A,B, all 256 roots and production budgets')
        sources += [args.compare_final / name for name in ('report.json', 'roots.jsonl')]
    conditions = dict(format='minase-qsearch-output-diagnostics-v1', smoke=smoke,
        script_sha256=sha256(__file__), files={str(p): sha256(p) for p in sources},
        engines={n: dict(path=str(p), sha256=sha256(p), weights_sha256=sha256(weights[n]))
                 for n, p in engines.items()}, roots=roots, budgets=list(budgets),
        margin_cp=str(MARGIN), rules='engine-default', threads=1, hash_mib=64, resign_value=99999,
        jobs=args.jobs, judge=judge, include_phase0_roots=args.include_phase0_roots)
    journal = Journal(args.out, conditions)
    try:
        if not journal.completed.keys() <= {r['id'] for r in roots}:
            raise ValueError('unknown completed root')
        promotions = promotion_diagnostics(args.data, weights, modules)
        print(f'reusing {len(journal.completed)}/{len(roots)} completed roots', flush=True)
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            futures = {pool.submit(evaluate_root, r, engines, budgets, weights, leaf, modules): r['id']
                       for r in roots if r['id'] not in journal.completed}
            for future in as_completed(futures):
                journal.append(future.result())
                print(f'completed {len(journal.completed)}/{len(roots)}: {futures[future]}', flush=True)
        rows = [journal.completed[r['id']] for r in roots]
        result = aggregate(rows, engines, judge, promotions, smoke)
        result['conditions_sha256'] = journal.condition_hash
        result['roots_sha256'] = sha256(journal.path)
        if args.compare_final is not None:
            result['archive_comparison'] = compare_archive(rows, args.compare_final)
        write_json(args.out / 'result.json', result)
        (args.out / 'summary.md').write_text(summary_markdown(result), encoding='utf-8')
        if args.compare_final is not None and not result['archive_comparison']['passed']:
            raise ValueError('archive reproduction differs; inspect result.json archive_comparison')
    finally:
        journal.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    live = commands.add_parser('run')
    live.add_argument('--engine', action='append', required=True)
    live.add_argument('--weights', action='append', default=[])
    live.add_argument('--judge', help='default: supplied Q,Qc; empty means no candidate gate')
    live.add_argument('--data', type=Path, default=DATA)
    live.add_argument('--out', type=Path, required=True)
    live.add_argument('--include-phase0-roots', action='store_true')
    live.add_argument('--compare-final', type=Path, help='compare a full S0/A/B rerun with this archive')
    subset = live.add_mutually_exclusive_group()
    subset.add_argument('--limit', type=int)
    subset.add_argument('--root-id', help='evaluate one selected root for testing; disables candidate gates')
    live.add_argument('--jobs', type=int, default=1)
    for flag, default in zip(('proposal', 'teacher-low', 'teacher-high'), DEFAULT_NODES):
        live.add_argument(f'--{flag}-nodes', type=int, default=default)
    live.add_argument('--qsearch-leaf', type=Path, default=WT / 'target/qsearch-leaf/release/qsearch_leaf')
    archive = commands.add_parser('archive')
    archive.add_argument('--final', type=Path, default=DATA / 'phase4/final')
    archive.add_argument('--out', type=Path, default=Path(__file__).with_name('ab-rule.json'))
    args = parser.parse_args()
    if args.command == 'archive':
        result = archived_rule(args.final)
        write_json(args.out, result)
        print(json.dumps(result['engines'], ensure_ascii=False, indent=2))
    else:
        run(args)


if __name__ == '__main__':
    main()
