#!/usr/bin/env python3
"""Run repeatable nn-eval, USI MCTS search, and zero-selfplay probes against minase."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import time


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Reproduce minase nn-eval, USI MCTS match-play, and zero-selfplay measurements."
    )
    parser.add_argument("--minase-bin", type=Path, required=True)
    parser.add_argument("--zero-net", type=Path, required=True)
    parser.add_argument("--commit40", type=str, required=True)
    parser.add_argument("--ep", choices=("cuda", "tensorrt"), required=True)
    parser.add_argument("--cp-per-logit", type=str, required=True)
    parser.add_argument("--batch-size", type=int, required=True)
    parser.add_argument("--search-nodes", type=int, required=True)
    parser.add_argument("--selfplay-threads", type=int, required=True)
    parser.add_argument("--selfplay-games", type=int, required=True)
    parser.add_argument("--games-per-thread", type=int, required=True)
    parser.add_argument("--simulations", type=int, required=True)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--xdg-cache-home", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    return parser.parse_args()


def usi_session(
    minase_bin: Path, commands: list[str], env: dict[str, str]
) -> list[str]:
    payload = "\n".join(commands) + "\n"
    proc = subprocess.run(
        [str(minase_bin), "--protocol", "usi", "--search", "mcts"],
        input=payload,
        text=True,
        capture_output=True,
        check=True,
        env=env,
    )
    return proc.stdout.splitlines()


def run_nn_eval(
    args: argparse.Namespace, fixtures: Path, env: dict[str, str]
) -> dict[str, object]:
    out_json = args.out_dir / f"nn-eval-{args.ep}-b{args.batch_size}.json"
    cmds = [
        "usi",
        "setoption name RuleSet value lishogi",
        f"setoption name NnModel value {args.zero_net}",
        f"setoption name NnCpPerLogit value {args.cp_per_logit}",
        f"setoption name NnBackend value {args.ep}",
        f"setoption name NnBatchSize value {args.batch_size}",
        "isready",
        f"nn-eval out {out_json}",
        f"nn-eval out {out_json}",
        f"nn-eval out {out_json}",
        "quit",
    ]
    lines = usi_session(args.minase_bin, cmds, env)
    inf_lines = [line for line in lines if "NN inference" in line]
    return {"raw_inference_lines": inf_lines}


def run_usi_midgame_search(
    args: argparse.Namespace, sfens: list[str], env: dict[str, str]
) -> dict[str, object]:
    cmds = [
        "usi",
        "setoption name RuleSet value lishogi",
        f"setoption name NnModel value {args.zero_net}",
        f"setoption name NnCpPerLogit value {args.cp_per_logit}",
        f"setoption name NnBackend value {args.ep}",
        f"setoption name NnBatchSize value {args.batch_size}",
        "isready",
        "usinewgame",
        "position startpos",
        "go nodes 256",
        "isready",
    ]
    for sfen in sfens:
        cmds.extend(["usinewgame", f"position sfen {sfen}", f"go nodes {args.search_nodes}", "isready"])
    cmds.append("quit")
    started = time.perf_counter_ns()
    lines = usi_session(args.minase_bin, cmds, env)
    wall_ns = time.perf_counter_ns() - started
    info_lines = [line for line in lines if line.startswith("info depth ")][-len(sfens) :]
    total_nodes = 0
    total_search_ms = 0
    for line in info_lines:
        toks = line.split()
        total_nodes += int(toks[toks.index("nodes") + 1])
        total_search_ms += int(toks[toks.index("time") + 1])
    return {
        "positions": len(sfens),
        "target_nodes": args.search_nodes,
        "total_nodes": total_nodes,
        "total_search_ms": total_search_ms,
        "wall_seconds": wall_ns / 1e9,
    }


def main() -> None:
    args = parse_args()
    if not args.minase_bin.exists():
        raise FileNotFoundError(f"minase binary not found: {args.minase_bin}")
    if not args.zero_net.exists():
        raise FileNotFoundError(f"zero net not found: {args.zero_net}")
    args.out_dir.mkdir(parents=True, exist_ok=True)
    args.xdg_cache_home.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, XDG_CACHE_HOME=str(args.xdg_cache_home))

    fixtures_path = args.out_dir / "fixtures.jsonl"
    usi_session(
        args.minase_bin,
        ["usi", f"nn-fixture out {fixtures_path}", "quit"],
        env,
    )
    sfens = [
        json.loads(line)["sfen"]
        for line in fixtures_path.read_text(encoding="utf-8").splitlines()
    ][20:28]

    nn_eval_res = run_nn_eval(args, fixtures_path, env)
    search_res = run_usi_midgame_search(args, sfens, env)

    bench_lines = usi_session(
        args.minase_bin,
        ["usi", f"bench nodes {args.search_nodes} batch {args.batch_size}", "quit"],
        env,
    )

    selfplay_dir = args.out_dir / f"selfplay-t{args.selfplay_threads}"
    selfplay_proc = subprocess.run(
        [
            str(args.minase_bin),
            "data",
            "zero-selfplay",
            "--output-dir",
            str(selfplay_dir),
            "--nn-model",
            str(args.zero_net),
            "--nn-backend",
            args.ep,
            "--generation-commit",
            args.commit40,
            "--seed",
            str(args.seed),
            "--start-position",
            "shuffled-setup",
            "--games",
            str(args.selfplay_games),
            "--target-positions",
            "1000000",
            "--chunk-games",
            str(args.selfplay_games),
            "--simulations",
            str(args.simulations),
            "--resign-threshold",
            "0",
            "--threads",
            str(args.selfplay_threads),
            "--devices",
            "1",
            "--games-per-thread",
            str(args.games_per_thread),
        ],
        check=True,
        text=True,
        capture_output=True,
        env=env,
    )

    payload = {
        "ep": args.ep,
        "batch_size": args.batch_size,
        "nn_eval": nn_eval_res,
        "match_play_search": search_res,
        "cpu_bench_tail": bench_lines[-5:],
        "zero_selfplay_stdout": selfplay_proc.stdout,
    }
    (args.out_dir / "probe_run.json").write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
