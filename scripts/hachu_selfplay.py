#!/usr/bin/env python3
"""HaChuのCECP自己対局を回して棋譜NDJSONを生成するスクリプト。

プロトコル層マイルストーン（docs/plans/protocol-layer.md フェーズ5）の
HaChu互換検証で用いる。HaChuプロセスを1個起動し、各手番で`go`を送って
両側を同じプロセスに指させ、各手のCECP文字列と終局の結果行を1局1行の
NDJSONへ記録する。出力は scripts/hachu_replay.py が読み取る。

HaChuを2プロセス立てて`usermove`で手を転送する構成は採らない。HaChuは
`memory`コマンドを受け取るまで置換表を確保せず、`go`を受けた時点で
NULLポインタを参照して異常終了する（hachu.c 435行）。さらに`usermove`の
照合に使うルート合法手リストは、探索が改善手をmoveStackの負の添字へ
前置する実装（hachu.c 782行）のために先頭が0より前へ伸びるのに対し、
ParseMoveは古い先頭位置を使う（move.c 129行）。この食い違いで合法手が
リストから見えなくなり、さらにParseMoveの末尾のループが添字iを0へ
戻すため（move.c 151行）、不一致時にINVALIDではなく先頭の手が返る。
結果として転送した手と別の手が適用され、2プロセス構成は数手で局面が
食い違う。単一プロセスの自己対局はこの経路を通らない。

各手の出力の確定にはCECPの`ping`/`pong`を使う。HaChuは非ポンダー時に
探索中の入力を読まないため、`go`と`ping`をまとめて送っても`pong`は
着手と結果行の出力後に返る。
"""

import argparse
import json
import queue
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

# 局ごとの探索深さ（先手, 後手）。HaChuの局面評価に乱数項はなく
# （hachu.c 688行の加算はコメントアウト済み）、同じ深さ組では同じ
# 棋譜が再現するため、局の多様性は深さ組の変化で作る。
DEPTH_PAIRS = [
    (4, 4),
    (4, 3),
    (3, 4),
    (5, 4),
    (4, 5),
    (3, 3),
    (5, 3),
    (3, 5),
    (5, 5),
    (2, 4),
]

RESULT_RE = re.compile(r"^(1-0|0-1|1/2-1/2)(?:\s*\{(.*)\})?\s*$")


class EngineDied(RuntimeError):
    pass


class EngineTimeout(RuntimeError):
    pass


class Engine:
    """CECPで駆動するHaChuプロセス1個。"""

    def __init__(self, path: str, log):
        self.log = log
        self.proc = subprocess.Popen(
            [path],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
        )
        self.lines: "queue.Queue[str]" = queue.Queue()
        threading.Thread(target=self._read_loop, daemon=True).start()
        self.ping_counter = 0

    def _read_loop(self) -> None:
        for line in self.proc.stdout:
            self.lines.put(line.rstrip("\n"))
        self.lines.put(None)

    def send(self, text: str) -> None:
        if self.log is not None:
            self.log.write(f"< {text}\n")
        if self.proc.poll() is not None:
            raise EngineDied(f"process exited with {self.proc.poll()} before send")
        try:
            self.proc.stdin.write(text + "\n")
            self.proc.stdin.flush()
        except (BrokenPipeError, ValueError) as exc:
            raise EngineDied(str(exc)) from exc

    def read_line(self, timeout: float) -> str:
        try:
            line = self.lines.get(timeout=timeout)
        except queue.Empty:
            raise EngineTimeout(f"no output within {timeout}s")
        if line is None:
            raise EngineDied(f"stdout closed (exit code {self.proc.poll()})")
        if self.log is not None:
            self.log.write(f"> {line}\n")
        return line

    def sync(self, timeout: float) -> list:
        """pingを送り、pongまでに現れた非デバッグ行を返す。"""
        self.ping_counter += 1
        token = self.ping_counter
        self.send(f"ping {token}")
        collected = []
        while True:
            line = self.read_line(timeout)
            if line.startswith("#"):
                continue
            if line == f"pong {token}":
                return collected
            collected.append(line)

    def close(self) -> None:
        try:
            if self.proc.poll() is None:
                self.proc.stdin.write("quit\n")
                self.proc.stdin.flush()
        except Exception:
            pass
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(timeout=5)


def start_engine(path: str, memory_mb: int, time_cs: int, log) -> Engine:
    engine = Engine(path, log)
    engine.send("xboard")
    engine.send("protover 2")
    deadline = time.monotonic() + 10.0
    while True:
        line = engine.read_line(max(0.5, deadline - time.monotonic()))
        if line.startswith("feature") and "done=1" in line:
            break
    # 置換表の確保はmemoryコマンドが唯一の契機であり、省略するとgoで異常終了する。
    engine.send(f"memory {memory_mb}")
    engine.send("new")
    engine.send("variant chu")
    engine.send("easy")
    engine.send("nopost")
    engine.send(f"time {time_cs}")
    engine.sync(10.0)
    return engine


def collect_move(lines: list) -> tuple:
    """pong到達までの行から、着手（連結済み）と結果行を取り出す。"""
    move_parts = []
    move = None
    result = None
    reason = None
    other = []
    for line in lines:
        if line.startswith("move "):
            body = line[5:].strip()
            if body.endswith(","):
                move_parts.append(body)
            else:
                move = "".join(move_parts) + body
                move_parts = []
            continue
        matched = RESULT_RE.match(line)
        if matched:
            # HaChuのPrintResultは引き分け時に1/2-1/2と0-1を続けて出す
            # （hachu.c 1104-1111行のelse欠落）ため、最初の行だけを採る。
            if result is None:
                result = matched.group(1)
                reason = matched.group(2)
            continue
        if line.strip():
            other.append(line)
    return move, result, reason, other


def play_game(
    hachu: str,
    game_index: int,
    white_depth: int,
    black_depth: int,
    memory_mb: int,
    time_cs: int,
    max_plies: int,
    move_timeout: float,
    log,
) -> dict:
    started = time.monotonic()
    record = {
        "game_index": game_index,
        "white_depth": white_depth,
        "black_depth": black_depth,
        "moves": [],
        "result": None,
        "reason": None,
        "termination": "unknown",
        "ok": False,
        "plies": 0,
        "elapsed_sec": 0.0,
        "note": None,
    }
    engine = None
    try:
        engine = start_engine(hachu, memory_mb, time_cs, log)
        while True:
            if len(record["moves"]) >= max_plies:
                record["termination"] = "truncated"
                record["note"] = f"reached max_plies={max_plies}"
                break
            depth = white_depth if len(record["moves"]) % 2 == 0 else black_depth
            engine.send(f"sd {depth}")
            engine.send("go")
            lines = engine.sync(move_timeout)
            move, result, reason, other = collect_move(lines)
            if other:
                record["termination"] = "protocol_error"
                record["note"] = f"unexpected output: {other!r}"
                break
            if move is not None:
                record["moves"].append(move)
            if result is not None:
                record["result"] = result
                record["reason"] = reason
                record["termination"] = "result"
                record["ok"] = True
                break
            if move is None:
                record["termination"] = "no_move"
                record["note"] = "engine produced neither a move nor a result"
                break
    except EngineTimeout as exc:
        record["termination"] = "timeout"
        record["note"] = str(exc)
    except EngineDied as exc:
        record["termination"] = "crash"
        record["note"] = str(exc)
    finally:
        if engine is not None:
            engine.close()
    record["plies"] = len(record["moves"])
    record["elapsed_sec"] = round(time.monotonic() - started, 2)
    return record


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--hachu", required=True, help="HaChu実行ファイルのパス")
    parser.add_argument("--out", required=True, help="出力NDJSONのパス")
    parser.add_argument("--games", type=int, default=2, help="生成する対局数")
    parser.add_argument("--start-index", type=int, default=0, help="最初の局のgame_index")
    parser.add_argument("--memory", type=int, default=64, help="memoryコマンドで渡すMB数")
    parser.add_argument("--time-cs", type=int, default=60000, help="timeコマンドで渡すセンチ秒")
    parser.add_argument("--max-plies", type=int, default=600, help="打ち切り手数")
    parser.add_argument("--move-timeout", type=float, default=180.0, help="1手あたりの待機上限秒")
    parser.add_argument("--log", help="CECP送受信ログの出力先")
    parser.add_argument("--append", action="store_true", help="出力へ追記する")
    parser.add_argument(
        "--depths",
        help="DEPTH_PAIRSの代わりに使う深さ組。'4,3'のように先手,後手で指定する",
    )
    args = parser.parse_args()

    fixed_depths = None
    if args.depths:
        white, black = args.depths.split(",")
        fixed_depths = (int(white), int(black))

    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    log = open(args.log, "a", encoding="utf-8") if args.log else None
    with open(out_path, "a" if args.append else "w", encoding="utf-8") as out:
        for offset in range(args.games):
            index = args.start_index + offset
            white_depth, black_depth = fixed_depths or DEPTH_PAIRS[index % len(DEPTH_PAIRS)]
            record = play_game(
                args.hachu,
                index,
                white_depth,
                black_depth,
                args.memory,
                args.time_cs,
                args.max_plies,
                args.move_timeout,
                log,
            )
            out.write(json.dumps(record, ensure_ascii=False) + "\n")
            out.flush()
            print(
                f"game {index}: depths={white_depth}/{black_depth} "
                f"plies={record['plies']} result={record['result']} "
                f"reason={record['reason']} termination={record['termination']} "
                f"elapsed={record['elapsed_sec']}s",
                file=sys.stderr,
                flush=True,
            )
    if log is not None:
        log.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
