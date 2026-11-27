#!/usr/bin/env python3
"""minaseのバイナリの探索の結果が基準の記録と一致するかを確かめる（設計書movegen-speedup-3の「検証」の節）。

  verify_identical.py record --binary <基準> --net <ネット> --cwd <作業ディレクトリ> --out <記録.json>
  verify_identical.py check  --binary <候補> --net <ネット> --cwd <作業ディレクトリ> --baseline <記録.json>

4種類の出力を比べる。探索ワーカーはすべて1である。
1. bench深さ16（規則セットJCSA）の15局面の行の、深さ、ノード数、最善手、および評価値。
2. 教師生成（規則セットlishogi、各局面2万ノード）のファイルのSHA-256、対局数、および探索ノード数の合計。
   シード9010200000の2局と、シード424242の4局。
3. 7つの規則セットのそれぞれで、benchの15局面を6万ノードで探索したときの、最後の`info depth`の行の
   深さ、ノード数、評価値、および読み筋と、`bestmove`の行。
4. 同じ7つの規則セットのそれぞれで、benchの15局面の`perft 2 divide`の、最初の手ごとの行（順序を含む）と、
   深さごとの経路数の行および総数の行。
経過時間と毎秒ノード数は比べない。行や欄が欠けた場合は不一致とする。

教師生成は作業ディレクトリのコミットを教師のファイルに書くので、`--cwd`には基準と候補で同じ、
変更のない作業ディレクトリ（基準のコミットのdetachedなworktree）を渡す。出力先は実行ごとに空の一時ディレクトリである。
不一致があれば終了コード1で終わる。
"""
import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile

RULE_SETS = [
    'JCSA',
    'lishogi',
    'L1,L3,P0,P5,P6,R2,E1,E2',
    'L0,P0,R3,E0',
    'L0,P1,R1,E0',
    'L1,L2,P2,P3,P4,R2,E0',
    'L0,L4,P0,R1,E0',
]
TEACHER_RUNS = [dict(seed=9010200000, games=2), dict(seed=424242, games=4)]
BENCH_POSITIONS = 15
USI_NODES = 60000
POSITION_LINE = re.compile(r'^info string position=\S+ depth=\d+ nodes=\d+ best=\S+ score=-?\d+ elapsed=[\d.]+s$')


def bench_positions(cwd):
    src = open(os.path.join(cwd, 'crates/minase/src/diagnostics/bench.rs'), encoding='utf-8').read()
    positions = re.findall(r'sfen: "([^"]+)"', src)
    if len(positions) != BENCH_POSITIONS:
        raise RuntimeError(f'expected {BENCH_POSITIONS} bench positions, found {len(positions)}')
    return positions


def run_usi(binary, cwd, script, timeout):
    p = subprocess.run([binary, '--protocol', 'usi'], cwd=cwd, input=script, capture_output=True, text=True, timeout=timeout)
    if p.returncode != 0:
        raise RuntimeError(f'engine exited with {p.returncode}: {p.stderr[-500:]}')
    for line in p.stdout.splitlines():
        if line.startswith('info string error'):
            raise RuntimeError(line)
    return p.stdout.splitlines()


def bench(binary, cwd, net):
    script = (f'setoption name RuleSet value JCSA\nsetoption name EvalFile value {net}\nisready\n'
              'bench depth 16 threads 1 repetitions 1\nquit\n')
    out = run_usi(binary, cwd, script, 600)
    rows = [l for l in out if l.startswith('info string position=')]
    if len(rows) != BENCH_POSITIONS or not all(POSITION_LINE.match(l) for l in rows):
        raise RuntimeError('bench output is incomplete:\n' + '\n'.join(out[-20:]))
    return [l.split(' elapsed=')[0] for l in rows]


def teacher(binary, cwd, net, seed, games):
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, 'out')
        p = subprocess.run([binary, 'data', 'selfplay', 'generate', '--eval-file', net, '--output-dir', out,
                            '--chunk-games', str(games), '--games', str(games), '--seed', str(seed),
                            '--nodes', '20000', '--random-moves', '0', '--concurrency', '1', '--hash-mb', '16'],
                           cwd=cwd, capture_output=True, text=True, timeout=1800)
        if p.returncode != 0:
            raise RuntimeError(f'teacher generation failed: {p.stdout[-500:]} {p.stderr[-500:]}')
        log = p.stdout + p.stderr
        nodes = re.search(r'searched_nodes_total: (\d+)', log)
        played = re.search(r'^games: (\d+)$', log, re.MULTILINE)
        if nodes is None or played is None:
            raise RuntimeError('teacher generation summary is incomplete:\n' + log[-1000:])
        data = open(os.path.join(out, 'teacher.mnsd'), 'rb').read()
        return dict(seed=seed, games=int(played[1]), nodes=int(nodes[1]), sha256=hashlib.sha256(data).hexdigest())


def fixed_nodes(binary, cwd, net, rules, positions):
    # bestmoveを受け取ってから次の局面を送る。探索中に次のコマンドを送ると、探索中の入力として扱われる。
    proc = subprocess.Popen([binary, '--protocol', 'usi'], cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, bufsize=1)

    def send(line):
        proc.stdin.write(line + '\n')
        proc.stdin.flush()

    def until(prefix):
        last = None
        while True:
            line = proc.stdout.readline()
            if not line:
                raise RuntimeError(f'{rules}: engine exited')
            line = line.strip()
            if line.startswith('info string error'):
                raise RuntimeError(f'{rules}: {line}')
            if line.startswith(prefix):
                return line, last
            if line.startswith('info depth') and ' nodes ' in line and ' pv ' in line:
                last = line

    send(f'setoption name RuleSet value {rules}')
    send(f'setoption name EvalFile value {net}')
    send('setoption name Threads value 1')
    send('isready')
    until('readyok')
    rows = []
    for sfen in positions:
        send('usinewgame')
        send(f'position sfen {sfen} - 1')
        send(f'go nodes {USI_NODES}')
        best, info = until('bestmove')
        if info is None:
            raise RuntimeError(f'{rules}: bestmove without info depth line')
        rows.append([re.sub(r' (time|nps|hashfull) \d+', '', info), best])
    send('quit')
    if proc.wait(timeout=60) != 0:
        raise RuntimeError(f'{rules}: engine exited with {proc.returncode}')
    return rows


def perft(binary, cwd, net, rules, positions):
    script = [f'setoption name RuleSet value {rules}', f'setoption name EvalFile value {net}', 'isready']
    for sfen in positions:
        script += [f'position sfen {sfen} - 1', 'perft 2 divide']
    script.append('quit')
    out = run_usi(binary, cwd, '\n'.join(script) + '\n', 600)
    blocks, current = [], []
    for line in out:
        if line.startswith('info string move ') or line.startswith('info string perft('):
            current.append(line)
        elif line.startswith('info string total nodes:'):
            current.append(line)
            blocks.append(current)
            current = []
    if len(blocks) != len(positions) or any(
            sum(l.startswith('info string perft(') for l in b) != 2 for b in blocks):
        raise RuntimeError(f'{rules}: perft output is incomplete')
    return blocks


def run_all(binary, cwd, net):
    positions = bench_positions(cwd)
    res = {'bench': bench(binary, cwd, net), 'teacher': [], 'fixed_nodes': {}, 'perft': {}}
    for run in TEACHER_RUNS:
        res['teacher'].append(teacher(binary, cwd, net, run['seed'], run['games']))
    for rules in RULE_SETS:
        res['fixed_nodes'][rules] = fixed_nodes(binary, cwd, net, rules, positions)
        res['perft'][rules] = perft(binary, cwd, net, rules, positions)
    return res


def compare(cur, base):
    bad = 0
    if cur['bench'] != base['bench']:
        bad += 1
        for x, y in zip(cur['bench'], base['bench']):
            if x != y:
                print('BENCH DIFF\n cand', x, '\n base', y)
    for t, u in zip(cur['teacher'], base['teacher']):
        if t != u:
            bad += 1
            print('TEACHER DIFF\n cand', t, '\n base', u)
    for key in ('fixed_nodes', 'perft'):
        for rules in RULE_SETS:
            if cur[key][rules] != base[key][rules]:
                bad += 1
                for i, (x, y) in enumerate(zip(cur[key][rules], base[key][rules])):
                    if x != y:
                        print(f'{key.upper()} DIFF {rules} position {i}\n cand', x, '\n base', y)
    return bad


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('mode', choices=['record', 'check'])
    ap.add_argument('--binary', required=True)
    ap.add_argument('--net', required=True)
    ap.add_argument('--cwd', required=True)
    ap.add_argument('--out')
    ap.add_argument('--baseline')
    a = ap.parse_args()
    binary, net, cwd = (os.path.abspath(p) for p in (a.binary, a.net, a.cwd))
    cur = run_all(binary, cwd, net)
    if a.mode == 'record':
        with open(a.out, 'w', encoding='utf-8') as f:
            json.dump(cur, f, indent=1, ensure_ascii=False)
        print('recorded', a.out)
        return
    with open(a.baseline, encoding='utf-8') as f:
        base = json.load(f)
    bad = compare(cur, base)
    print('IDENTICAL' if not bad else f'DIFFERENT ({bad} groups)')
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
