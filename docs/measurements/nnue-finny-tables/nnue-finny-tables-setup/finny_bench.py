#!/usr/bin/env python3
"""Finny tablesの設計書（docs/plans/nnue-finny-tables.md）のサイクル数の対比較と探索結果の照合。

サブコマンド:
  pairs    基準と候補のバイナリを1組ずつ交互に実行し、組ごとのサイクル数と出力の要約をJSON Linesへ追記する。
           組iでは、iが偶数なら基準を先に、奇数なら候補を先に実行する。
           各組で、2つの実行の探索結果（benchは局面別の深さ、ノード数、最善手、評価値。
           教師生成は対局数、探索ノード数の合計、教師のファイルのSHA-256）が一致しなければ終了コード1で止まる。
           基準どうしの算定では、--baselineと--candidateに同じバイナリを渡す。
  loadonly ネットの読み込みだけを行う起動のサイクル数を測り、network_sha256、readyok、終了コード0を確かめる。
  summary  JSON Linesから組ごとの値（教師生成は差d、benchは比の自然対数r）の平均、標準偏差、信頼区間を出す。
  npairs   基準どうしの算定のJSON Linesから、設計書のフェーズ4の手順2で組の数nを求める。

教師生成は、バイナリのコミットではなく作業ディレクトリのコミットをファイルに書くので、
--cwdには変更のないgitの作業ディレクトリを渡し、両方のバイナリを同じ場所から起動する。
組iの基本シードは`--seed-base + i×--games`である。

外部のライブラリを使わないので、t分布の分位点は正則化不完全ベータ関数の連分数と二分法で求める。
"""

import argparse
import hashlib
import json
import math
import os
import re
import shutil
import statistics
import subprocess
import sys
import tempfile

EVENTS = [
    "cpu_core/cycles/u",
    "cpu_core/instructions/u",
    "cpu_core/LLC-load-misses/u",
    "cpu_core/dTLB-load-misses/u",
]


def perf_run(cmd, cpu, cwd, stdin_text=None):
    """`taskset -c <cpu> perf stat`の下でcmdを実行し、(標準出力, 事象ごとの計数)を返す。"""
    with tempfile.NamedTemporaryFile("r", suffix=".perf", delete=False) as handle:
        stat_path = handle.name
    try:
        full = ["taskset", "-c", cpu, "perf", "stat", "-x,", "-o", stat_path, "-e", ",".join(EVENTS), "--", *cmd]
        proc = subprocess.run(full, cwd=cwd, input=stdin_text, capture_output=True, text=True)
        if proc.returncode != 0:
            raise SystemExit(f"command failed ({proc.returncode}): {' '.join(cmd)}\n{proc.stdout[-2000:]}\n{proc.stderr[-2000:]}")
        counts = {}
        with open(stat_path, encoding="utf-8") as stat:
            for line in stat:
                fields = line.strip().split(",")
                if line.startswith("#") or len(fields) < 3 or fields[2] not in EVENTS:
                    continue
                name = fields[2].split("/")[1]
                if fields[0] in ("<not counted>", "<not supported>"):
                    raise SystemExit(f"perf did not count {fields[2]}; is CPU {cpu} a performance core?")
                counts[name] = int(fields[0])
        missing = {e.split("/")[1] for e in EVENTS[:2]} - counts.keys()
        if missing:
            raise SystemExit(f"perf output lacks {missing}")
        return proc.stdout, counts
    finally:
        os.remove(stat_path)


def bench_script(net, depth):
    return (
        "setoption name RuleSet value JCSA\n"
        f"setoption name EvalFile value {net}\n"
        "isready\n"
        f"bench depth {depth} threads 1 repetitions 1\n"
        "quit\n"
    )


def run_bench(binary, net, depth, cpu, cwd):
    stdout, counts = perf_run([binary, "--protocol", "usi"], cpu, cwd, bench_script(net, depth))
    lines = stdout.splitlines()
    if not lines or lines[-1] != "info string end":
        raise SystemExit(f"bench did not end with 'info string end':\n{stdout[-2000:]}")
    positions = [re.sub(r" elapsed=\S+", "", l) for l in lines if l.startswith("info string position=")]
    if len(positions) != 15:
        raise SystemExit(f"expected 15 bench positions, got {len(positions)}")
    accumulators = next((l for l in lines if l.startswith("info string accumulators:")), None)
    return {"result": positions, "accumulators": accumulators}, counts


def run_generate(binary, net, games, seed, cpu, cwd):
    out = tempfile.mkdtemp(prefix="finny-gen-")
    shutil.rmtree(out)
    try:
        cmd = [
            binary, "data", "selfplay", "generate", "--eval-file", net, "--output-dir", out,
            "--chunk-games", str(games), "--games", str(games), "--seed", str(seed),
            "--nodes", "20000", "--random-moves", "0", "--concurrency", "1", "--hash-mb", "16",
        ]
        stdout, counts = perf_run(cmd, cpu, cwd)
        summary = dict(re.findall(r"^(\w+): (\S+)$", stdout, re.M))
        with open(os.path.join(out, "teacher.mnsd"), "rb") as teacher:
            sha = hashlib.sha256(teacher.read()).hexdigest()
        result = {
            "games_completed": int(summary["games_completed"]),
            "searched_nodes_total": int(summary["searched_nodes_total"]),
            "teacher_sha256": sha,
        }
        return {"result": result}, counts
    finally:
        shutil.rmtree(out, ignore_errors=True)


def cmd_pairs(args):
    names = (("baseline", args.baseline), ("candidate", args.candidate))
    existing = 0
    if os.path.exists(args.out):
        with open(args.out, encoding="utf-8") as f:
            existing = sum(1 for _ in f)
    if existing:
        raise SystemExit(f"{args.out} already has {existing} pairs; use a new file")
    with open(args.out, "a", encoding="utf-8") as out:
        for i in range(args.pairs):
            order = names if i % 2 == 0 else names[::-1]
            record = {"pair": i, "first": order[0][0]}
            seed = args.seed_base + i * args.games if args.workload == "generate" else None
            if seed is not None:
                record["seed"] = seed
            outputs = {}
            for name, binary in order:
                if args.workload == "generate":
                    output, counts = run_generate(binary, args.net, args.games, seed, args.cpu, args.cwd)
                else:
                    output, counts = run_bench(binary, args.net, args.depth, args.cpu, args.cwd)
                outputs[name] = output
                record[name] = counts
            same = outputs["baseline"]["result"] == outputs["candidate"]["result"]
            record["identical"] = same
            record["result"] = outputs["baseline"]["result"]
            if not same:
                record["candidate_result"] = outputs["candidate"]["result"]
            for name in ("baseline", "candidate"):
                if outputs[name].get("accumulators"):
                    record[f"{name}_accumulators"] = outputs[name]["accumulators"]
            out.write(json.dumps(record) + "\n")
            out.flush()
            value = pair_value(record, args.workload)
            print(f"pair {i} first={order[0][0]} value={value:.6g} identical={same}", flush=True)
            if not same:
                print("search results differ", file=sys.stderr)
                sys.exit(1)
            if args.early_stop and i + 1 == 8:
                values = [pair_value(r, args.workload) for r in load(args.out)]
                lower = mean_bound(values, 0.95, lower=True)
                print(f"early-stop check after 8 pairs: one-sided 95% lower bound of mean d = {lower:.6g}", flush=True)
                if lower >= 0:
                    print("EARLY STOP: the candidate does not reduce cycles", flush=True)
                    sys.exit(2)


def cmd_loadonly(args):
    script = (
        "setoption name RuleSet value JCSA\n"
        f"setoption name EvalFile value {args.net}\n"
        "isready\n"
        "quit\n"
    )
    for _ in range(args.repeat):
        stdout, counts = perf_run([args.binary, "--protocol", "usi"], args.cpu, args.cwd, script)
        lines = stdout.splitlines()
        if f"info string network_sha256 {args.sha256}" not in lines or "readyok" not in lines:
            raise SystemExit(f"unexpected load output:\n{stdout}")
        print(json.dumps(counts), flush=True)


def load(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(l) for l in f if l.strip()]


def pair_value(record, workload):
    base, cand = record["baseline"]["cycles"], record["candidate"]["cycles"]
    return cand - base if workload == "generate" else math.log(cand / base)


# --- Student t distribution -------------------------------------------------


def _betacf(a, b, x):
    tiny = 1e-300
    qab, qap, qam = a + b, a + 1, a - 1
    c, d = 1.0, 1 - qab * x / qap
    d = 1 / (d if abs(d) > tiny else tiny)
    h = d
    for m in range(1, 1000):
        m2 = 2 * m
        aa = m * (b - m) * x / ((qam + m2) * (a + m2))
        d = 1 + aa * d
        d = 1 / (d if abs(d) > tiny else tiny)
        c = 1 + aa / c
        c = c if abs(c) > tiny else tiny
        h *= d * c
        aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2))
        d = 1 + aa * d
        d = 1 / (d if abs(d) > tiny else tiny)
        c = 1 + aa / c
        c = c if abs(c) > tiny else tiny
        delta = d * c
        h *= delta
        if abs(delta - 1) < 1e-15:
            return h
    raise ArithmeticError("betacf did not converge")


def _betainc(a, b, x):
    if x <= 0:
        return 0.0
    if x >= 1:
        return 1.0
    front = math.exp(math.lgamma(a + b) - math.lgamma(a) - math.lgamma(b) + a * math.log(x) + b * math.log1p(-x))
    if x < (a + 1) / (a + b + 2):
        return front * _betacf(a, b, x) / a
    return 1 - front * _betacf(b, a, 1 - x) / b


def t_cdf(t, df):
    tail = 0.5 * _betainc(df / 2, 0.5, df / (df + t * t))
    return 1 - tail if t >= 0 else tail


def t_quantile(q, df):
    """t分布の下側確率qの分位点（0.5 < q < 1）。"""
    lo, hi = 0.0, 1000.0
    for _ in range(200):
        mid = (lo + hi) / 2
        if t_cdf(mid, df) < q:
            lo = mid
        else:
            hi = mid
    return (lo + hi) / 2


def mean_bound(values, q, lower):
    n = len(values)
    half = t_quantile(q, n - 1) * statistics.stdev(values) / math.sqrt(n)
    mean = statistics.fmean(values)
    return mean - half if lower else mean + half


# --- summaries ----------------------------------------------------------------


def workload_of(records):
    return "generate" if "seed" in records[0] else "bench"


def cmd_summary(args):
    records = load(args.path)
    workload = workload_of(records)
    values = [pair_value(r, workload) for r in records]
    n = len(values)
    mean = statistics.fmean(values)
    sd = statistics.stdev(values)
    base_mean = statistics.fmean(r["baseline"]["cycles"] for r in records)
    cand_mean = statistics.fmean(r["candidate"]["cycles"] for r in records)
    result = {
        "workload": workload,
        "pairs": n,
        "identical_all": all(r["identical"] for r in records),
        "baseline_cycles_mean": base_mean,
        "candidate_cycles_mean": cand_mean,
        "mean": mean,
        "sd": sd,
    }
    if workload == "generate":
        half = t_quantile(0.975, n - 1) * sd / math.sqrt(n)
        result.update(
            ci95_two_sided=[mean - half, mean + half],
            mean_relative=mean / base_mean,
            total_difference=mean * n,
            accept=mean + half < 0,
        )
    else:
        upper = mean + t_quantile(0.95, n - 1) * sd / math.sqrt(n)
        result.update(
            ratio=math.exp(mean),
            upper95_one_sided=upper,
            upper95_ratio=math.exp(upper),
            accept=upper <= math.log(1.005),
        )
    for event in ("instructions", "LLC-load-misses", "dTLB-load-misses"):
        if all(event in r["baseline"] and event in r["candidate"] for r in records):
            result[f"{event}_ratio_of_means"] = statistics.fmean(r["candidate"][event] for r in records) / statistics.fmean(
                r["baseline"][event] for r in records
            )
    print(json.dumps(result, indent=2))


def cmd_npairs(args):
    records = load(args.path)
    workload = workload_of(records)
    values = [pair_value(r, workload) for r in records]
    s = statistics.stdev(values)
    base_mean = statistics.fmean(r["baseline"]["cycles"] for r in records)
    if workload == "generate":
        q, bound = 0.975, 0.003 * base_mean
    else:
        q, bound = 0.95, math.log(1.003)
    chosen = 60
    for n in range(20, 61, 2):
        if t_quantile(q, n - 1) * s / math.sqrt(n) <= bound:
            chosen = n
            break
    print(json.dumps({"workload": workload, "calibration_pairs": len(values), "sd": s, "baseline_cycles_mean": base_mean, "bound": bound, "n": chosen}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    pairs = sub.add_parser("pairs")
    pairs.add_argument("--workload", choices=("generate", "bench"), required=True)
    pairs.add_argument("--baseline", required=True)
    pairs.add_argument("--candidate", required=True)
    pairs.add_argument("--net", required=True)
    pairs.add_argument("--pairs", type=int, required=True)
    pairs.add_argument("--out", required=True)
    pairs.add_argument("--cwd", required=True)
    pairs.add_argument("--cpu", default="3")
    pairs.add_argument("--depth", type=int)
    pairs.add_argument("--games", type=int)
    pairs.add_argument("--seed-base", type=int)
    pairs.add_argument("--early-stop", action="store_true", help="主の負荷で、8組の時点の打ち切り判定を行う")

    loadonly = sub.add_parser("loadonly")
    loadonly.add_argument("--binary", required=True)
    loadonly.add_argument("--net", required=True)
    loadonly.add_argument("--sha256", required=True)
    loadonly.add_argument("--cwd", required=True)
    loadonly.add_argument("--cpu", default="3")
    loadonly.add_argument("--repeat", type=int, default=3)

    summary = sub.add_parser("summary")
    summary.add_argument("path")

    npairs = sub.add_parser("npairs")
    npairs.add_argument("path")

    args = parser.parse_args()
    if args.command == "pairs":
        if args.workload == "bench" and args.depth is None:
            parser.error("--depth is required for bench")
        if args.workload == "generate" and (args.games is None or args.seed_base is None):
            parser.error("--games and --seed-base are required for generate")
        if args.early_stop and args.workload != "generate":
            parser.error("--early-stop applies only to the generate workload")
        cmd_pairs(args)
    elif args.command == "loadonly":
        cmd_loadonly(args)
    elif args.command == "summary":
        cmd_summary(args)
    else:
        cmd_npairs(args)


if __name__ == "__main__":
    main()
