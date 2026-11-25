#!/usr/bin/env python3
"""置換表の寄与の調査（2026年11月25日）で使った計測の道具。

diagnostic.patch を当ててビルドしたminaseを起動し、次の4つを行う。

- bench: 固定15局面を固定深さで探索し、置換表の使い方の方式ごとのノード数を集める。
- replay: 保存済みの対局の着手列に沿って、置換表を保持したまま各手を探索する。
- nps: 固定15局面をUSIの `go depth` で探索し、置換表の容量ごとのNPSを測る。
- show: bench と replay が保存したJSONから、照会と保存の内訳を表示する。

置換表の使い方の方式は環境変数 MINASE_TT_MODE、照会と保存の分類は MINASE_TT_DIAG=1 で選ぶ。
分類を有効にしたminaseは、置換表を破棄するときに標準エラーへ `ttdiag {JSON}` を1行出す。
その配列の添字は次のとおりである。

- probes[s]: 照会の回数。s=0 は通常探索と根、s=1 は静止探索。
- hits[s*8 + o*2 + v]: 照合に成功した回数。o は当たった記録の由来で、0 が同じ反復の別経路、
  1 が同じ反復の同じ経路、2 が同じ探索の前の反復、3 が過去の探索。v=1 は探索値つきの記録、
  v=0 は静的評価だけの記録。
- hit_moves[s]: 記録手を得た回数。
- cutoffs[s*4 + o]: 置換表の値で打ち切った回数。
- depth_ok[o]: 通常探索で、探索値つきの記録の深さが要求深さ以上だった回数。
- stores[k*7 + r]: 保存の試行の回数。k=0 は残り深さ1以上、1 は静止探索の深さ0、
  2 は静的評価だけの記録。r=0 は空のスロットへの書き込み、1 は同じ局面への上書き、
  2 は同じ局面への上書きの拒否、3 は過去の世代の深い記録の追い出し、
  4 は過去の世代の浅い記録の追い出し、5 は同じ世代の浅い記録の追い出し、
  6 は別の局面があるスロットへの書き込みの拒否。
- occ_deep、occ_shallow、occ_eval_only: 破棄の時点で表に残っていた記録の数。
"""
import argparse
import glob
import json
import math
import os
import random
import re
import statistics
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor

ORIGINS = [
    "同じ反復の別経路",
    "同じ反復の同じ経路",
    "同じ探索の前の反復",
    "過去の探索",
]
STORE_KINDS = ["残り深さ1以上", "静止探索", "静的評価だけ"]
STORE_OUTCOMES = [
    "空",
    "同じ局面へ上書き",
    "同じ局面で拒否",
    "過去の世代の深い記録を追い出し",
    "過去の世代の浅い記録を追い出し",
    "同じ世代の浅い記録を追い出し",
    "別の局面で拒否",
]


def engine_env(mode, diag, fresh):
    env = dict(os.environ)
    env["MINASE_TT_MODE"] = mode
    if diag:
        env["MINASE_TT_DIAG"] = "1"
    if fresh:
        env["MINASE_TT_FRESH"] = "1"
    return env


def parse_diag(stderr):
    return [json.loads(line[7:]) for line in stderr.splitlines() if line.startswith("ttdiag ")]


def bench(binary, net, depth, mode, diag):
    """`bench depth N` を実行する。benchは規則セットJCSAと既定の256 MBの表を使う。"""
    commands = [
        "usi",
        "setoption name RuleSet value JCSA",
        f"setoption name EvalFile value {net}",
        "isready",
        f"bench depth {depth}",
        "quit",
    ]
    started = time.time()
    done = subprocess.run(
        [binary, "--protocol", "usi"],
        input="\n".join(commands) + "\n",
        capture_output=True,
        text=True,
        env=engine_env(mode, diag, False),
        check=True,
    )
    positions = {}
    pattern = r"position=(\S+) depth=\d+ nodes=(\d+) best=(\S+) score=(\S+) elapsed=([\d.]+)s"
    for match in re.finditer(pattern, done.stdout):
        positions[match.group(1)] = {
            "nodes": int(match.group(2)),
            "best": match.group(3),
            "score": match.group(4),
            "elapsed": float(match.group(5)),
        }
    if len(positions) != 15:
        raise RuntimeError(f"bench returned {len(positions)} positions")
    return {
        "depth": depth,
        "mode": mode,
        "pos": positions,
        "diag": parse_diag(done.stderr),
        "wall": time.time() - started,
    }


class Engine:
    """USIで対話するminaseのプロセス。"""

    def __init__(self, binary, net, rules, hash_mb, env):
        self.proc = subprocess.Popen(
            [binary, "--protocol", "usi"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
            env=env,
        )
        self.send("usi")
        self.read_until(lambda line: line.strip() == "usiok")
        self.send(f"setoption name RuleSet value {rules}")
        self.send(f"setoption name EvalFile value {net}")
        self.send(f"setoption name USI_Hash value {hash_mb}")
        self.send("isready")
        self.read_until(lambda line: line.strip() == "readyok")

    def send(self, command):
        self.proc.stdin.write(command + "\n")
        self.proc.stdin.flush()

    def read_until(self, accept):
        while True:
            line = self.proc.stdout.readline()
            if not line or line.startswith("info string error"):
                self.proc.kill()
                raise RuntimeError(f"engine failed: {line!r}")
            if accept(line):
                return line

    def go(self, command):
        """探索を1回行い、最後の `info` 行の深さとノード数、および最善手を返す。"""
        last = {"depth": 0, "nodes": 0}

        def accept(line):
            tokens = line.split()
            if tokens[:2] == ["info", "depth"] and "nodes" in tokens:
                last["depth"] = int(tokens[2])
                last["nodes"] = int(tokens[tokens.index("nodes") + 1])
            return line.startswith("bestmove")

        self.send(command)
        best = self.read_until(accept).split()[1]
        return last["depth"], last["nodes"], best

    def quit(self):
        self.send("quit")
        stderr = self.proc.stderr.read()
        self.proc.wait(timeout=60)
        return stderr


def load_games(run_dir, count, seed):
    """`minase match run` の実行ディレクトリから、開始手順を含む着手列を無作為に選ぶ。"""
    games = []
    for path in sorted(glob.glob(f"{run_dir}/pairs/*.json")):
        with open(path) as file:
            pair = json.load(file)
        for game in pair["games"]:
            moves = [
                turn["response"]["usi"]
                for turn in game["turns"]
                if turn["response"]["kind"] == "move"
            ]
            games.append(list(pair["opening"]["moves"]) + moves)
    random.Random(seed).shuffle(games)
    return games[:count]


def replay(binary, net, rules, moves, go, mode, hash_mb, diag, fresh):
    """1局の全手を、1つのプロセスの置換表を保持したまま順に探索する。"""
    engine = Engine(binary, net, rules, hash_mb, engine_env(mode, diag, fresh))
    engine.send("usinewgame")
    nodes = 0
    depths = []
    bests = []
    for ply in range(len(moves)):
        prefix = " moves " + " ".join(moves[:ply]) if ply else ""
        engine.send("position startpos" + prefix)
        depth, count, best = engine.go(go)
        nodes += count
        depths.append(depth)
        bests.append(best)
    diags = parse_diag(engine.quit())
    final = max(diags, key=lambda item: sum(item["probes"])) if diags else None
    return {"nodes": nodes, "depths": depths, "bests": bests, "diag": final}


def bench_sfens(repository):
    with open(f"{repository}/crates/minase/src/diagnostics/bench.rs") as file:
        sfens = re.findall(r'sfen: "([^"]+)"', file.read())
    if len(sfens) != 15:
        raise RuntimeError(f"found {len(sfens)} bench positions")
    return sfens


def nps(binary, net, sfens, depth, hash_mb):
    """固定15局面を、局面ごとに `usinewgame` で表を消してから探索し、総ノード数とNPSを返す。"""
    engine = Engine(binary, net, "JCSA", hash_mb, engine_env("full", False, False))
    total_nodes = 0
    total_time = 0.0
    for sfen in sfens:
        engine.send("usinewgame")
        engine.send(f"position sfen {sfen} - 1")
        started = time.perf_counter()
        _, nodes, _ = engine.go(f"go depth {depth}")
        total_time += time.perf_counter() - started
        total_nodes += nodes
    engine.quit()
    return total_nodes, total_nodes / total_time


def merge(diags):
    """複数の局の計数を足し合わせる。表に残った記録の数は局の平均にする。"""
    merged = dict(diags[0])
    for key, value in merged.items():
        if isinstance(value, list):
            merged[key] = [sum(diag[key][index] for diag in diags) for index in range(len(value))]
        elif key.startswith("occ_"):
            merged[key] = sum(diag[key] for diag in diags) / len(diags)
    return merged


def show_diag(diag, label):
    print(f"== {label}: エントリ数 {diag['entries']:,}")
    for site, name in ((0, "通常探索"), (1, "静止探索")):
        probes = diag["probes"][site]
        if not probes:
            continue
        hits = diag["hits"][site * 8 : site * 8 + 8]
        cutoffs = diag["cutoffs"][site * 4 : site * 4 + 4]
        valued = sum(hits[1::2])
        print(
            f" {name}: 照会 {probes:,} 照合成功 {sum(hits) / probes:.1%}"
            f"（探索値つき {valued / probes:.1%}、静的評価だけ {(sum(hits) - valued) / probes:.1%}）"
            f" 記録手あり {diag['hit_moves'][site] / probes:.1%} 打ち切り {sum(cutoffs) / probes:.1%}"
        )
        for origin in range(4):
            line = (
                f"    {ORIGINS[origin]}: 探索値つき {hits[origin * 2 + 1] / probes:.2%}"
                f" 静的評価だけ {hits[origin * 2] / probes:.2%} 打ち切り {cutoffs[origin] / probes:.2%}"
            )
            if site == 0:
                line += f" 深さが足りた {diag['depth_ok'][origin] / probes:.2%}"
            print(line)
    for kind in range(3):
        stores = diag["stores"][kind * 7 : kind * 7 + 7]
        total = sum(stores)
        if total:
            shares = "、".join(
                f"{STORE_OUTCOMES[index]} {stores[index] / total:.1%}"
                for index in range(7)
                if stores[index]
            )
            print(f" 保存（{STORE_KINDS[kind]}）{total:,}: {shares}")
    occupied = diag["occ_deep"] + diag["occ_shallow"] + diag["occ_eval_only"]
    print(f" 終了時の使用率 {occupied / diag['entries']:.2%}")


def show(path):
    with open(path) as file:
        data = json.load(file)
    for key, value in data.items():
        if isinstance(value, list):
            diags = [game["diag"] for game in value if game.get("diag")]
            if diags:
                depths = [depth for game in value for depth in game["depths"]]
                print(f"{key}: {len(depths)}手、深さの平均 {statistics.mean(depths):.2f}")
                show_diag(merge(diags), key)
        elif value.get("diag"):
            show_diag(max(value["diag"], key=lambda item: item["generation"]), key)


def summarize_bench(results):
    for depth in sorted({result["depth"] for result in results}):
        by_mode = {result["mode"]: result["pos"] for result in results if result["depth"] == depth}
        base = by_mode["full"]
        print(f"深さ {depth}: full の総ノード数 {sum(item['nodes'] for item in base.values()):,}")
        for mode, positions in by_mode.items():
            ratios = [positions[name]["nodes"] / base[name]["nodes"] for name in base]
            total = sum(item["nodes"] for item in positions.values()) / sum(
                item["nodes"] for item in base.values()
            )
            mean = math.exp(sum(map(math.log, ratios)) / len(ratios))
            print(f"  {mode}: 総ノード数の比 {total:.2f}、局面ごとの比の幾何平均 {mean:.2f}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)

    sub = commands.add_parser("bench")
    sub.add_argument("--bin", required=True)
    sub.add_argument("--net", required=True)
    sub.add_argument("--depth", type=int, nargs="+", required=True)
    sub.add_argument("--modes", nargs="+", required=True)
    sub.add_argument("--diag", action="store_true")
    sub.add_argument("--workers", type=int, required=True)
    sub.add_argument("--out", required=True)

    sub = commands.add_parser("replay")
    sub.add_argument("--bin", required=True)
    sub.add_argument("--net", required=True)
    sub.add_argument("--rules", required=True)
    sub.add_argument("--run-dir", required=True)
    sub.add_argument("--games", type=int, required=True)
    sub.add_argument("--seed", type=int, required=True)
    sub.add_argument("--go", required=True)
    sub.add_argument("--mode", required=True)
    sub.add_argument("--hash", type=int, required=True)
    sub.add_argument("--diag", action="store_true")
    sub.add_argument("--fresh", action="store_true")
    sub.add_argument("--workers", type=int, required=True)
    sub.add_argument("--out", required=True)

    sub = commands.add_parser("nps")
    sub.add_argument("--bin", required=True)
    sub.add_argument("--net", required=True)
    sub.add_argument("--repository", required=True)
    sub.add_argument("--depth", type=int, required=True)
    sub.add_argument("--hash", type=int, nargs="+", required=True)
    sub.add_argument("--rounds", type=int, required=True)
    sub.add_argument("--out", required=True)

    sub = commands.add_parser("show")
    sub.add_argument("path")

    args = parser.parse_args()
    if args.command == "bench":
        jobs = [(depth, mode) for depth in args.depth for mode in args.modes]
        with ThreadPoolExecutor(max_workers=args.workers) as pool:
            results = list(
                pool.map(lambda job: bench(args.bin, args.net, job[0], job[1], args.diag), jobs)
            )
        with open(args.out, "w") as file:
            json.dump({f"{result['depth']}:{result['mode']}": result for result in results}, file)
        if "full" in args.modes:
            summarize_bench(results)
    elif args.command == "replay":
        games = load_games(args.run_dir, args.games, args.seed)
        with ThreadPoolExecutor(max_workers=args.workers) as pool:
            results = list(
                pool.map(
                    lambda moves: replay(
                        args.bin,
                        args.net,
                        args.rules,
                        moves,
                        args.go,
                        args.mode,
                        args.hash,
                        args.diag,
                        args.fresh,
                    ),
                    games,
                )
            )
        with open(args.out, "w") as file:
            json.dump({args.mode: results}, file)
        depths = [depth for result in results for depth in result["depths"]]
        print(
            f"{len(results)}局 {len(depths)}手 総ノード数 {sum(result['nodes'] for result in results):,}"
            f" 深さの平均 {statistics.mean(depths):.2f}"
        )
    elif args.command == "nps":
        sfens = bench_sfens(args.repository)
        samples = {size: [] for size in args.hash}
        for _ in range(args.rounds):
            for size in args.hash:
                samples[size].append(nps(args.bin, args.net, sfens, args.depth, size))
        with open(args.out, "w") as file:
            json.dump({str(size): values for size, values in samples.items()}, file)
        for size, values in samples.items():
            speeds = [speed for _, speed in values]
            print(f"{size} MB: ノード数 {values[0][0]:,} NPSの中央値 {statistics.median(speeds):,.0f}")
    else:
        show(args.path)


if __name__ == "__main__":
    sys.exit(main())
