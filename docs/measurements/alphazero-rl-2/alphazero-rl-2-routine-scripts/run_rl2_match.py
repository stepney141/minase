"""2回目の強化学習のSTC、乱択相手、ネット間の対局を起動する。

リポジトリの根で、bashで tools/train/zero/env.sh を読み込んでから使う。
ログは data/matches/<測定名>.log へ追記する。
条件は docs/plans/alphazero-rl-2.md「進捗の測定」に従う。
"""

import argparse
from pathlib import Path
import re
import sys


sys.path.insert(0, str(Path(__file__).resolve().parent /
                       "../../mcts-fixed-opponents/mcts-fixed-opponents-scripts"))
import run_match


def commit_hash(text):
    if re.fullmatch(r"[0-9a-fA-F]{40}", text) is None:
        raise argparse.ArgumentTypeError("コミットには40桁の16進数を指定してください")
    return text


def network_path(text):
    if not text or any(character.isspace() for character in text):
        raise argparse.ArgumentTypeError("ネットのパスは空文字や空白文字を含められません")
    return text


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    for name in ("stc", "random", "versus"):
        command = commands.add_parser(name)
        command.add_argument("--harness", required=True)
        command.add_argument("--name", required=True, type=run_match.measurement_name)
        command.add_argument("--mcts-commit", required=True, type=commit_hash)
        command.add_argument("--net", required=True, type=network_path)
        if name == "versus":
            command.add_argument("--baseline-net", required=True, type=network_path)
        command.add_argument("--seed", required=True, type=run_match.positive_integer)
        command.add_argument("--pairs", required=True, type=run_match.positive_integer)
    return result


def build_argv(args):
    argv = [
        args.harness, "match", "run",
        "--run-dir", f"data/matches/{args.name}", "--seed", str(args.seed),
        "--candidate", f"commit:{args.mcts_commit}",
        "--candidate-args", "--search mcts",
        "--candidate-option", f"NnModel={args.net}",
        "--candidate-option", f"NnBatchSize={run_match.NN_BATCH_SIZE}",
    ]
    if args.command == "stc":
        argv += [
            "--baseline", f"commit:{run_match.ALPHABETA_COMMIT}",
            "--baseline-args", f"--rules {run_match.ALPHABETA_RULES}",
        ]
        limit, concurrency = "time=10000+100", 1
    elif args.command == "random":
        argv += ["--baseline", "random"]
        limit, concurrency = "nodes=1", 2
    elif args.command == "versus":
        argv += [
            "--baseline", f"commit:{args.mcts_commit}",
            "--baseline-args", "--search mcts",
            "--baseline-option", f"NnModel={args.baseline_net}",
            "--baseline-option", f"NnBatchSize={run_match.NN_BATCH_SIZE}",
        ]
        limit, concurrency = f"nodes={run_match.MCTS_NODES}", 2
    else:
        raise ValueError(f"未知のサブコマンド: {args.command}")
    return argv + [
        "--each", limit,
        "--rules", run_match.ALPHABETA_RULES,
        "--max-ply", str(run_match.ALPHABETA_MAX_PLY),
        "--concurrency", str(concurrency), "elo", "--pairs", str(args.pairs),
    ]


def main():
    args = parser().parse_args()
    return run_match.run_logged(build_argv(args), Path(f"data/matches/{args.name}.log"))


if __name__ == "__main__":
    raise SystemExit(main())
