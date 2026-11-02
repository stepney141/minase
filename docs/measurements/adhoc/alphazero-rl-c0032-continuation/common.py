"""駒の損得の概算と、対局記録の再生。

駒価値は歩を1とする比較用の概算であり、エンジンの評価値とは関係がない。
"""

import glob
import json
import math
import re

# 2026年11月の時点の埋め込みNNUEの勝率尺度（1ロジットあたりのcp）。
CP_PER_LOGIT = 1072.65

VALUES = dict(
    Pawn=1, GoBetween=2, Lance=3, FerociousLeopard=3, CopperGeneral=3, SilverGeneral=4, GoldGeneral=4,
    BlindTiger=4, DrunkElephant=4, ReverseChariot=5, SideMover=6, VerticalMover=6, Bishop=8, Phoenix=6,
    Kirin=6, Rook=10, DragonHorse=12, DragonKing=14, FreeKing=20, Lion=30, King=0, CrownPrince=0,
    WhiteHorse=10, Whale=10, FlyingOx=14, FreeBoar=12, FlyingStag=8, HornedFalcon=16, SoaringEagle=18,
)
PROMOTED = dict(
    Pawn=4, GoBetween=4, Lance=10, ReverseChariot=10, SideMover=12, VerticalMover=14, Bishop=12, Rook=14,
    DragonHorse=16, DragonKing=18, DrunkElephant=0, FerociousLeopard=8, BlindTiger=8, CopperGeneral=6,
    SilverGeneral=6, GoldGeneral=10, Kirin=30, Phoenix=20,
)
JAPANESE = {
    "歩": "Pawn", "仲": "GoBetween", "香": "Lance", "猛": "FerociousLeopard", "銅": "CopperGeneral",
    "銀": "SilverGeneral", "金": "GoldGeneral", "盲": "BlindTiger", "醉": "DrunkElephant",
    "反": "ReverseChariot", "横": "SideMover", "竪": "VerticalMover", "角": "Bishop", "鳳": "Phoenix",
    "麒": "Kirin", "飛": "Rook", "馬": "DragonHorse", "龍": "DragonKing", "奔": "FreeKing", "獅": "Lion",
    "王": "King", "玉": "King",
}
# RULES.md第5条の初期配置。各段の左端が12筋である。
INITIAL = [
    "香 猛 銅 銀 金 醉 玉 金 銀 銅 猛 香",
    "反 . 角 . 盲 鳳 麒 盲 . 角 . 反",
    "横 竪 飛 馬 龍 奔 獅 龍 馬 飛 竪 横",
    "歩 歩 歩 歩 歩 歩 歩 歩 歩 歩 歩 歩",
    ". . . 仲 . . . . 仲 . . .",
    ". . . . . . . . . . . .",
    ". . . . . . . . . . . .",
    ". . . 仲 . . . . 仲 . . .",
    "歩 歩 歩 歩 歩 歩 歩 歩 歩 歩 歩 歩",
    "横 竪 飛 馬 龍 獅 奔 龍 馬 飛 竪 横",
    "反 . 角 . 盲 麒 鳳 盲 . 角 . 反",
    "香 猛 銅 銀 金 王 醉 金 銀 銅 猛 香",
]
MOVE = re.compile(r"^(\d+)([a-l])(\d+)([a-l])(?:(\d+)([a-l]))?(\+)?$")
MATERIAL_BINS = [(-999, -40), (-40, -20), (-20, -10), (-10, -5), (-5, 0), (0, 1), (1, 5), (5, 10), (10, 20), (20, 999)]


def is_royal(piece):
    return piece[0] == "King" or (piece[0] == "DrunkElephant" and piece[2])


def piece_value(piece):
    return PROMOTED.get(piece[0], VALUES[piece[0]]) if piece[2] else VALUES[piece[0]]


def final_square(usi):
    m = MOVE.match(usi)
    if m is None:
        raise ValueError(f"unknown move: {usi}")
    file, rank = (m[5], m[6]) if m[5] else (m[3], m[4])
    return (int(file), ord(rank) - ord("a"))


def win_probability(cp):
    return 1.0 / (1.0 + math.exp(-cp / CP_PER_LOGIT))


class Board:
    """王駒を含む全駒の位置だけを持つ、再生専用の盤。合法性は検査しない。"""

    def __init__(self):
        self.pieces = {}
        for rank, row in enumerate(INITIAL):
            for index, name in enumerate(row.split()):
                if name != ".":
                    owner = "black" if rank > 5 else "white"
                    self.pieces[(12 - index, rank)] = [JAPANESE[name], owner, False]

    def play(self, usi, mover):
        """1手を適用して、取った駒の一覧を返す。動かした駒の持ち主が手番と違えば止める。"""
        m = MOVE.match(usi)
        if m is None:
            raise ValueError(f"unknown move: {usi}")
        square = lambda f, r: (int(f), ord(r) - ord("a"))
        origin, second = square(m[1], m[2]), square(m[3], m[4])
        piece = self.pieces.pop(origin)
        if piece[1] != mover:
            raise ValueError(f"{usi}: the piece on the origin belongs to {piece[1]}, not {mover}")
        captured = []
        if m[5]:
            final = square(m[5], m[6])
            if second != origin and second in self.pieces:
                captured.append(self.pieces.pop(second))
            if final in self.pieces:
                captured.append(self.pieces.pop(final))
        else:
            final = second
            if final in self.pieces:
                captured.append(self.pieces.pop(final))
        self.pieces[final] = piece
        if m[7]:
            piece[2] = True
        return captured

    def copy(self):
        board = Board.__new__(Board)
        board.pieces = {square: list(piece) for square, piece in self.pieces.items()}
        return board

    def material(self, side):
        total = 0
        for piece in self.pieces.values():
            total += piece_value(piece) if piece[1] == side else -piece_value(piece)
        return total


def pair_files(run_dir):
    return sorted(glob.glob(f"{run_dir}/pairs/*.json"))


def opening_moves(pair):
    return [m if isinstance(m, str) else m["usi"] for m in pair["opening"]["moves"]]


def replay(game, opening):
    """開始局面を適用した盤から、各手番の(手数, 手順, 盤, 手番の記録)を順に返す。盤は着手の前の状態である。"""
    board = Board()
    history = []
    side = "black"
    for usi in opening:
        board.play(usi, side)
        history.append(usi)
        side = "white" if side == "black" else "black"
    for turn in game["turns"]:
        response = turn["response"]
        if response.get("kind") != "move":
            break
        if turn["side"] != side:
            raise ValueError("the record does not alternate sides")
        yield len(history), history, board, turn
        board.play(response["usi"], side)
        history.append(response["usi"])
        side = "white" if side == "black" else "black"


def load_pair(path):
    with open(path) as f:
        return json.load(f)
