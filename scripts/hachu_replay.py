#!/usr/bin/env python3
"""HaChu自己対局の棋譜をminaseのCECPへ再生して照合するスクリプト。

scripts/hachu_selfplay.py が生成したNDJSONを読み、各局の指し手列を
`minase --protocol cecp`へ`usermove`として順に送り、合法性の判定と終局
裁定がHaChuと一致するかを記録する。プロトコル層マイルストーン
（docs/plans/protocol-layer.md フェーズ5）のHaChu互換規則セット検証で
用いる。

1手ごとの応答の確定にはCECPの`ping`/`pong`を使う。minaseは合法手に無応答
であるため、`pong`までに現れた行だけがその手への応答である。
"""

import argparse
import json
import queue
import re
import subprocess
import sys
import threading
from pathlib import Path

RESULT_RE = re.compile(r"^(1-0|0-1|1/2-1/2)(?:\s*\{(.*)\})?\s*$")

DEFAULT_RULE_SETS = ["R2,E1", "L1,R2,E1"]


class Minase:
    """CECPで駆動するminaseプロセス1個。"""

    def __init__(self, binary: str, rules: str, log):
        self.log = log
        self.rules = rules
        self.proc = subprocess.Popen(
            [binary, "--protocol", "cecp", "--rules", rules],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
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
        self.proc.stdin.write(text + "\n")
        self.proc.stdin.flush()

    def sync(self, timeout: float = 30.0) -> list:
        """pingを送り、pongまでに現れた行を返す。"""
        self.ping_counter += 1
        token = self.ping_counter
        self.send(f"ping {token}")
        collected = []
        while True:
            try:
                line = self.lines.get(timeout=timeout)
            except queue.Empty:
                raise RuntimeError(f"minase({self.rules}): no output within {timeout}s")
            if line is None:
                stderr = self.proc.stderr.read()
                raise RuntimeError(
                    f"minase({self.rules}): stdout closed "
                    f"(exit={self.proc.poll()}) stderr={stderr!r}"
                )
            if self.log is not None:
                self.log.write(f"> {line}\n")
            if line == f"pong {token}":
                return collected
            if line.strip():
                collected.append(line)

    def close(self) -> None:
        try:
            if self.proc.poll() is None:
                self.send("quit")
        except Exception:
            pass
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(timeout=5)


def replay_game(binary: str, rules: str, game: dict, log) -> dict:
    outcome = {
        "game_index": game["game_index"],
        "rules": rules,
        "hachu_result": game.get("result"),
        "hachu_reason": game.get("reason"),
        "hachu_plies": game.get("plies"),
        "accepted_plies": 0,
        "status": "all_legal",
        "minase_result": None,
        "minase_reason": None,
        "minase_result_ply": None,
        "failure_ply": None,
        "failure_move": None,
        "failure_response": None,
        "context": None,
        "trailing_unplayed": 0,
    }
    engine = Minase(binary, rules, log)
    try:
        engine.send("xboard")
        engine.send("protover 2")
        engine.send("variant chu")
        engine.send("new")
        engine.sync()
        for index, move in enumerate(game["moves"], start=1):
            engine.send(f"usermove {move}")
            lines = engine.sync()
            result_line = None
            problem = None
            for line in lines:
                matched = RESULT_RE.match(line)
                if matched:
                    result_line = matched
                    continue
                if line.startswith("Illegal move") or line.startswith("Error"):
                    problem = line
                    continue
                problem = problem or line
            if problem is not None:
                outcome["status"] = "illegal"
                outcome["failure_ply"] = index
                outcome["failure_move"] = move
                outcome["failure_response"] = problem
                outcome["context"] = game["moves"][max(0, index - 6) : index + 1]
                outcome["trailing_unplayed"] = len(game["moves"]) - index
                break
            outcome["accepted_plies"] = index
            if result_line is not None:
                outcome["minase_result"] = result_line.group(1)
                outcome["minase_reason"] = result_line.group(2)
                outcome["minase_result_ply"] = index
                outcome["trailing_unplayed"] = len(game["moves"]) - index
                outcome["status"] = "finished"
                break
    finally:
        engine.close()
    if outcome["status"] == "all_legal" and outcome["accepted_plies"] == len(game["moves"]):
        outcome["status"] = "all_legal_no_result"
    return outcome


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--games", required=True, help="入力NDJSONのパス")
    parser.add_argument("--minase", required=True, help="minase実行ファイルのパス")
    parser.add_argument(
        "--rules",
        action="append",
        help="照合する規則セット。複数回指定できる（既定はR2,E1とL1,R2,E1）",
    )
    parser.add_argument("--out", help="照合結果NDJSONの出力先")
    parser.add_argument("--log", help="CECP送受信ログの出力先")
    parser.add_argument(
        "--include-abnormal",
        action="store_true",
        help="異常終了した対局も照合対象に含める",
    )
    args = parser.parse_args()

    rule_sets = args.rules or DEFAULT_RULE_SETS
    games = []
    with open(args.games, encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if line:
                games.append(json.loads(line))
    usable = [game for game in games if args.include_abnormal or game.get("ok")]
    skipped = len(games) - len(usable)
    log = open(args.log, "a", encoding="utf-8") if args.log else None
    results = []
    for rules in rule_sets:
        for game in usable:
            outcome = replay_game(args.minase, rules, game, log)
            results.append(outcome)
            print(
                f"rules={rules} game={outcome['game_index']} "
                f"status={outcome['status']} accepted={outcome['accepted_plies']}"
                f"/{outcome['hachu_plies']} "
                f"minase={outcome['minase_result']}({outcome['minase_reason']}) "
                f"hachu={outcome['hachu_result']}({outcome['hachu_reason']})",
                file=sys.stderr,
                flush=True,
            )
    if log is not None:
        log.close()
    if args.out:
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        with open(args.out, "w", encoding="utf-8") as out:
            for outcome in results:
                out.write(json.dumps(outcome, ensure_ascii=False) + "\n")

    print(f"\n=== 照合まとめ（対象{len(usable)}局、除外{skipped}局）===")
    for rules in rule_sets:
        subset = [item for item in results if item["rules"] == rules]
        all_legal = [item for item in subset if item["status"] != "illegal"]
        matched = [
            item
            for item in subset
            if item["minase_result"] is not None
            and item["minase_result"] == item["hachu_result"]
            and item["minase_result_ply"] == item["hachu_plies"]
        ]
        print(
            f"{rules}: 全手合法 {len(all_legal)}/{len(subset)}局、"
            f"終局一致 {len(matched)}/{len(subset)}局"
        )
    print("\n=== 局別の照合結果 ===")
    header = ["game", "HaChu手数", "HaChu結果"] + [f"{r} 受理/応答" for r in rule_sets]
    print(" | ".join(header))
    for game in usable:
        row = [
            str(game["game_index"]),
            str(game["plies"]),
            f"{game['result']}({game['reason']})",
        ]
        for rules in rule_sets:
            item = next(
                x
                for x in results
                if x["rules"] == rules and x["game_index"] == game["game_index"]
            )
            if item["status"] == "illegal":
                row.append(
                    f"{item['accepted_plies']} / 第{item['failure_ply']}手 "
                    f"{item['failure_move']} を拒否"
                )
            elif item["status"] == "finished":
                row.append(
                    f"{item['accepted_plies']} / 第{item['minase_result_ply']}手で"
                    f"{item['minase_result']}({item['minase_reason']})"
                )
            else:
                row.append(f"{item['accepted_plies']} / 全手合法・裁定なし")
        print(" | ".join(row))
    return 0


if __name__ == "__main__":
    sys.exit(main())
