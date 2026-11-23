#!/usr/bin/env python3
"""haitaka-variantsの中将棋エンジンを、USIエンジンとして対局ハーネスへつなぐアダプタ。

haitaka-variants（https://github.com/na2hiro/haitaka-variants）の中将棋は
USIを実装せず、局面と着手を独自のJSON（haitaka-json-v2）で受け渡す
`haitaka_cli json`だけを入口に持つ。本スクリプトは標準入力からUSIのコマンドを
受け取り、`haitaka_cli json apply`で着手を適用し、`haitaka_cli json search`で
探索した結果を`bestmove`として返す。`minase match run`の`--candidate`または
`--baseline`に、本スクリプトの起動コマンドをUSIエンジンとして指定する。

    python3 scripts/haitaka/haitaka_usi.py <haitaka_cliのパス>

`haitaka_cli`は、haitaka-variantsのリポジトリで
`cargo build --release -p haitaka_cli --features chushogi`を実行して作る。

haitakaは将棋ったーの中将棋（ルール48）だけを実装し、規則を切り替えられない。
minaseの規則コード（docs/rules/chu-shogi.md 第10章）では`L1,P0,P3,R1,E1,E2`が
最も近いので、アダプタは`RuleSet`にこの組だけを受け付ける。haitakaのソース
（haitaka/src/extended/chu.rs）とこの組の間には、次の差が残る。

- 反復の裁定では、haitakaは同一局面の4回目で必ず裁定し、連続して王手をかけ
  続けた側を負け、それ以外を引き分けとする。R1は可逆手が12手以上続いている
  ことを裁定の前提とし、負けとする攻撃的着手を王手に限らない。このため、
  haitakaが引き分けと読んで繰り返した着手を、審判層がhaitakaの負けと裁定する
  ことがある。1手1秒の40局では5局がこの裁定で終わった
  （docs/measurements/adhoc/haitaka-movetime1000-elo.md）。
- 先獅子の判定では、haitakaは2段階移動の段階ごとに先獅子の状態を更新する。
  角鷹または飛鷲が第1段階で獅子を取り、第2段階で獅子以外を取るか空升へ進むと、
  haitakaでは先獅子が成立せず、minaseのL1では成立する。この直後にhaitakaが
  獅子以外の駒で獅子を取り返すと、審判層はhaitakaの不正着手と判定する。
- 隣接する獅子の捕獲では、獅子が第1段階で歩兵または仲人を取り、第2段階で
  移動元に隣接する相手の獅子を取る着手を、haitakaは相手の獅子に足があれば
  不合法とする。minaseはdocs/rules/chu-shogi.md 第16条第12項により合法とする。
  `haitaka_rule_diff.py`の乱数の対局では、24,060局面のうち9局面でminaseだけが
  この着手を合法とし、ほかの食い違いはなかった。

審判層が続行とする局面をhaitakaが終局と判定した場合と、審判層が合法とした
着手をhaitakaが拒否した場合は、理由を標準エラーへ書いて終了コード1で終わる。
対局ハーネスはこれをhaitakaの側の異常（クラッシュ）として数える。

haitakaの探索は1手ごとの打ち切り時間、深さ、およびノード数だけを受け取り、
持ち時間を配分する機能を持たない。したがって`go`は`movetime`、`depth`、および
`nodes`だけを受け付ける。`movetime`では、`position`を受け取ってから探索を
始めるまでにかかった時間を打ち切り時間から引く。探索は`haitaka_cli`の
プロセスを手ごとに起動して行うので、置換表は手をまたいで残らない。
"""

import argparse
import json
import re
import subprocess
import sys
import time

RULES = "L1,P0,P3,R1,E1,E2"

# haitakaの`search`が受け付ける深さの上限と、打ち切り時間（u32のミリ秒）の上限。
MAX_DEPTH = 100
MAX_TIMEOUT_MS = 2**32 - 1

SQUARE_RE = re.compile(r"(1[0-2]|[1-9])([a-l])")
MOVE_RE = re.compile(r"(?:(?:1[0-2]|[1-9])[a-l]){2,3}\+?")


class AdapterError(RuntimeError):
    """対局を続けられない入力、またはhaitakaの失敗。"""


def usi_to_move(text):
    """lishogi系のUSIの着手をhaitaka-json-v2の着手へ変換する。

    升は`[筋, 段]`で、筋はUSIの数字、段はUSIの英字を1始まりの番号にした値である。
    3升の形式は経由升と到達升の2個の経路、2升の形式は到達升だけの経路にする。
    haitakaは空の経由升を通る経路も合法手に含めるので、盤面を見ずに変換できる。
    """
    if not MOVE_RE.fullmatch(text):
        raise AdapterError(f"unsupported move text '{text}'")
    squares = [[int(file), ord(rank) - ord("a") + 1] for file, rank in SQUARE_RE.findall(text)]
    return {
        "schemaVersion": 2,
        "from": squares[0],
        "path": squares[1:],
        "promotion": text.endswith("+"),
    }


def move_to_usi(move):
    """haitaka-json-v2の着手をlishogi系のUSIの着手へ変換する。"""
    squares = [move["from"], *move["path"]]
    text = "".join(f"{file}{chr(ord('a') + rank - 1)}" for file, rank in squares)
    return text + ("+" if move["promotion"] else "")


class Haitaka:
    """`haitaka_cli json`の1回ごとの呼び出し。"""

    def __init__(self, cli):
        self.cli = cli

    def run(self, *args, stdin=""):
        """`haitaka_cli json <args>`を実行し、標準出力を返す。失敗は例外にする。"""
        result = subprocess.run(
            [self.cli, "json", *args], input=stdin, capture_output=True, text=True, check=False
        )
        if result.returncode != 0:
            raise AdapterError(
                f"haitaka_cli json {' '.join(args)} failed: {result.stderr.strip()}"
            )
        return result.stdout

    def start(self):
        """初期局面の対局JSONを返す。"""
        return self.run("startpos")

    def apply(self, game, move):
        """対局JSONに着手を適用し、履歴を含む新しい対局JSONを返す。"""
        return self.run("apply", "--move", json.dumps(move), stdin=game)


class Engine:
    """USIのコマンドを1行ずつ処理する。"""

    def __init__(self, haitaka):
        self.haitaka = haitaka
        self.rules_set = False
        self.start = None
        # 現局面の対局JSON、そこまでに適用したUSIの着手列、および`position`の受信時刻。
        self.game = None
        self.applied = []
        self.position_received = None

    def handle(self, line):
        """1行を処理する。`quit`なら偽を返す。"""
        words = line.split()
        if not words:
            return True
        command, args = words[0], words[1:]
        if command == "quit":
            return False
        if command == "usi":
            print("id name haitaka-variants chushogi")
            print("option name Threads type spin default 1 min 1 max 1")
            print("usiok")
        elif command == "setoption":
            self.setoption(args)
        elif command == "isready":
            if not self.rules_set:
                raise AdapterError("required option RuleSet is not set")
            self.start = self.haitaka.start()
            print("readyok")
        elif command in ("usinewgame", "gameover"):
            self.game = None
            self.applied = []
        elif command == "position":
            self.position(args)
        elif command == "go":
            self.go(args)
        elif command != "stop":
            raise AdapterError(f"unknown command '{command}'")
        return True

    def setoption(self, args):
        """`RuleSet`を検査する。宣言していない名前は、USIの慣例どおり無視する。"""
        if args[:2] != ["name", "RuleSet"]:
            return
        value = " ".join(args[3:])
        if args[2:3] != ["value"] or frozenset(value.split(",")) != frozenset(RULES.split(",")):
            raise AdapterError(f"RuleSet must be {RULES}: '{value}'")
        self.rules_set = True

    def position(self, args):
        """`position startpos [moves ...]`を受け、前回からの差分の着手だけを適用する。"""
        self.position_received = time.monotonic()
        if self.start is None:
            raise AdapterError("position requires isready")
        if args[:1] != ["startpos"] or (len(args) > 1 and args[1] != "moves"):
            raise AdapterError("position accepts only 'startpos [moves ...]'")
        moves = args[2:]
        if self.game is None or moves[: len(self.applied)] != self.applied:
            self.game = self.start
            self.applied = []
        for text in moves[len(self.applied) :]:
            self.game = self.haitaka.apply(self.game, usi_to_move(text))
            self.applied.append(text)

    def go(self, args):
        """現局面を探索し、`info`と`bestmove`を出す。"""
        if self.game is None:
            raise AdapterError("go requires position")
        if len(args) % 2 != 0 or not args:
            raise AdapterError(f"unsupported go arguments '{' '.join(args)}'")
        limits = {}
        for name, value in zip(args[::2], args[1::2]):
            if name not in ("movetime", "depth", "nodes") or name in limits or not value.isdigit():
                raise AdapterError(f"unsupported go arguments '{' '.join(args)}'")
            limits[name] = int(value)
        timeout_ms = MAX_TIMEOUT_MS
        if "movetime" in limits:
            spent_ms = (time.monotonic() - self.position_received) * 1000
            timeout_ms = int(limits["movetime"] - spent_ms)
            if timeout_ms < 1:
                raise AdapterError(
                    f"movetime {limits['movetime']} ms leaves no time"
                    f" after {spent_ms:.0f} ms of setup"
                )
        result = json.loads(
            self.haitaka.run(
                "search",
                "--depth",
                str(limits.get("depth", MAX_DEPTH)),
                "--timeout-ms",
                str(timeout_ms),
                "--nodes",
                str(limits.get("nodes", 0)),
                stdin=self.game,
            )
        )
        if result["bestMove"] is None:
            raise AdapterError(
                f"haitaka has no move after '{' '.join(self.applied)}': outcome {result['outcome']}"
            )
        if result["completedDepth"] > 0:
            print(
                f"info depth {result['completedDepth']} score cp {result['score']}"
                f" nodes {result['states']} time {round(result['elapsedMs'])}"
            )
        print(f"bestmove {move_to_usi(result['bestMove'])}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("haitaka_cli", help="chushogiのfeatureでビルドしたhaitaka_cliのパス")
    engine = Engine(Haitaka(parser.parse_args().haitaka_cli))
    for line in sys.stdin:
        try:
            if not engine.handle(line):
                break
        except AdapterError as error:
            print(f"haitaka_usi: {error}", file=sys.stderr)
            return 1
        sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
