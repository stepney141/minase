#!/usr/bin/env python3
"""Check that a minase binary keeps every search result of the baseline binary.

  verify_identical.py record --binary <baseline> --out baseline.json
  verify_identical.py check  --binary <candidate> --baseline baseline.json [--quick]

Runs from a fixed detached worktree (default `cwd/` next to this script) because the teacher header stores `git rev-parse HEAD`.
Checks: `dev bench --depth 9` lines, teacher files (SHA-256) for two seeds, and fixed-node USI searches
under several rule sets (final info line and bestmove). Timing fields are excluded. Exit code 1 on any difference.
"""
import argparse, hashlib, json, os, re, subprocess, sys, tempfile

RULES = ['engine-default', 'lishogi', 'L1,L3,P0,P5,P6,R2,E1,E2', 'L0,P0,R3,E0', 'L0,P1,R1,E0', 'L1,L2,P2,P3,P4,R2,E0']
GAMES = [dict(seed=9010200000, games=2, max_ply=4000), dict(seed=424242, games=4, max_ply=500)]

def bench(binary, cwd):
    out = subprocess.run([binary, 'dev', 'bench', '--depth', '9', '--threads', '1', '--repetitions', '1'], cwd=cwd, capture_output=True, text=True, check=True).stdout
    lines = [l.split(' elapsed=')[0] for l in out.splitlines() if l.startswith('position=')]
    assert len(lines) == 15, out[-500:]
    return lines

def teacher(binary, cwd, seed, games, max_ply):
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, 't.mnsd')
        p = subprocess.run([binary, 'data', 'selfplay', 'generate', '--output', path, '--seed', str(seed), '--games', str(games), '--concurrency', '1', '--nodes', '20000', '--random-moves', '0', '--max-ply', str(max_ply), '--hash-mb', '16', '--allow-dirty'], cwd=cwd, capture_output=True, text=True, check=True)
        nodes = int(re.search(r'searched_nodes_total: (\d+)', p.stdout)[1])
        return dict(sha256=hashlib.sha256(open(path, 'rb').read()).hexdigest(), nodes=nodes)

def usi_positions(cwd):
    src = open(os.path.join(cwd, 'crates/minase/src/bin/minase/bench.rs'), encoding='utf-8').read()
    return re.findall(r'sfen: "([^"]+)"', src)

def usi(binary, cwd, rules, positions, nodes):
    # A fresh process per rule set; usinewgame resets the tables between positions.
    results = []
    proc = subprocess.Popen([binary, '--protocol', 'usi', '--rules', rules], cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
    def send(s): proc.stdin.write(s + '\n'); proc.stdin.flush()
    def until(prefix):
        last = None
        while True:
            l = proc.stdout.readline()
            if not l: raise RuntimeError(f'engine exited for rules {rules}')
            l = l.strip()
            if l.startswith('info string error'): raise RuntimeError(f'{rules}: {l}')
            if l.startswith(prefix): return l, last
            if l.startswith('info') and ' nodes ' in l: last = l
    send('usi'); until('usiok')
    send('setoption name Threads value 1'); send('isready'); until('readyok')
    for sfen in positions:
        send('usinewgame'); send(f'position sfen {sfen} - 1'); send(f'go nodes {nodes}')
        best, info = until('bestmove')
        info = re.sub(r' (time|nps|hashfull) \d+', '', info or '')
        results.append([info, best])
    send('quit'); proc.wait(timeout=30)
    return results

def run_all(binary, cwd, quick):
    res = {'bench': bench(binary, cwd), 'teacher': [], 'usi': {}}
    for g in (GAMES[:1] if quick else GAMES):
        res['teacher'].append(teacher(binary, cwd, g['seed'], g['games'], g['max_ply']))
    positions = usi_positions(cwd)
    for rules in RULES:
        res['usi'][rules] = usi(binary, cwd, rules, positions[1:6:2] if quick else positions[1:], 60000)
    return res

def main():
    import signal
    signal.alarm(1200)
    ap = argparse.ArgumentParser()
    ap.add_argument('mode', choices=['record', 'check'])
    ap.add_argument('--binary', required=True); ap.add_argument('--out'); ap.add_argument('--baseline')
    ap.add_argument('--cwd', default=os.path.join(os.path.dirname(os.path.abspath(__file__)), 'cwd')); ap.add_argument('--quick', action='store_true')
    a = ap.parse_args()
    binary = os.path.abspath(a.binary); cwd = os.path.abspath(a.cwd)
    if a.mode == 'record':
        json.dump(run_all(binary, cwd, False), open(a.out, 'w'), indent=1, ensure_ascii=False)
        print('recorded', a.out); return
    base = json.load(open(a.baseline))
    cur = run_all(binary, cwd, a.quick)
    bad = 0
    if cur['bench'] != base['bench']:
        bad += 1
        for x, y in zip(cur['bench'], base['bench']):
            if x != y: print('BENCH DIFF\n cand', x, '\n base', y)
    for i, t in enumerate(cur['teacher']):
        if t != base['teacher'][i]:
            bad += 1; print('TEACHER DIFF', i, t, base['teacher'][i])
    for rules, rows in cur['usi'].items():
        expected = base['usi'][rules]
        if a.quick:
            positions = usi_positions(cwd)
            idx = list(range(1, len(positions)))
            keep = [idx.index(k) for k in range(1, 6, 2)]
            expected = [expected[k] for k in keep]
        if rows != expected:
            bad += 1
            for x, y in zip(rows, expected):
                if x != y: print('USI DIFF', rules, '\n cand', x, '\n base', y)
    print('IDENTICAL' if not bad else f'DIFFERENT ({bad} groups)')
    sys.exit(1 if bad else 0)

if __name__ == '__main__':
    main()
