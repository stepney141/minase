#!/usr/bin/env python3
"""サイクル数の対比較を行い、集計する（設計書movegen-speedup-3の「探索木を変えない段階の採否」の節）。

  paired.py run --workload bench|teacher --pairs <組数> --parent <親> --candidate <候補>
                --net <ネット> --cwd <作業ディレクトリ> --out <結果.jsonl> [--cpu 3]
  paired.py judge --results <結果.jsonl>     測定単位の採否の判定
  paired.py final --results <結果.jsonl>     最終測定（速度比と最低受入条件）
  paired.py pairs --results <結果.jsonl>     同じバイナリどうしの対比較から組数を定める

各組は親と候補を1回ずつ実行し、奇数番目の組は親から、偶数番目の組は候補から実行する。
各実行は`taskset -c <cpu> perf stat`でサイクル数、命令数、および分岐予測ミスの回数を数える。
benchは規則セットJCSAの深さ16、教師生成は規則セットlishogiのシード9010200000の2局（各局面2万ノード）である。
benchは局面別の行（経過時間を除く）が、教師生成はファイルのSHA-256と探索ノード数の合計が、
組の中の2つの実行で一致することを確かめ、一致しない組は結果に印を付ける。
教師生成は作業ディレクトリのコミットを教師のファイルに書くので、`--cwd`には変更のない作業ディレクトリを渡す。

組ごとの対数比は ln(候補のサイクル数 ÷ 親のサイクル数) であり、速度比は exp(−対数比の平均) である。
"""
import argparse
import hashlib
import json
import math
import os
import shutil
import statistics
import subprocess
import sys
import tempfile

EVENTS = {'cpu_core/cycles/u': 'cycles', 'cpu_core/instructions/u': 'instructions',
          'cpu_core/branch-misses/u': 'branch_misses'}
BENCH_LIMIT = math.log(1.001)
TEACHER_LIMIT = math.log(1.003)
BENCH_MAX_PAIRS = 40
TEACHER_MAX_PAIRS = 60
TEACHER_REGRESSION = math.log(1.005)
MIN_SPEEDUP = 1.02


# Student t 分布の分位点（scipyに依存しないため、正則化不完全ベータ関数から求める）。
def _betacf(a, b, x):
    tiny = 1e-300
    qab, qap, qam = a + b, a + 1.0, a - 1.0
    c, d = 1.0, 1.0 - qab * x / qap
    d = 1.0 / (d if abs(d) > tiny else tiny)
    h = d
    for m in range(1, 300):
        m2 = 2 * m
        aa = m * (b - m) * x / ((qam + m2) * (a + m2))
        d = 1.0 + aa * d
        d = 1.0 / (d if abs(d) > tiny else tiny)
        c = 1.0 + aa / c
        c = c if abs(c) > tiny else tiny
        h *= d * c
        aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2))
        d = 1.0 + aa * d
        d = 1.0 / (d if abs(d) > tiny else tiny)
        c = 1.0 + aa / c
        c = c if abs(c) > tiny else tiny
        delta = d * c
        h *= delta
        if abs(delta - 1.0) < 1e-15:
            return h
    raise RuntimeError('betacf did not converge')


def _betainc(a, b, x):
    if x <= 0.0:
        return 0.0
    if x >= 1.0:
        return 1.0
    lbeta = math.lgamma(a + b) - math.lgamma(a) - math.lgamma(b) + a * math.log(x) + b * math.log(1.0 - x)
    if x < (a + 1.0) / (a + b + 2.0):
        return math.exp(lbeta) * _betacf(a, b, x) / a
    return 1.0 - math.exp(lbeta) * _betacf(b, a, 1.0 - x) / b


def t_cdf(t, df):
    tail = 0.5 * _betainc(df / 2.0, 0.5, df / (df + t * t))
    return 1.0 - tail if t > 0 else tail


def t_quantile(p, df):
    lo, hi = 0.0, 1000.0
    for _ in range(200):
        mid = (lo + hi) / 2.0
        if t_cdf(mid, df) < p:
            lo = mid
        else:
            hi = mid
    return (lo + hi) / 2.0


def perf_run(cmd, cwd, cpu, stdin_path, stdout_path):
    with tempfile.NamedTemporaryFile('r', suffix='.stat', delete=False) as stat:
        stat_path = stat.name
    try:
        full = ['taskset', '-c', str(cpu), 'perf', 'stat', '-x,', '-e', ','.join(EVENTS), '-o', stat_path, '--'] + cmd
        with open(stdin_path or os.devnull) as fin, open(stdout_path, 'w') as fout:
            p = subprocess.run(full, cwd=cwd, stdin=fin, stdout=fout, stderr=subprocess.STDOUT)
        if p.returncode != 0:
            raise RuntimeError(f'command failed ({p.returncode}): {" ".join(cmd)}; see {stdout_path}')
        counts = {}
        for line in open(stat_path):
            fields = line.strip().split(',')
            if len(fields) > 2 and fields[2] in EVENTS:
                counts[EVENTS[fields[2]]] = int(fields[0])
        if set(counts) != set(EVENTS.values()):
            raise RuntimeError(f'perf output is incomplete: {open(stat_path).read()}')
        return counts
    finally:
        os.unlink(stat_path)


def run_bench(binary, args, work):
    usi = os.path.join(work, 'bench16.usi')
    with open(usi, 'w') as f:
        f.write(f'setoption name RuleSet value JCSA\nsetoption name EvalFile value {args.net}\nisready\n'
                'bench depth 16 threads 1 repetitions 1\nquit\n')
    log = os.path.join(work, 'bench.out')
    counts = perf_run([binary, '--protocol', 'usi'], args.cwd, args.cpu, usi, log)
    rows = [l.split(' elapsed=')[0] for l in open(log) if l.startswith('info string position=')]
    if len(rows) != 15:
        raise RuntimeError(f'bench output is incomplete: {log}')
    counts['identity'] = hashlib.sha256('\n'.join(rows).encode()).hexdigest()
    return counts


def run_teacher(binary, args, work):
    out = os.path.join(work, 'teacher')
    log = os.path.join(work, 'teacher.out')
    counts = perf_run([binary, 'data', 'selfplay', 'generate', '--eval-file', args.net, '--output-dir', out,
                       '--chunk-games', '2', '--games', '2', '--seed', '9010200000', '--nodes', '20000',
                       '--random-moves', '0', '--concurrency', '1', '--hash-mb', '16'],
                      args.cwd, args.cpu, None, log)
    nodes = [l.split(':')[1].strip() for l in open(log) if l.startswith('searched_nodes_total:')]
    if len(nodes) != 1:
        raise RuntimeError(f'teacher summary is incomplete: {log}')
    sha = hashlib.sha256(open(os.path.join(out, 'teacher.mnsd'), 'rb').read()).hexdigest()
    counts['identity'] = f'{sha} nodes={nodes[0]}'
    shutil.rmtree(out)
    return counts


def cmd_run(args):
    args.net = os.path.abspath(args.net)
    args.cwd = os.path.abspath(args.cwd)
    binaries = {'parent': os.path.abspath(args.parent), 'candidate': os.path.abspath(args.candidate)}
    runner = run_bench if args.workload == 'bench' else run_teacher
    with open(args.out, 'a') as out:
        for pair in range(1, args.pairs + 1):
            order = ['parent', 'candidate'] if pair % 2 == 1 else ['candidate', 'parent']
            row = {'workload': args.workload, 'pair': pair, 'order': order,
                   'parent_binary': binaries['parent'], 'candidate_binary': binaries['candidate']}
            for label in order:
                with tempfile.TemporaryDirectory(prefix='ms3-paired-') as work:
                    row[label] = runner(binaries[label], args, work)
            row['identical'] = row['parent']['identity'] == row['candidate']['identity']
            row['log_ratio'] = math.log(row['candidate']['cycles'] / row['parent']['cycles'])
            out.write(json.dumps(row) + '\n')
            out.flush()
            print(f"{args.workload} pair {pair}: log_ratio={row['log_ratio']:+.5f} identical={row['identical']}",
                  flush=True)


def load(path):
    rows = [json.loads(l) for l in open(path) if l.strip()]
    by = {}
    for r in rows:
        by.setdefault(r['workload'], []).append(r)
    return by


def stats(rows):
    xs = [r['log_ratio'] for r in rows]
    n = len(xs)
    mean = statistics.fmean(xs)
    sd = statistics.stdev(xs)
    se = sd / math.sqrt(n)
    t2 = t_quantile(0.975, n - 1)
    t1 = t_quantile(0.95, n - 1)
    def med(label, key):
        return statistics.median(r[label][key] for r in rows)
    return dict(n=n, mean=mean, sd=sd, lo2=mean - t2 * se, hi2=mean + t2 * se, hi1=mean + t1 * se,
                speed=math.exp(-mean), speed_lo=math.exp(-(mean + t2 * se)), speed_hi=math.exp(-(mean - t2 * se)),
                identical=all(r['identical'] for r in rows),
                parent_cycles=med('parent', 'cycles'), candidate_cycles=med('candidate', 'cycles'),
                instr_change=med('candidate', 'instructions') / med('parent', 'instructions') - 1,
                miss_change=med('candidate', 'branch_misses') / med('parent', 'branch_misses') - 1)


def report(name, s):
    print(f"{name}: pairs={s['n']} identical={s['identical']}")
    print(f"  log ratio mean={s['mean']:+.5f} sd={s['sd']:.5f} two-sided 95% CI=[{s['lo2']:+.5f}, {s['hi2']:+.5f}]"
          f" one-sided 95% upper={s['hi1']:+.5f}")
    print(f"  speed ratio={s['speed']:.4f} two-sided 95% CI=[{s['speed_lo']:.4f}, {s['speed_hi']:.4f}]")
    print(f"  median cycles parent={s['parent_cycles']:,} candidate={s['candidate_cycles']:,}"
          f" instructions {s['instr_change']:+.3%} branch-misses {s['miss_change']:+.3%}")


def cmd_judge(args):
    by = load(args.results)
    b, t = stats(by['bench']), stats(by['teacher'])
    report('bench', b)
    report('teacher', t)
    bench_ok = b['identical'] and b['hi2'] < 0
    teacher_ok = t['identical'] and t['hi1'] <= TEACHER_REGRESSION
    print(f"bench improvement (two-sided upper < 0): {bench_ok}")
    print(f"teacher regression guard (one-sided upper <= ln 1.005 = {TEACHER_REGRESSION:.5f}): {teacher_ok}")
    print('ACCEPT' if bench_ok and teacher_ok else 'REJECT')


def cmd_final(args):
    by = load(args.results)
    ok = True
    for name in ('bench', 'teacher'):
        s = stats(by[name])
        report(name, s)
        cond = s['identical'] and s['speed_lo'] >= MIN_SPEEDUP
        print(f"  speed ratio lower bound >= {MIN_SPEEDUP}: {cond}")
        ok = ok and cond
    print('MEETS MINIMUM' if ok else 'BELOW MINIMUM')


def required_pairs(sd, limit, quantile, cap):
    for n in range(2, cap + 1, 2):
        if t_quantile(quantile, n - 1) * sd / math.sqrt(n) <= limit:
            return n
    return cap


def cmd_pairs(args):
    by = load(args.results)
    b, t = stats(by['bench']), stats(by['teacher'])
    report('bench', b)
    report('teacher', t)
    nb = required_pairs(b['sd'], BENCH_LIMIT, 0.975, BENCH_MAX_PAIRS)
    nt = required_pairs(t['sd'], TEACHER_LIMIT, 0.95, TEACHER_MAX_PAIRS)
    print(f"bench pairs: {nb} (half width {t_quantile(0.975, nb - 1) * b['sd'] / math.sqrt(nb):.5f},"
          f" target {BENCH_LIMIT:.5f}, cap {BENCH_MAX_PAIRS})")
    print(f"teacher pairs: {nt} (half width {t_quantile(0.95, nt - 1) * t['sd'] / math.sqrt(nt):.5f},"
          f" target {TEACHER_LIMIT:.5f}, cap {TEACHER_MAX_PAIRS})")


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest='cmd', required=True)
    r = sub.add_parser('run')
    r.add_argument('--workload', choices=['bench', 'teacher'], required=True)
    r.add_argument('--pairs', type=int, required=True)
    r.add_argument('--parent', required=True)
    r.add_argument('--candidate', required=True)
    r.add_argument('--net', required=True)
    r.add_argument('--cwd', required=True)
    r.add_argument('--out', required=True)
    r.add_argument('--cpu', type=int, default=3)
    for name in ('judge', 'final', 'pairs'):
        s = sub.add_parser(name)
        s.add_argument('--results', required=True)
    args = ap.parse_args()
    {'run': cmd_run, 'judge': cmd_judge, 'final': cmd_final, 'pairs': cmd_pairs}[args.cmd](args)


if __name__ == '__main__':
    sys.exit(main())
