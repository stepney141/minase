#!/usr/bin/env python3
"""minaseとhaitaka-variantsの中将棋の合法手と終局の判定を、同じ局面で突き合わせる。

haitakaをminaseと対局させる前に、審判層になるminaseの規則セットとhaitakaの規則の
差を調べるために使う（docs/lessons/verify-external-engine-before-cecp-match.md）。
初期局面から乱数で選んだ着手で対局を進め、各局面で、minaseがUSIの`moves`と`state`で
返す合法手と対局の状態を、`haitaka_cli json moves`と`haitaka_cli json inspect`の
結果と比べる。次の着手は、双方が合法とした手から選ぶ。

    python3 scripts/haitaka/haitaka_rule_diff.py \
        --minase target/release/minase --haitaka <haitaka_cliのパス> \
        --games 20 --max-plies 400 --seed 1

着手は、獅子、角鷹、および飛鷲の2段階移動を次の形へそろえてから比べる。
経由升に相手の駒がある着手は移動元、経由升、到達升の3升で表し、相手の駒がない
着手は移動元と到達升の2升で表す。haitakaは空の経由升を通る経路を別の着手として
列挙し、minaseはそれを経由升のない着手と同じに扱うからである。

駒を取る手が少ない一様な乱数の対局では、獅子の捕獲制限と成りの規則がほとんど
試されない。そこで`--capture-rate`の確率で、駒を取る手だけから次の着手を選ぶ。
最後の王駒を取る手は、ほかに手がある限り選ばず、対局を長く続ける。

結果はJSONで標準出力へ書く。1局面でも食い違えば終了コード1で終わる。
"""

import argparse
import json
import random
import subprocess
import sys

from haitaka_usi import RULES, Haitaka, move_to_usi, usi_to_move

# haitakaの駒IDのうち、王駒（玉将と太子）。
ROYAL_PIECES = ("ah", "cg")
WINNERS = ("black", "white")


class Minase:
    """USIで起動したminaseへの照会。"""

    def __init__(self, path, rules):
        self.process = subprocess.Popen(
            [path, "--protocol", "usi"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True
        )
        self.ask("usi", "usiok")
        self.send(f"setoption name RuleSet value {rules}")

    def send(self, line):
        self.process.stdin.write(line + "\n")
        self.process.stdin.flush()

    def ask(self, line, prefix):
        """1行を送り、`prefix`で始まる応答の行を返す。エラーの行は例外にする。"""
        self.send(line)
        while True:
            response = self.process.stdout.readline()
            if not response:
                raise RuntimeError(f"minase exited while handling '{line}'")
            if response.startswith("info string error"):
                raise RuntimeError(f"minase rejected '{line}': {response.strip()}")
            if response.startswith(prefix):
                return response.split()

    def query(self, moves):
        """着手列の後の対局の状態と合法手を返す。終局した局面の合法手は空にする。

        状態は`ongoing`、`win black`、`win white`、または`draw`であり、終局の理由は落とす。
        """
        self.send("usinewgame")
        self.send("position startpos" + (" moves " + " ".join(moves) if moves else ""))
        state = self.ask("state", "state")
        result = state[state.index("status") + 1 :]
        status = " ".join(result[:2]) if result[0] == "win" else result[0]
        return status, (self.ask("moves", "moves")[1:] if status == "ongoing" else [])

    def close(self):
        self.send("quit")
        self.process.wait()


def enemy_royals(board, side):
    """相手の王駒がある升の一覧を返す。"""
    return [
        at
        for at, piece in board.items()
        if piece["color"] != side and piece["piece"] in ROYAL_PIECES
    ]


def canonical(move, board, side):
    """2段階移動の書き方の差をそろえた着手の文字列を返す。

    経由升に相手の駒がなければ経由升を落とす。経由升で相手の最後の王駒を取る着手は、
    その時点で対局が終わるので、到達升を落とす。haitakaはこの着手を第1段階だけの
    経路として列挙し、minaseは第2段階ごとに別の着手として列挙する。
    """
    path = move["path"]
    if len(path) == 2:
        via = board.get(tuple(path[0]))
        if via is None or via["color"] == side:
            move = {**move, "path": path[1:]}
        elif enemy_royals(board, side) == [tuple(path[0])]:
            move = {**move, "path": path[:1]}
    return move_to_usi(move)


def captured(move, board, side):
    """着手が取る相手の駒のIDを返す。"""
    pieces = (board.get(tuple(at)) for at in move["path"])
    return [piece["piece"] for piece in pieces if piece and piece["color"] != side]


def choose(rng, moves, board, side, capture_rate):
    """双方が合法とした着手から、次の着手を選ぶ。"""
    royals = len(enemy_royals(board, side))
    safe = [
        move
        for move in moves
        if sum(1 for piece in captured(move, board, side) if piece in ROYAL_PIECES) < royals
    ]
    captures = [move for move in safe if captured(move, board, side)]
    if captures and rng.random() < capture_rate:
        return rng.choice(captures)
    return rng.choice(safe or moves)


def play(rng, minase, haitaka, max_plies, capture_rate):
    """1局を進め、比べた局面の数と食い違いの一覧を返す。"""
    game = haitaka.start()
    usi_moves = []
    mismatches = []
    for ply in range(max_plies + 1):
        position = json.loads(game)["position"]
        side = position["sideToMove"]
        board = {tuple(piece["at"]): piece for piece in position["pieces"]}
        outcome = json.loads(haitaka.run("inspect", stdin=game))["outcome"]
        haitaka_status = outcome["kind"]
        if outcome["winner"] is not None:
            haitaka_status += f" {WINNERS[outcome['winner']]}"
        haitaka_moves = {
            canonical(move, board, side): move
            for move in json.loads(haitaka.run("moves", stdin=game))
        }
        minase_status, minase_texts = minase.query(usi_moves)
        minase_moves = {canonical(usi_to_move(text), board, side): text for text in minase_texts}
        common = sorted(haitaka_moves.keys() & minase_moves.keys())
        if haitaka_status != minase_status or haitaka_moves.keys() != minase_moves.keys():
            mismatches.append(
                {
                    "moves": " ".join(usi_moves),
                    "minase_status": minase_status,
                    "haitaka_status": haitaka_status,
                    "only_minase": sorted(minase_moves.keys() - haitaka_moves.keys()),
                    "only_haitaka": sorted(haitaka_moves.keys() - minase_moves.keys()),
                }
            )
        if not common or ply == max_plies:
            return ply + 1, mismatches
        move = choose(rng, [haitaka_moves[key] for key in common], board, side, capture_rate)
        game = haitaka.apply(game, move)
        usi_moves.append(minase_moves[canonical(move, board, side)])
    raise AssertionError("unreachable")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--minase", required=True, help="minaseの実行ファイルのパス")
    parser.add_argument("--haitaka", required=True, help="haitaka_cliのパス")
    parser.add_argument("--games", type=int, required=True, help="対局の数")
    parser.add_argument("--max-plies", type=int, required=True, help="1局の手数の上限")
    parser.add_argument("--seed", type=int, required=True, help="乱数のシード")
    parser.add_argument(
        "--capture-rate", type=float, default=0.3, help="駒を取る手だけから選ぶ確率"
    )
    args = parser.parse_args()
    rng = random.Random(args.seed)
    minase = Minase(args.minase, RULES)
    haitaka = Haitaka(args.haitaka)
    positions = 0
    mismatches = []
    for game in range(args.games):
        compared, found = play(rng, minase, haitaka, args.max_plies, args.capture_rate)
        positions += compared
        mismatches.extend({"game": game, **mismatch} for mismatch in found)
    minase.close()
    json.dump(
        {"positions": positions, "mismatched_positions": len(mismatches), "mismatches": mismatches},
        sys.stdout,
        ensure_ascii=False,
        indent=1,
    )
    print()
    return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
