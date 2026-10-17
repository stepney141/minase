#!/usr/bin/env python3
"""無作為に抽出した局面で、候補と現行を同じノード予算で探索し、より深い参照探索と比べる。

設計書 docs/plans/move-ordering.md「候補3」の「実装後、STCの前に、無作為に抽出した局面で候補と現行を
同じノード予算で探索し、根の評価値と最善手をより深い参照探索と比べて、候補の誤差が現行より悪化していない
ことを確かめる」を実行する。参照探索は現行のバイナリに予算の倍率を掛けたノード数で行う。
局面は保存済みの対局記録から、固定シードで対局を選び、各局の10手目から最終手の1手前までの局面を1つ一様に選ぶ。
"""
import argparse
import json
import random
import statistics
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

RULES = "L0,P0,R1,E0"
MATE_CLIP = 30_000


def sample_positions(run_dir, count, seed):
    games = []
    for path in sorted((run_dir / "pairs").glob("*.json")):
        pair = json.loads(path.read_text())
        for index, game in enumerate(pair["games"]):
            moves = list(pair["opening"]["moves"]) + [
                turn["response"]["usi"] for turn in game["turns"] if turn["response"]["kind"] == "move"
            ]
            if len(moves) > 12:
                games.append(dict(source=str(path), pair=pair["pair_number"], game=index, moves=moves))
    rng = random.Random(seed)
    rng.shuffle(games)
    if count > len(games):
        raise ValueError(f"requested {count} positions, only {len(games)} games available")
    positions = []
    for game in games[:count]:
        ply = rng.randint(10, len(game["moves"]) - 1)
        positions.append(dict(source=game["source"], pair=game["pair"], game=game["game"], ply=ply, moves=game["moves"][:ply]))
    return positions


def search(binary, moves, nodes):
    process = subprocess.Popen(
        [str(binary), "--protocol", "usi", "--rules", RULES],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
    )

    def send(line):
        process.stdin.write(line + "\n")
        process.stdin.flush()

    def read():
        line = process.stdout.readline().strip()
        if not line:
            raise RuntimeError(f"engine closed output: {binary}")
        return line

    try:
        send("usi")
        while read() != "usiok":
            pass
        send("setoption name Threads value 1")
        send("isready")
        while read() != "readyok":
            pass
        send("usinewgame")
        send("position startpos" + (" moves " + " ".join(moves) if moves else ""))
        send(f"go nodes {nodes}")
        score = None
        kind = None
        depth = None
        while True:
            line = read()
            tokens = line.split()
            if tokens and tokens[0] == "info" and "score" in tokens:
                i = tokens.index("score")
                kind = tokens[i + 1]
                value = int(tokens[i + 2])
                score = value if kind == "cp" else (MATE_CLIP if value > 0 else -MATE_CLIP)
                if "depth" in tokens:
                    depth = int(tokens[tokens.index("depth") + 1])
            elif tokens and tokens[0] == "bestmove":
                best = tokens[1]
                break
        send("quit")
        process.communicate(timeout=10)
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate()
    if score is None:
        raise RuntimeError("search returned no score")
    return dict(score=score, kind=kind, depth=depth, bestmove=best)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("current", type=Path, help="現行（候補を含まない構成）のバイナリ")
    parser.add_argument("candidate", type=Path, help="候補のバイナリ")
    parser.add_argument("run_dir", type=Path, help="pairs/を含む記録ディレクトリ")
    parser.add_argument("count", type=int)
    parser.add_argument("seed", type=int)
    parser.add_argument("nodes", type=int, help="候補と現行に与えるノード予算")
    parser.add_argument("multiplier", type=int, help="参照探索のノード予算の倍率")
    parser.add_argument("workers", type=int)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    positions = sample_positions(args.run_dir, args.count, args.seed)

    def run(item):
        reference = search(args.current, item["moves"], args.nodes * args.multiplier)
        current = search(args.current, item["moves"], args.nodes)
        candidate = search(args.candidate, item["moves"], args.nodes)
        return {**item, "reference": reference, "current": current, "candidate": candidate}

    with ThreadPoolExecutor(max_workers=args.workers) as executor:
        results = list(executor.map(run, positions))

    scored = [r for r in results if r["reference"]["kind"] == "cp"]
    summary = {}
    for name in ["current", "candidate"]:
        errors = [abs(r[name]["score"] - r["reference"]["score"]) for r in scored if r[name]["kind"] == "cp"]
        agree = sum(r[name]["bestmove"] == r["reference"]["bestmove"] for r in results)
        summary[name] = dict(
            positions=len(results),
            cp_positions=len(errors),
            mean_abs_error=statistics.fmean(errors) if errors else None,
            median_abs_error=statistics.median(errors) if errors else None,
            bestmove_agreement=agree / len(results),
            mean_depth=statistics.fmean(r[name]["depth"] for r in results if r[name]["depth"] is not None),
        )
    both = [r for r in scored if r["current"]["kind"] == "cp" and r["candidate"]["kind"] == "cp"]
    cand_better = sum(abs(r["candidate"]["score"] - r["reference"]["score"]) < abs(r["current"]["score"] - r["reference"]["score"]) for r in both)
    cur_better = sum(abs(r["candidate"]["score"] - r["reference"]["score"]) > abs(r["current"]["score"] - r["reference"]["score"]) for r in both)
    summary["paired"] = dict(compared=len(both), candidate_closer=cand_better, current_closer=cur_better, ties=len(both) - cand_better - cur_better)
    output = dict(current=str(args.current), candidate=str(args.candidate), run_dir=str(args.run_dir), seed=args.seed,
                  nodes=args.nodes, multiplier=args.multiplier, rules=RULES, summary=summary, positions=results)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=1) + "\n")
    print(json.dumps(summary, ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
