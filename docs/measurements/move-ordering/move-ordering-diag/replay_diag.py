#!/usr/bin/env python3
"""保存対局を固定ノードで再生し、対局内累計の診断値（手の順序付けの診断、項目6と7）と各手の結果を保存する。

診断ビルド（--features search-stats）は bestmove の直前に `info string diag scope=game ...` を出す。
--baseline に計測なしのビルドを渡すと、同じ対局を再生して bestmove とノード数の一致を確かめる。
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import random
import subprocess

KEYS = ("searches "
        "ch_segments ch_segments_changed ch_cut_count ch_cut_forward ch_cut_backward ch_cut_forward_sum ch_cut_backward_sum ch_refs ch_nonzero "
        "cmh_nodes cmh_nodes_with_record cmh_refs cmh_nonzero cmh_order_changed "
        "cont_nodes cont_nodes_with_record cont_refs cont_nonzero cont_order_changed").split()


def positive(text):
    value = int(text)
    if value <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return value


def sample_games(directories, count, seed):
    games = []
    for directory in directories:
        paths = sorted((directory / "pairs").glob("*.json"))
        if not paths:
            raise ValueError(f"no pair records in {directory}")
        for path in paths:
            pair = json.loads(path.read_text())
            for index, game in enumerate(pair["games"]):
                moves = list(pair["opening"]["moves"]) + [
                    turn["response"]["usi"] for turn in game["turns"]
                    if turn["response"]["kind"] == "move"
                ]
                if not moves:
                    raise ValueError(f"empty game: {path}, {index}")
                games.append(dict(source=str(path), pair=pair["pair_number"], game=index, moves=moves))
    if count > len(games):
        raise ValueError(f"requested {count} games, only {len(games)} available")
    random.Random(seed).shuffle(games)
    return games[:count]


def replay(binary, cwd, item, nodes, diagnostic):
    env = os.environ.copy()
    process = subprocess.Popen(
        [str(binary), "--protocol", "usi", "--rules", "L0,P0,R1,E0"],
        cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        text=True, bufsize=1,
    )

    def send(line):
        process.stdin.write(line + "\n")
        process.stdin.flush()

    def read():
        line = process.stdout.readline().strip()
        if not line:
            raise RuntimeError(f"engine closed output: {binary}")
        if line.startswith("info string error"):
            raise RuntimeError(line)
        return line

    def handshake(command, response):
        send(command)
        while read() != response:
            pass

    results = []
    try:
        handshake("usi", "usiok")
        send("setoption name Threads value 1")
        handshake("isready", "readyok")
        send("usinewgame")
        for ply in range(len(item["moves"])):
            prefix = item["moves"][:ply]
            send("position startpos" + (" moves " + " ".join(prefix) if prefix else ""))
            send(f"go nodes {nodes}")
            info = None
            counts = None
            previous = None
            while True:
                line = read()
                if line.startswith("info string diag "):
                    fields = dict(token.split("=", 1) for token in line.split()[3:])
                    if fields.pop("scope") != "game" or set(fields) != set(KEYS):
                        raise ValueError(f"unexpected diagnostic schema: {line}")
                    counts = {key: int(value) for key, value in fields.items()}
                elif line.startswith("info ") and " nodes " in line:
                    info = line
                elif line.startswith("bestmove "):
                    if info is None:
                        raise ValueError(f"no final search info at ply {ply}")
                    if diagnostic and (counts is None or not previous.startswith("info string diag ")):
                        raise ValueError(f"missing diagnostic immediately before bestmove at ply {ply}")
                    if not diagnostic and counts is not None:
                        raise ValueError("diagnostic output while disabled")
                    fields = info.split()
                    row = dict(ply=ply, bestmove=line, nodes=int(fields[fields.index("nodes") + 1]), info=info)
                    if diagnostic:
                        row["diag"] = counts
                    results.append(row)
                    break
                previous = line
        send("quit")
        process.communicate(timeout=10)
        if process.returncode:
            raise RuntimeError(f"engine exit status {process.returncode}")
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate()
        process.stdin.close()
        process.stdout.close()
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("run_dir", type=Path, help="pairs/を含む記録ディレクトリ")
    parser.add_argument("games", type=positive)
    parser.add_argument("seed", type=int)
    parser.add_argument("nodes", type=positive)
    parser.add_argument("workers", type=positive)
    parser.add_argument("output", type=Path)
    parser.add_argument("--also-run-dir", type=Path, action="append", default=[], help="追加の記録ディレクトリ")
    parser.add_argument("--cwd", type=Path, default=Path.cwd(), help="エンジンの実行ディレクトリ")
    parser.add_argument("--baseline", type=Path, help="同じ対局を計測なしで再生して比較するバイナリ")
    args = parser.parse_args()
    args.binary = args.binary.resolve(strict=True)
    directories = [path.resolve(strict=True) for path in [args.run_dir, *args.also_run_dir]]
    games = sample_games(directories, args.games, args.seed)
    baseline = args.baseline.resolve(strict=True) if args.baseline else None

    def run(item):
        rows = replay(args.binary, args.cwd, item, args.nodes, True)
        result = {**item, "plies": len(rows), "turns": rows, "totals": rows[-1]["diag"]}
        if baseline:
            reference = replay(baseline, args.cwd, item, args.nodes, False)
            mismatches = [a["ply"] for a, b in zip(rows, reference, strict=True)
                          if (a["bestmove"], a["nodes"]) != (b["bestmove"], b["nodes"])]
            result["baseline_turns"] = reference
            result["mismatches"] = mismatches
        return result

    with ThreadPoolExecutor(max_workers=args.workers) as executor:
        results = list(executor.map(run, games))
    totals = {key: sum(result["totals"][key] for result in results) for key in KEYS}
    rates = {}
    for name, numerator, denominator in [
        ("ch_order", "ch_segments_changed", "ch_segments"),
        ("ch_nonzero", "ch_nonzero", "ch_refs"),
        ("ch_cut_forward", "ch_cut_forward", "ch_cut_count"),
        ("ch_cut_backward", "ch_cut_backward", "ch_cut_count"),
        ("cmh_record", "cmh_nodes_with_record", "cmh_nodes"),
        ("cmh_nonzero", "cmh_nonzero", "cmh_refs"),
        ("cmh_order", "cmh_order_changed", "cmh_nodes"),
        ("cmh_order_given_record", "cmh_order_changed", "cmh_nodes_with_record"),
        ("cont_record", "cont_nodes_with_record", "cont_nodes"),
        ("cont_nonzero", "cont_nonzero", "cont_refs"),
        ("cont_order", "cont_order_changed", "cont_nodes"),
        ("cont_order_given_record", "cont_order_changed", "cont_nodes_with_record"),
    ]:
        rates[name] = totals[numerator] / totals[denominator] if totals[denominator] else None
    output = dict(binary=str(args.binary), baseline=str(baseline) if baseline else None,
                  run_dirs=list(map(str, directories)), cwd=str(args.cwd.resolve()),
                  seed=args.seed, nodes_per_move=args.nodes, workers=args.workers,
                  rules="L0,P0,R1,E0", threads=1, usi_hash="default", scope="game",
                  totals=totals, rates=rates, games=results)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(dict(games=len(results), plies=sum(r["plies"] for r in results),
                         totals=totals, rates=rates), ensure_ascii=False))
    if baseline:
        mismatches = sum(len(result["mismatches"]) for result in results)
        print(f"baseline mismatches: {mismatches}")
        if mismatches:
            raise SystemExit(1)


if __name__ == "__main__":
    main()
