"""lishogiの中将棋棋譜を再生し、麒麟（成って獅子になった後も追跡）の筋の分布を集計する。"""
import collections
import json
import re
import sys

START = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL"
SQ = re.compile(r"(\d{1,2})([a-l])")


def parse_board(sfen_board):
    board = {}
    for r, row in enumerate(sfen_board.split("/")):
        f = 12
        i = 0
        while i < len(row):
            ch = row[i]
            if ch.isdigit():
                j = i
                while j < len(row) and row[j].isdigit():
                    j += 1
                f -= int(row[i:j])
                i = j
                continue
            prom = ch == "+"
            if prom:
                i += 1
                ch = row[i]
            board[(f, "abcdefghijkl"[r])] = {"p": ch.lower(), "side": "sente" if ch.isupper() else "gote", "prom": prom, "id": None}
            f -= 1
            i += 1
    return board


def replay(game):
    sfen = game.get("initialSfen", START)
    board_part, *rest = sfen.split()
    board = parse_board(board_part)
    turn = "sente"
    if rest and rest[0] == "w":
        turn = "gote"
    for sq, pc in board.items():
        if pc["p"] == "o" and not pc["prom"]:
            pc["id"] = ("kirin", pc["side"], sq)
        if pc["p"] == "x" and not pc["prom"]:
            pc["id"] = ("phoenix", pc["side"], sq)
    traj = collections.defaultdict(list)  # id -> [(ply, square, promoted)]
    for sq, pc in board.items():
        if pc["id"]:
            traj[pc["id"]].append((0, sq, pc["prom"]))
    for ply, mv in enumerate(game["moves"].split(), 1):
        promote = mv.endswith("+")
        sqs = [(int(a), b) for a, b in SQ.findall(mv)]
        frm, to = sqs[0], sqs[-1]
        pc = board.pop(frm)
        assert pc["side"] == turn, (game["id"], ply, mv)
        for mid in sqs[1:-1]:
            if mid in board and mid != frm:
                cap = board.pop(mid)
                if cap["id"]:
                    traj[cap["id"]].append((ply, None, None))
        if to in board:
            cap = board.pop(to)
            if cap["id"]:
                traj[cap["id"]].append((ply, None, None))
        if promote:
            pc["prom"] = True
        board[to] = pc
        if pc["id"]:
            traj[pc["id"]].append((ply, to, pc["prom"]))
        turn = "gote" if turn == "sente" else "sente"
    return traj, ply


def main(path, me):
    out = []
    for line in open(path):
        g = json.loads(line)
        if not g.get("moves"):
            continue
        me_side = "sente" if g["players"]["sente"].get("user", {}).get("id") == me else "gote"
        traj, nply = replay(g)
        out.append({"id": g["id"], "me": me_side, "nply": nply, "traj": {"|".join(map(str, [k[0], k[1], k[2][0], k[2][1]])): v for k, v in traj.items()}, "created": g["createdAt"], "initial": "initialSfen" in g, "status": g["status"], "winner": g.get("winner")})
    json.dump(out, open(sys.argv[3], "w"))


main(sys.argv[1], sys.argv[2])
