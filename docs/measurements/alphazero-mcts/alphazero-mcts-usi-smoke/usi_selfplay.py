"""MCTSのminaseをUSIで1プロセス起動し、両側を指させて1局を最後まで進める確認用の駆動部。

終局はエンジン自身の審判層を`state`で照会して判定する。`go`の応答時間と`info`の行を記録する。
"""

import argparse
import json
import subprocess
import sys
import time


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", required=True)
    parser.add_argument("--net", required=True)
    parser.add_argument("--backend", required=True)
    parser.add_argument("--batch", required=True, type=int)
    parser.add_argument("--go", required=True, help="go の引数（例: nodes 32）")
    parser.add_argument("--max-plies", required=True, type=int)
    parser.add_argument("--log", required=True)
    parser.add_argument("--cache-dir")
    parser.add_argument("--draw-value", type=int)
    args = parser.parse_args()

    command = [args.engine, "--protocol", "usi", "--rules", "engine-default",
               "--search", "mcts", "--nn-model", args.net]
    engine = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              text=True, bufsize=1)
    log = open(args.log, "w", encoding="utf-8")

    def send(line: str) -> None:
        log.write(f"> {line}\n")
        engine.stdin.write(line + "\n")
        engine.stdin.flush()

    def read_until(prefix: str) -> list[str]:
        lines = []
        while True:
            line = engine.stdout.readline()
            if not line:
                raise RuntimeError(f"engine exited while waiting for {prefix}; see {args.log}")
            line = line.rstrip("\n")
            log.write(f"< {line}\n")
            lines.append(line)
            if line.startswith(prefix):
                return lines

    send("usi")
    read_until("usiok")
    send(f"setoption name NnBackend value {args.backend}")
    send(f"setoption name NnBatchSize value {args.batch}")
    if args.cache_dir:
        send(f"setoption name NnCacheDir value {args.cache_dir}")
    if args.draw_value is not None:
        send(f"setoption name DrawValueBlack value {args.draw_value}")
        send(f"setoption name DrawValueWhite value {args.draw_value}")
    started = time.monotonic()
    send("isready")
    read_until("readyok")
    ready_seconds = time.monotonic() - started
    send("usinewgame")

    moves: list[str] = []
    think_seconds: list[float] = []
    status = "ongoing"
    infos = []
    while len(moves) < args.max_plies:
        send("position startpos" + (" moves " + " ".join(moves) if moves else ""))
        started = time.monotonic()
        send("go " + args.go)
        lines = read_until("bestmove")
        think_seconds.append(time.monotonic() - started)
        infos.append([line for line in lines if line.startswith("info ")])
        best = lines[-1].split()[1]
        if best == "resign":
            status = "resign"
            break
        moves.append(best)
        send("position startpos moves " + " ".join(moves))
        send("state")
        state = read_until("state ")[-1]
        status = state.split(" status ", 1)[1]
        if status != "ongoing":
            break
    send("quit")
    engine.wait(timeout=60)
    log.close()
    summary = {
        "plies": len(moves),
        "status": status,
        "isready_seconds": round(ready_seconds, 3),
        "think_seconds_max": round(max(think_seconds), 3),
        "think_seconds_mean": round(sum(think_seconds) / len(think_seconds), 4),
        "exit_code": engine.returncode,
        "think_seconds_first_100": round(sum(think_seconds[:100]) / max(1, len(think_seconds[:100])), 4),
        "think_seconds_last_100": round(sum(think_seconds[-100:]) / max(1, len(think_seconds[-100:])), 4),
        "first_info": infos[0],
        "last_info": infos[-1],
    }
    json.dump(summary, sys.stdout, ensure_ascii=False, indent=2)
    print()


if __name__ == "__main__":
    main()
