"""事前登録した6根で、教師最善手とS0提案手の子局面を再探索し、読み筋と駒の損得を出力する。"""
import json
import struct
import subprocess
import sys
from pathlib import Path

WT = Path("/home/stepney141/board-games/minase/data/worktrees/relative-pair-eval")
BIN = WT / "target/release/minase"
SA = Path("/home/stepney141/board-games/minase/data/search-aware-evaluation")
M = 142.1
NODES = 10_000_000

KIND = {
    "P": 0, "I": 1, "L": 2, "A": 3, "M": 4, "V": 5, "B": 6, "R": 7, "H": 8, "D": 9,
    "Q": 10, "K": 11, "E": 12, "F": 13, "T": 14, "C": 15, "S": 16, "G": 17, "O": 18,
    "X": 19, "N": 20,
}
PROMOTE = {0: 17, 1: 12, 2: 22, 3: 23, 4: 25, 5: 24, 6: 8, 7: 9, 8: 27, 9: 28,
           12: 21, 13: 6, 14: 26, 15: 4, 16: 5, 17: 7, 18: 20, 19: 10}
PROMOTABLE = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13, 14, 15, 16, 17, 18, 19]
UNPROMOTED_STATE = {k: 29 + i for i, k in enumerate(PROMOTABLE)}

body = (WT / "nets/pst.bin").read_bytes()[80:]
VALUES = [struct.unpack_from("<i", body, 13680 * 4 + s * 4)[0] for s in range(47)]


def state_of(letter, promoted):
    kind = KIND[letter.upper()]
    if promoted:
        return PROMOTE[kind]  # 成駒の駒種番号がそのまま状態になる
    return UNPROMOTED_STATE.get(kind, kind)


def material(sfen_board, black_view):
    total = 0
    promoted = False
    for ch in sfen_board:
        if ch == "+":
            promoted = True
            continue
        if ch.isalpha():
            v = VALUES[state_of(ch, promoted)]
            if ch.upper() == "K" or state_of(ch, promoted) == 21:
                v = 0  # 王駒は駒の損得に数えない
            total += v if ch.isupper() == black_view else -v
        promoted = False
    return total


class Engine:
    def __init__(self):
        self.p = subprocess.Popen([str(BIN), "--protocol", "usi", "--rules", "engine-default"],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1)
        self.send("usi"); self.wait("usiok")
        self.send("setoption name USI_Hash value 64"); self.send("setoption name Threads value 1")
        self.send("isready"); self.wait("readyok")
        self.send("usinewgame")

    def send(self, s):
        self.p.stdin.write(s + "\n")

    def wait(self, prefix):
        lines = []
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("engine exited: " + "".join(lines[-5:]))
            lines.append(line.rstrip("\n"))
            if line.startswith(prefix):
                return lines

    def board(self, moves):
        self.send("position startpos moves " + " ".join(moves) if moves else "position startpos")
        self.send("state")
        line = self.wait("state")[-1]
        return line.split(" board ", 1)[1].split(" status ", 1)[0]

    def search(self, moves, nodes):
        self.send("position startpos moves " + " ".join(moves))
        self.send(f"go nodes {nodes}")
        lines = self.wait("bestmove")
        info = [l for l in lines if l.startswith("info") and " pv " in l and " score " in l]
        last = info[-1].split()
        score = (last[last.index("score") + 1], int(last[last.index("score") + 2]))
        depth = int(last[last.index("depth") + 1])
        pv = last[last.index("pv") + 1:]
        return score, depth, pv, lines[-1]

    def close(self):
        self.send("quit"); self.p.wait()


def main():
    report = json.loads((SA / "phase4/final/report.json").read_text())["record"]
    finals = {}
    for line in (SA / "phase4/final/roots.jsonl").open():
        r = json.loads(line)["record"]
        finals[r["id"]] = r
    roots = {}
    for line in (SA / "phase3/roots.jsonl").open():
        r = json.loads(line)["record"]["data"]
        roots[r["id"]] = r
    games = {}
    for path in SA.glob("phase3/games-*.jsonl"):
        for line in path.open():
            g = json.loads(line)["record"]["data"]
            games[g["seed"]] = g["moves"]
    targets = [d for d in report["decisions"]
               if d["category"] == "stable" and d["loss"]["S0"] >= M]
    out = []
    for d in targets:
        rid = d["id"]
        fr, root = finals[rid], roots[rid]
        prefix = games[root["game_seed"]][: root["ply"]]
        children = {c["mv"]: c for c in fr["children"]}
        best = max(children, key=lambda mv: children[mv]["searches"][1]["score"]["value"])
        s0 = fr["proposals"][0]["best_move"]
        d0 = fr["proposals"][0]["depth"]
        e = Engine()
        board = e.board(prefix)
        e.close()
        if board.split()[0] != root["extended_sfen"].split()[0]:
            raise SystemExit(f"{rid}: board mismatch")
        black = root["extended_sfen"].split()[1] == "b"
        entry = {"id": rid, "sfen": root["extended_sfen"], "moves_prefix_len": len(prefix),
                 "loss_S0": d["loss"]["S0"], "d0": d0, "best": best, "s0": s0, "lines": {}}
        for mv in dict.fromkeys([best, s0]):
            e = Engine()
            score, depth, pv, bm = e.search(prefix + [mv], NODES)
            e.close()
            recorded = children[mv]["searches"][1]["score"]
            root_view = -score[1] if score[0] == "cp" else None
            e = Engine()
            mats = [material(e.board(prefix).split()[0], black)]
            seq = [mv] + pv
            for k in range(1, len(seq) + 1):
                mats.append(material(e.board(prefix + seq[:k]).split()[0], black))
            boards = [e.board(prefix + seq[:k]) for k in (1, min(7, len(seq)), len(seq))]
            e.close()
            entry["lines"][mv] = {"score_root_view": root_view, "recorded": recorded, "depth": depth,
                                  "pv": pv, "material_by_ply": mats, "boards_after_1_7_end": boards}
        out.append(entry)
    json.dump(out, sys.stdout, indent=1, ensure_ascii=False)


main()
