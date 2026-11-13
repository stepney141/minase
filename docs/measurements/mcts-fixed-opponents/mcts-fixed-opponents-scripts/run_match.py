"""固定した相手との対局を起動し、端末と測定名のログへ出力する。

リポジトリの根で、呼び出し側が推論用の環境を設定してから実行する。
使い方は python3 -I <このファイル> alphabeta --help、hachu --help、
または resume --help で確認できる。各条件の省略は認めない。
ログは data/matches/<測定名>.log へ追記し、各起動の先頭に実行コマンドを記録する。
"""

import argparse
import os
from pathlib import Path
import selectors
import shlex
import subprocess
import sys


# docs/plans/mcts-fixed-opponents.md「HaChuとの対局の条件」に従う。
# αβとの対局にも同じMCTS条件を使う。
MCTS_NODES = 800
NN_BATCH_SIZE = 16
HACHU_HASH_MB = 256
HACHU_RULES = "L1,L3,P0,P5,P6,R2,E1,E2"
HACHU_MAX_PLY = 1900
# docs/plans/mcts-fixed-opponents.md「固定したαβ探索との対局の条件」に従う。
ALPHABETA_COMMIT = "45d5db5498e1739b2307b3719891a1eec24f7fbd"
ALPHABETA_RULES = "lishogi"
ALPHABETA_MAX_PLY = 4096


def positive_integer(text):
    value = int(text)
    if value < 1:
        raise argparse.ArgumentTypeError("1以上の整数を指定してください")
    return value


def measurement_name(text):
    if not text or text in {".", ".."} or Path(text).name != text:
        raise argparse.ArgumentTypeError("測定名には単一のディレクトリ名を指定してください")
    return text


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    commands = result.add_subparsers(dest="command", required=True)
    for name in ("alphabeta", "hachu", "resume"):
        command = commands.add_parser(name)
        command.add_argument("--harness", required=True)
        command.add_argument("--name", required=True, type=measurement_name)
        if name == "resume":
            command.add_argument("--target-pairs", type=positive_integer)
            continue
        command.add_argument("--mcts-commit", required=True)
        command.add_argument("--net", required=True)
        for option in ("seed", "pairs", "concurrency"):
            command.add_argument(f"--{option}", required=True, type=positive_integer)
        if name == "alphabeta":
            command.add_argument("--nodes", required=True, type=int, choices=(250, 500, 1000))
        else:
            command.add_argument("--hachu", required=True)
            command.add_argument("--depth", required=True, type=positive_integer)
    return result


def build_argv(args):
    """設計書の対局条件をハーネスの引数列へ変換する。

    docs/plans/mcts-fixed-opponents.mdの「HaChuとの対局の条件」と
    「固定したαβ探索との対局の条件」に従う。
    """
    run_dir = f"data/matches/{measurement_name(args.name)}"
    argv = [args.harness, "match", "run"]
    if args.command == "resume":
        argv += ["--resume", run_dir]
        if args.target_pairs is not None:
            argv += ["--target-pairs", str(args.target_pairs)]
        return argv
    argv += [
        "--run-dir", run_dir, "--seed", str(args.seed),
        "--candidate", f"commit:{args.mcts_commit}",
        "--candidate-args", "--search mcts",
        "--candidate-option", f"NnModel={args.net}",
        "--candidate-option", f"NnBatchSize={NN_BATCH_SIZE}",
        "--candidate-limit", f"nodes={MCTS_NODES}",
    ]
    if args.command == "alphabeta":
        argv += [
            "--baseline", f"commit:{ALPHABETA_COMMIT}",
            "--baseline-args", f"--rules {ALPHABETA_RULES}",
            "--baseline-limit", f"nodes={args.nodes}",
        ]
        rules, max_ply = ALPHABETA_RULES, ALPHABETA_MAX_PLY
    elif args.command == "hachu":
        if not args.hachu or any(character.isspace() for character in args.hachu):
            raise ValueError("HaChuのパスは空文字や空白文字を含められません")
        if args.depth < 1:
            raise ValueError("HaChuの深さには1以上の整数が必要です")
        argv += [
            "--baseline", f"cecp:{args.hachu}",
            "--baseline-limit", f"depth={args.depth}",
            "--baseline-hash", str(HACHU_HASH_MB),
        ]
        rules, max_ply = HACHU_RULES, HACHU_MAX_PLY
    else:
        raise ValueError(f"未知のサブコマンド: {args.command}")
    return argv + [
        "--rules", rules, "--max-ply", str(max_ply),
        "--concurrency", str(args.concurrency), "elo", "--pairs", str(args.pairs),
    ]


def run_logged(argv, log_path):
    """コマンドを記録してから起動し、両出力を端末とログへ流す。"""
    command = shlex.join(argv)
    print(command, flush=True)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("a+b") as log, selectors.DefaultSelector() as selector:
        if log.seek(0, os.SEEK_END):
            log.seek(-1, os.SEEK_END)
            if log.read(1) != b"\n":
                log.write(b"\n")
        log.write((command + "\n").encode("utf-8"))
        log.flush()
        process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        buffers = {process.stdout: b"", process.stderr: b""}
        selector.register(process.stdout, selectors.EVENT_READ, sys.stdout)
        selector.register(process.stderr, selectors.EVENT_READ, sys.stderr)
        try:
            while selector.get_map():
                for key, _ in selector.select():
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    pending = buffers[key.fileobj] + chunk
                    lines = pending.split(b"\n")
                    buffers[key.fileobj] = lines.pop()
                    complete = [line + b"\n" for line in lines]
                    if not chunk:
                        if buffers[key.fileobj]:
                            complete.append(buffers[key.fileobj])
                        selector.unregister(key.fileobj)
                    for line in complete:
                        key.data.write(line.decode("utf-8"))
                        key.data.flush()
                        log.write(line)
                        log.flush()
            return process.wait()
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            process.stdout.close()
            process.stderr.close()


def main():
    args = parser().parse_args()
    return run_logged(build_argv(args), Path(f"data/matches/{args.name}.log"))


if __name__ == "__main__":
    raise SystemExit(main())
