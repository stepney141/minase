#!/usr/bin/env python3
"""初形からUSI自己対局を行い、先手の指定手数までの時計と探索情報を記録する。

使い方:
    scripts/byoyomi_game_profile.py --engine target/release/minase --rules L0,P0,R1,E0 --hash 256 --time 3000+0 --byoyomi 300 --black-moves 3 --budget quadruple-soft --record /tmp/byoyomi.json
"""

import argparse
import json
import math
import os
import re
import selectors
import subprocess
import time
from contextlib import ExitStack
from pathlib import Path
from typing import Any

import clock_budget_stats as budgets


class EngineError(RuntimeError):
    """ハーネスと同じ異常分類と、診断用の詳細を保持する。"""

    def __init__(self, reason: str, detail: str) -> None:
        super().__init__(detail)
        self.reason = reason


def protocol_int(value: str, low: int, high: int) -> int:
    """USIの整数を、ハーネスが受理する範囲で読む。"""
    if re.fullmatch(r"[+-]?[0-9]+", value) is None:
        raise ValueError(f"不正な整数: {value}")
    parsed = int(value)
    if not low <= parsed <= high:
        raise ValueError(f"範囲外の整数: {value}")
    return parsed


def parse_evaluation(tokens: list[str], side: str) -> dict[str, Any] | None:
    """最後の有効なscore行だけを評価として採用し、不正な評価行は無視する。"""
    if not tokens or tokens[0] != "info" or tokens[1:2] == ["string"] or "score" not in tokens:
        return None
    try:
        index = tokens.index("score")
        kind, value = tokens[index + 1:index + 3]
        if kind == "cp":
            score = {"kind": "cp", "value": protocol_int(value, -(2**31), 2**31 - 1)}
        elif kind == "mate":
            if value in ("+", "-"):
                score = {"kind": "mate_in" if value == "+" else "mated_in", "moves": None}
            else:
                moves = protocol_int(value, -(2**31), 2**31 - 1)
                score = {"kind": "mate_in" if moves >= 0 else "mated_in", "moves": abs(moves)}
        else:
            return None
        depth = (
            protocol_int(tokens[tokens.index("depth") + 1], 0, 2**32 - 1)
            if "depth" in tokens else None
        )
        lower = "lowerbound" in tokens[index + 3:]
        upper = "upperbound" in tokens[index + 3:]
        if lower and upper:
            return None
        return {
            "perspective": side,
            "depth": depth,
            "score": score,
            "bound": "lower" if lower else "upper" if upper else "exact",
        }
    except (ValueError, IndexError):
        return None


def observe_search_data(turn: dict[str, Any], line: str) -> None:
    """評価・停止理由・完了時間を、それぞれ最後の該当行で更新する。"""
    tokens = line.split()
    evaluation = parse_evaluation(tokens, turn["side"])
    if evaluation is not None:
        turn["evaluation"] = evaluation
    if tokens[:3] == ["info", "string", "stop"]:
        if len(tokens) != 4 or tokens[3] not in budgets.STOP_REASONS:
            raise EngineError("crash", f"不正な停止理由: {line}")
        turn["stop_reason"] = tokens[3]
    if tokens[:1] == ["info"] and tokens[1:2] != ["string"] and "time" in tokens:
        try:
            turn["completed_time_ms"] = protocol_int(tokens[tokens.index("time") + 1], 0, 2**64 - 1)
        except (ValueError, IndexError) as error:
            raise EngineError("crash", f"不正な完了時間: {line}") from error


class Engine:
    """期限つきでUSIの行を読み、終了時に回収するエンジンプロセス。"""

    def __init__(self, path: Path, rules: str, timeout: float) -> None:
        self.timeout = timeout
        self.proc = subprocess.Popen(
            [str(path), "--protocol", "usi", "--rules", rules],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            bufsize=0,
        )
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.proc.stdout, selectors.EVENT_READ)
        self.buffer = b""

    def send(self, command: str) -> None:
        """改行で区切ったコマンドを送り、切断は異常として通知する。"""
        assert self.proc.stdin is not None
        try:
            data = memoryview((command + "\n").encode("utf-8"))
            while data:
                written = self.proc.stdin.write(data)
                if not written:
                    raise EngineError("crash", "エンジンの入力パイプが切断された")
                data = data[written:]
        except OSError as error:
            raise EngineError("crash", str(error)) from error

    def receive(self, deadline: float) -> str:
        """絶対期限までに1行受信し、途中までの行でも期限を延長しない。"""
        assert self.proc.stdout is not None
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise EngineError("timeout", "USI応答が期限を超えた")
            if b"\n" in self.buffer:
                raw, _, self.buffer = self.buffer.partition(b"\n")
                try:
                    line = raw.decode("utf-8").strip()
                except UnicodeDecodeError as error:
                    raise EngineError("crash", "USI応答がUTF-8ではない") from error
                if line.startswith("info string error:"):
                    raise EngineError("crash", line)
                return line
            if not self.selector.select(remaining):
                raise EngineError("timeout", "USI応答が期限を超えた")
            chunk = os.read(self.proc.stdout.fileno(), 65536)
            if not chunk:
                raise EngineError("crash", "bestmoveまたは初期化応答の前に出力が閉じられた")
            self.buffer += chunk

    def expect(self, response: str) -> None:
        """初期化時の所定の応答を、1つの期限内に受信する。"""
        deadline = time.monotonic() + self.timeout
        while self.receive(deadline) != response:
            pass

    def initialize(self, rules: str, hash_mb: int) -> None:
        """USIの握手、規則と置換表の設定、新規対局の開始を行う。"""
        self.send("usi")
        self.expect("usiok")
        self.send(f"setoption name RuleSet value {rules}")
        self.send(f"setoption name USI_Hash value {hash_mb}")
        self.send("isready")
        self.expect("readyok")
        self.send("usinewgame")

    def think(self, moves: list[str], side: str, go: str) -> tuple[dict[str, Any], str | None]:
        """go送信直前からbestmove受信までを測り、TurnRecord形式で返す。"""
        self.send("position startpos" + (" moves " + " ".join(moves) if moves else ""))
        turn: dict[str, Any] = {
            "side": side,
            "evaluation": None,
            "stop_reason": None,
            "completed_time_ms": None,
        }
        deadline = time.monotonic() + self.timeout
        start = time.perf_counter_ns()
        try:
            self.send(go)
            while True:
                line = self.receive(deadline)
                received = time.perf_counter_ns()
                tokens = line.split()
                if tokens[:1] == ["bestmove"]:
                    if len(tokens) < 2:
                        raise EngineError("crash", "bestmoveに着手がない")
                    turn["think_time_ns"] = received - start
                    turn["response"] = (
                        {"kind": "resigned"} if tokens[1] == "resign"
                        else {"kind": "move", "usi": tokens[1]}
                    )
                    return turn, None
                observe_search_data(turn, line)
        except (EngineError, OSError) as error:
            turn["think_time_ns"] = time.perf_counter_ns() - start
            reason = error.reason if isinstance(error, EngineError) else "crash"
            turn["response"] = {"kind": "failure", "reason": reason}
            return turn, str(error)

    def close(self) -> None:
        """quitを送り、応答しないプロセスも終了させて回収する。"""
        try:
            self.send("quit")
        except EngineError:
            pass
        try:
            self.proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        finally:
            self.selector.close()
            if self.proc.stdin is not None:
                self.proc.stdin.close()
            if self.proc.stdout is not None:
                self.proc.stdout.close()


def play(args: argparse.Namespace) -> dict[str, Any]:
    """別プロセスの両側を交互に指させ、途中終了も含めた記録を返す。"""
    base, increment = args.time
    initial_clock = {"base_ms": base, "increment_ms": increment, "byoyomi_ms": args.byoyomi}
    record: dict[str, Any] = {
        "initial_clock": initial_clock,
        "ply_origin": 0,
        "engine": {"path": str(args.engine), "rules": args.rules, "hash_mb": args.hash},
        "budget": args.budget,
        "black_moves_target": args.black_moves,
        "response_timeout_secs": args.response_timeout,
        "moves": [],
        "turns": [],
        "forfeits": {side: 0 for side in budgets.COLORS},
        "black_moves": 0,
        "completed": False,
        "termination": None,
    }
    clocks = {side: base * budgets.NS_PER_MS for side in budgets.COLORS}
    with ExitStack() as stack:
        engines = {}
        try:
            for side in budgets.COLORS:
                engine = Engine(args.engine, args.rules, args.response_timeout)
                stack.callback(engine.close)
                engines[side] = engine
                engine.initialize(args.rules, args.hash)
            while record["black_moves"] < args.black_moves:
                side = budgets.COLORS[len(record["moves"]) % 2]
                go = (
                    f"go btime {clocks['black'] // budgets.NS_PER_MS} "
                    f"wtime {clocks['white'] // budgets.NS_PER_MS} "
                    f"binc {increment} winc {increment} byoyomi {args.byoyomi}"
                )
                turn, detail = engines[side].think(record["moves"], side, go)
                record["turns"].append(turn)
                clocks[side], forfeit = budgets.advance_clock(
                    clocks[side], turn["think_time_ns"],
                    increment * budgets.NS_PER_MS, args.byoyomi * budgets.NS_PER_MS,
                )
                record["forfeits"][side] += int(forfeit)
                response = turn["response"]
                if response["kind"] != "move":
                    record["termination"] = {
                        "side": side,
                        "reason": "bestmove resign" if response["kind"] == "resigned" else response["reason"],
                        "detail": detail,
                    }
                    break
                record["moves"].append(response["usi"])
                if side == "black":
                    record["black_moves"] += 1
            else:
                record["completed"] = True
        except (EngineError, OSError) as error:
            record["termination"] = {
                "reason": error.reason if isinstance(error, EngineError) else "crash",
                "detail": str(error),
            }
    record["end_message"] = f"先手{record['black_moves']}手で終了"
    return record


def nonnegative(value: str) -> int:
    """非負の整数をコマンドラインから読む。"""
    try:
        return protocol_int(value, 0, 2**64 - 1)
    except ValueError as error:
        raise argparse.ArgumentTypeError(str(error)) from error


def positive(value: str) -> int:
    """正の整数をコマンドラインから読む。"""
    result = nonnegative(value)
    if result == 0:
        raise argparse.ArgumentTypeError("正の整数を指定すること")
    return result


def time_control(value: str) -> tuple[int, int]:
    """base_ms+increment_ms形式の時間制御を読む。"""
    if re.fullmatch(r"[0-9]+\+[0-9]+", value) is None:
        raise argparse.ArgumentTypeError("--timeはbase_ms+increment_ms形式で指定すること")
    base, increment = value.split("+")
    return nonnegative(base), nonnegative(increment)


def response_timeout(value: str) -> float:
    """応答期限には正の有限秒数だけを受理する。"""
    result = float(value)
    if not math.isfinite(result) or result <= 0:
        raise argparse.ArgumentTypeError("応答期限は正の有限秒数で指定すること")
    return result


def main() -> int:
    """先手を主集計として表示し、要求された記録と集計をJSONへ保存する。"""
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--rules", required=True)
    parser.add_argument("--hash", type=positive, required=True, help="各プロセスの置換表サイズ（MB）")
    parser.add_argument("--time", type=time_control, required=True)
    parser.add_argument("--byoyomi", type=nonnegative, required=True, help="秒読み（ms）")
    parser.add_argument("--black-moves", type=positive, default=40)
    parser.add_argument("--budget", choices=budgets.FORMULAS, required=True)
    parser.add_argument("--response-timeout", type=response_timeout, default=120)
    parser.add_argument("--record", type=Path)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    args.engine = args.engine.resolve()
    if not args.rules.strip() or "\n" in args.rules or "\r" in args.rules:
        parser.error("--rulesに空でない1行の規則を指定すること")
    if args.record is not None and args.json is not None and args.record.resolve() == args.json.resolve():
        parser.error("--recordと--jsonには別の保存先を指定すること")

    record = play(args)
    if args.record is not None:
        args.record.write_text(json.dumps(record, ensure_ascii=False, indent=2, allow_nan=False) + "\n")
    records = budgets.replay_game(
        record["turns"], {side: record["initial_clock"] for side in budgets.COLORS},
        record["ply_origin"], args.budget, game_id="startpos",
    )
    first_ns = record["turns"][0]["think_time_ns"] if record["turns"] else None
    summary = {
        "budget": args.budget,
        "completed": record["completed"],
        "black_moves": record["black_moves"],
        "termination": record["termination"],
        "first_move_think_time_ns": first_ns,
        "first_move_think_time_ms": first_ns / budgets.NS_PER_MS if first_ns is not None else None,
        "black": budgets.summarize([r for r in records if r["side"] == "black"]),
        "both": budgets.summarize(records),
    }
    print(record["end_message"])
    if record["termination"] is not None:
        print(json.dumps(record["termination"], ensure_ascii=False))
    first_text = f"{first_ns / budgets.NS_PER_MS:.6f}" if first_ns is not None else "欠測"
    print(f"初手（ply 0）の実測思考時間: {first_text} ms")
    print("\n先手側の集計（主）")
    print(budgets.format_summary(summary["black"]))
    print("\n両側の集計")
    print(budgets.format_summary(summary["both"]))
    if args.json is not None:
        args.json.write_text(json.dumps(summary, ensure_ascii=False, indent=2, allow_nan=False) + "\n")
    return 0 if record["completed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
