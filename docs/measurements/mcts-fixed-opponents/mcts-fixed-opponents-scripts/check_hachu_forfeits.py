"""HaChuの拒否・不正着手を再生し、既知の反復判定の差かを分類する。

使い方: python3 -I <このファイル> <実行ディレクトリ> --hachu <HaChu> --minase <minase>
JSON配列を出力し、unknownがあれば終了コード1、すべて既知なら0で終了する。
再生の不一致や通信異常は例外にする。拒否の再生では探索せず、旧形式も読める。
不正着手の再生には記録された深さが必要である。候補側の反則は再生せずunknownとする。
問題の着手は開始手順を含む1起算、game_indexは0起算である。
記録に残らず再生もしない着手と、比較できない局面の距離はnullで示す。
"""

import argparse
import json
import os
from pathlib import Path
import re
import selectors
import shlex
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from summarize import MOVE_FORFEITS, engine_side, game_moves, pair_records, read_json


# notation/sfen.rsのLISHOGI_INITIAL_SFEN。盤は内部密番号の順で保持する。
INITIAL_BOARD = (
    "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/"
    "3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL"
)
# harness/engine/cecp.rsの固定制限時の時計と、設計書のHaChuの置換表容量。
FIXED_TIME_CS = 3_000_000
HASH_MB = 256
HISTORY_WINDOW = 200
USI_SQUARE = r"(?:1[0-2]|[1-9])[a-l]"
CECP_SQUARE = r"[a-l](?:1[0-2]|[1-9])"
RESULT_PREFIXES = ("1-0", "0-1", "1/2-1/2", "resign")


def parse_usi(text):
    if not re.fullmatch(rf"(?:{USI_SQUARE}){{2,3}}[+=?]?", text):
        raise ValueError(f"USI着手の表記が不正です: {text}")
    squares = [
        (ord("l") - ord(square[-1])) * 12 + 12 - int(square[:-1])
        for square in re.findall(USI_SQUARE, text)
    ]
    if len(squares) == 2 and squares[0] == squares[1]:
        raise ValueError(f"じっとには中間升が必要です: {text}")
    return squares, text.endswith("+")


def usi_square(square):
    return f"{12 - square % 12}{chr(ord('l') - square // 12)}"


def cecp_square(square):
    return f"{chr(ord('a') + square % 12)}{square // 12 + 1}"


def piece_color(piece):
    return "black" if piece[-1].isupper() else "white"


class Position:
    """合法性の裁定はせず、配置と手番だけを追う。"""

    def __init__(self, cells, side):
        self.cells = cells
        self.side = side

    @classmethod
    def initial(cls):
        cells = []
        for row in reversed(INITIAL_BOARD.split("/")):
            for token in re.findall(r"\d+|\+?[A-Za-z]", row):
                cells.extend([None] * int(token) if token.isdigit() else [token])
        return cls(cells, "black")

    def key(self):
        return tuple(self.cells), self.side

    def play(self, text):
        squares, promote = parse_usi(text)
        piece = self.cells[squares[0]]
        if piece is None or piece_color(piece) != self.side:
            raise ValueError(f"移動元に手番側の駒がありません: {text}")
        if promote and piece.startswith("+"):
            raise ValueError(f"成駒がさらに成る記録です: {text}")
        self.cells[squares[0]] = None
        if len(squares) == 3:
            self.cells[squares[1]] = None
        self.cells[squares[-1]] = "+" + piece if promote else piece
        self.side = "white" if self.side == "black" else "black"

    def jitto(self):
        """反復の禁止を適用する前のじっとを、内部升番号の順で選ぶ。"""
        # harnessのcanonical_jittoとUSIのjitto_intermediateの順序に合わせる。
        # 反復で禁止された手も診断するため、空升への往復を駒の動きから選ぶ。
        for origin, piece in enumerate(self.cells):
            if piece is None or piece_color(piece) != self.side:
                continue
            middle = self.jitto_middle(origin)
            if middle is not None:
                return usi_square(origin) + usi_square(middle) + usi_square(origin)
        raise ValueError("じっとが可能な駒と空升がありません")

    def jitto_middle(self, origin):
        piece = self.cells[origin]
        if piece is None:
            return None
        kind = piece.upper()
        forward = 1 if piece_color(piece) == "black" else -1
        if kind in {"N", "+O"}:
            directions = [(dx, dy) for dy in (-1, 0, 1) for dx in (-1, 0, 1) if dx or dy]
        elif kind == "+H":
            directions = [(0, forward)]
        elif kind == "+D":
            directions = [(-1, forward), (1, forward)]
        else:
            return None
        empty = []
        for dx, dy in directions:
            file, rank = origin % 12 + dx, origin // 12 + dy
            if 0 <= file < 12 and 0 <= rank < 12 and self.cells[rank * 12 + file] is None:
                empty.append(rank * 12 + file)
        return min(empty) if empty else None


def explicit_cecp(usi):
    """じっとも往復する2レグとして表す。反復による拒否の診断に使う。"""
    squares, promote = parse_usi(usi)
    legs = [cecp_square(start) + cecp_square(end) for start, end in zip(squares, squares[1:])]
    return ",".join(legs) + ("+" if promote else "")


def usi_to_cecp(usi, position):
    squares, promote = parse_usi(usi)
    if len(squares) == 3 and position.cells[squares[1]] is None:
        if squares[0] == squares[-1]:
            return "@@@@"
        return cecp_square(squares[0]) + cecp_square(squares[-1]) + ("+" if promote else "")
    return explicit_cecp(usi)


def cecp_to_usi(text, position):
    if text == "@@@@":
        return position.jitto()
    if not re.fullmatch(rf"{CECP_SQUARE}{CECP_SQUARE}(?:,{CECP_SQUARE}{CECP_SQUARE})?[+=]?", text):
        raise ValueError(f"CECP着手の表記が不正です: {text}")
    squares = [
        (int(square[1:]) - 1) * 12 + ord(square[0]) - ord("a")
        for square in re.findall(CECP_SQUARE, text)
    ]
    if len(squares) == 4:
        if squares[1] != squares[2]:
            raise ValueError(f"2段階の着手が連続していません: {text}")
        squares.pop(2)
        if position.cells[squares[1]] is None:
            if squares[0] == squares[-1]:
                middle = position.jitto_middle(squares[0])
                if middle is None:
                    raise ValueError(f"じっとの中間升がありません: {text}")
                squares[1] = middle
            else:
                squares.pop(1)
    elif squares[0] == squares[1]:
        raise ValueError(f"単一レグのじっとは受理できません: {text}")
    return "".join(usi_square(square) for square in squares) + ("+" if text.endswith("+") else "")


def repetition_distance(keys):
    for index in range(len(keys) - 2, -1, -1):
        if keys[index] == keys[-1]:
            return len(keys) - 1 - index
    return None


class CecpEngine:
    """設計書「HaChuとの対局の条件」に従い、記録の応答期限で再生する。"""

    def __init__(self, argv, response_timeout_secs):
        self.response_timeout_secs = response_timeout_secs
        self.process = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
        self.selector = selectors.DefaultSelector()
        self.selector.register(self.process.stdout, selectors.EVENT_READ)
        self.pending = b""

    def __enter__(self):
        return self

    def __exit__(self, exception_type, exception, traceback):
        try:
            if exception_type is None:
                self.send("quit")
                code = self.process.wait(timeout=self.response_timeout_secs)
                if code != 0:
                    raise RuntimeError(f"エンジンが異常終了しました: 終了コード{code}")
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait()
            self.process.stdin.close()
            self.process.stdout.close()
            self.selector.close()

    def send(self, line):
        if "\n" in line or "\r" in line:
            raise ValueError("CECPコマンドに改行を含められません")
        if self.process.poll() is not None:
            raise RuntimeError(f"エンジンが応答の途中で終了しました: {self.process.returncode}")
        self.process.stdin.write((line + "\n").encode("utf-8"))
        self.process.stdin.flush()

    def read_line(self, deadline):
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(f"CECPの応答が{self.response_timeout_secs}秒以内に完了しませんでした")
            if b"\n" in self.pending:
                raw, self.pending = self.pending.split(b"\n", 1)
                line = raw.decode("utf-8").strip()
                if line.startswith(("Error", "tellusererror")):
                    raise RuntimeError(f"エンジンがエラーを返しました: {line}")
                return line
            if not self.selector.select(remaining):
                raise TimeoutError(f"CECPの応答が{self.response_timeout_secs}秒以内に完了しませんでした")
            chunk = os.read(self.process.stdout.fileno(), 65536)
            if not chunk:
                raise RuntimeError("CECPの応答が完了する前に出力が閉じられました")
            self.pending += chunk

    def initialize(self, rules):
        self.send("xboard")
        self.send("protover 2")
        deadline = time.monotonic() + self.response_timeout_secs
        while True:
            line = self.read_line(deadline)
            if line.startswith("feature ") and "done=1" in shlex.split(line)[1:]:
                break
        if rules is not None:
            self.send(f"option RuleSet={rules}")
        for command in (f"memory {HASH_MB}", "new", "variant chu", "easy", "nopost", "force"):
            self.send(command)

    def sync(self, number):
        self.send(f"ping {number}")
        # 設計書「HaChuとの対局の条件」の再生を、ハーネスのreceive_cecp_responseに合わせる。
        # 最初の着手・結果・拒否行までと、その後のpongまでで別々に期限を測る。
        deadline = time.monotonic() + self.response_timeout_secs
        response = []
        while True:
            line = self.read_line(deadline)
            if line == f"pong {number}":
                return response
            if line.startswith("pong"):
                raise RuntimeError(f"pingとpongが一致しません: {number}, {line}")
            if line.startswith(("move ", "Illegal move") + RESULT_PREFIXES):
                if not response:
                    deadline = time.monotonic() + self.response_timeout_secs
                response.append(line)


def received_move(lines):
    if any(line.startswith("Illegal move") for line in lines):
        raise RuntimeError(f"HaChuが再生中の着手を拒否しました: {lines}")
    moves = [line[5:].strip() for line in lines if line.startswith("move ")]
    if len(moves) == 1 and not moves[0].endswith(","):
        return moves[0]
    if len(moves) == 2 and moves[0].endswith(",") and not moves[1].endswith(","):
        return "".join(moves)
    raise RuntimeError(f"HaChuから1手分の着手を受信できませんでした: {lines}")


def replay_forced(argv, moves, final_cecp, rules, response_timeout_secs):
    """全手をforceで送り、最後の着手への応答と配置の履歴を返す。"""
    position = Position.initial()
    keys = [position.key()]
    with CecpEngine(argv, response_timeout_secs) as engine:
        engine.initialize(rules)
        for ply, usi in enumerate(moves, 1):
            engine.send(f"usermove {usi_to_cecp(usi, position)}")
            response = engine.sync(ply)
            if response:
                raise RuntimeError(f"{ply}手目で再生が食い違いました: {response}")
            position.play(usi)
            keys.append(position.key())
        engine.send(f"usermove {final_cecp}")
        response = engine.sync(len(moves) + 1)
    return response, position, keys


def reproduce_illegal_move(hachu, pair, game, depth, response_timeout_secs):
    """HaChu自身の探索を毎手再現し、記録にない最後の着手を得る。"""
    position = Position.initial()
    pending = []
    ply = 0
    for usi in pair["opening"]["moves"]:
        pending.append(usi_to_cecp(usi, position))
        position.play(usi)
        ply += 1
    with CecpEngine([hachu], response_timeout_secs) as engine:
        engine.initialize(None)
        # harness/engine/process.rsではsdを初期化の最後に1回だけ送る。
        engine.send(f"sd {depth}")
        for index, turn in enumerate(game["turns"]):
            if turn["side"] != position.side:
                raise ValueError(f"{ply + 1}手目の記録と配置の手番が一致しません")
            response = turn["response"]
            if turn["side"] != game["candidate_color"]:
                for move in pending:
                    engine.send(f"usermove {move}")
                pending.clear()
                engine.send(f"time {FIXED_TIME_CS}")
                engine.send(f"otim {FIXED_TIME_CS}")
                engine.send("go")
                lines = engine.sync(ply)
                move = received_move(lines)
                engine.send("force")
                if index == len(game["turns"]) - 1:
                    if response != {"kind": "failure", "reason": "illegal_move"}:
                        raise ValueError("最後の手番がHaChuの不正着手ではありません")
                    # 表記自体が不正な着手も、既知の反復差には分類しない。
                    try:
                        usi = cecp_to_usi(move, position)
                    except ValueError:
                        usi = None
                    return move, usi, lines
                usi = cecp_to_usi(move, position)
                if response["kind"] != "move" or usi != response["usi"]:
                    raise RuntimeError(f"{ply + 1}手目が一致しません: 再生{usi}, 記録{response}")
            else:
                if response["kind"] != "move":
                    raise ValueError(f"再生中の候補の手が着手ではありません: {response}")
                usi = response["usi"]
                pending.append(usi_to_cecp(usi, position))
            position.play(usi)
            ply += 1
    raise ValueError("HaChuの不正着手の手番が見つかりません")


def classify_repetition(reason, response, distance, cecp):
    if distance is None:
        return "unknown"
    # hachu.c 2714、2731〜2736行。@@@@はどのじっとにも合うが、拒否理由の照合には合わない。
    if (reason == "rejected_move" and distance <= HISTORY_WINDOW
            and ("Illegal move {Repeats earlier position}" in response
                 or (cecp == "@@@@" and "Illegal move" in response))):
        return "placement_only_repetition"
    if (reason == "illegal_move" and distance > HISTORY_WINDOW
            and any(line.startswith("Illegal move (repetition)") for line in response)):
        return "history_window"
    return "unknown"


def classify_game(manifest, pair, game_index, hachu, minase):
    game = pair["games"][game_index]
    termination = game["termination"]
    reason = termination["reason"]
    loser = engine_side(termination["loser"], game["candidate_color"])
    moves = game_moves(pair, game)
    result = {
        "pair_number": pair["pair_number"],
        "game_index": game_index,
        "candidate_color": game["candidate_color"],
        "reason": reason,
        "loser": loser,
        "loser_color": termination["loser"],
        "ply": len(moves) + (reason == "illegal_move"),
        "cecp": None,
        "usi": moves[-1] if reason == "rejected_move" and moves else None,
        "hachu_response_lines": [],
        "minase_response_lines": [],
        "repetition_distance": None,
        "classification": "unknown",
    }
    if loser == "candidate":
        return result
    response_timeout_secs = manifest["response_timeout_secs"]
    if reason == "rejected_move":
        if not game["turns"] or game["turns"][-1]["response"]["kind"] != "move":
            raise ValueError("拒否された最後の着手が記録されていません")
        position = Position.initial()
        for usi in moves[:-1]:
            position.play(usi)
        result["cecp"] = usi_to_cecp(moves[-1], position)
        response, position, keys = replay_forced([hachu], moves[:-1], result["cecp"], None, response_timeout_secs)
        position.play(moves[-1])
        keys.append(position.key())
        result["hachu_response_lines"] = response
    else:
        limit = manifest["baseline"]["limit"]
        if "depth" not in limit or type(limit["depth"]) is not int or limit["depth"] < 1:
            raise ValueError("HaChuの不正着手の再生には記録された1以上の深さが必要です")
        move, usi, lines = reproduce_illegal_move(hachu, pair, game, limit["depth"], response_timeout_secs)
        result.update(cecp=move, usi=usi, hachu_response_lines=lines)
        # @@@@は合法手集合から選ばれるため、反復で全候補が消えると理由を失う。
        # 最後のじっとは明示的な往復として送り、審判の反復判定まで到達させる。
        final_cecp = explicit_cecp(usi) if move == "@@@@" and usi is not None else move
        response, position, keys = replay_forced(
            [minase, "--protocol", "cecp"], moves, final_cecp, manifest["rules_source"], response_timeout_secs
        )
        result["minase_response_lines"] = response
        if usi is None:
            return result
        try:
            position.play(usi)
        except ValueError:
            # 空升から動くなど配置を追えない不正着手も、応答を残してunknownにする。
            return result
        keys.append(position.key())
    distance = repetition_distance(keys)
    result["repetition_distance"] = distance
    result["classification"] = classify_repetition(reason, response, distance, result["cecp"])
    return result


def check_run(run_dir, hachu, minase):
    manifest = read_json(Path(run_dir) / "manifest.json")
    if "response_timeout_secs" not in manifest:
        raise ValueError("manifest.jsonにresponse_timeout_secsがありません")
    baseline = manifest["baseline"]["identity"]
    if baseline["kind"] != "command" or baseline["protocol"] != "cecp":
        raise ValueError("基準がCECPのHaChuである実行ディレクトリを指定してください")
    results = []
    for pair in pair_records(run_dir):
        for index, game in enumerate(pair["games"]):
            termination = game["termination"]
            if termination["kind"] == "forfeit" and termination["reason"] in MOVE_FORFEITS:
                results.append(classify_game(manifest, pair, index, hachu, minase))
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_dir", type=Path)
    parser.add_argument("--hachu", required=True)
    parser.add_argument("--minase", required=True)
    args = parser.parse_args()
    results = check_run(args.run_dir, args.hachu, args.minase)
    json.dump(results, sys.stdout, ensure_ascii=False, indent=2, allow_nan=False)
    print()
    return int(any(result["classification"] == "unknown" for result in results))


if __name__ == "__main__":
    raise SystemExit(main())
