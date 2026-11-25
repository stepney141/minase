#!/usr/bin/env python3
"""HaChu戦の保存棋譜から、minaseが優勢を得てから勝ち切るまでの速さを版ごとに集計する。

リポジトリのルートディレクトリで実行する。対局記録は data/matches/diag-mating-*-hachu/ にあり、
候補のバイナリは対局ハーネスがビルドした target/match-cache/<コミット>/minase を使う。

  python3 docs/audits/mating-speed-against-hachu-2026-11-25/analyze.py games      > games.json
  python3 docs/audits/mating-speed-against-hachu-2026-11-25/analyze.py mate-probe > mate_probe.json
  python3 docs/audits/mating-speed-against-hachu-2026-11-25/analyze.py eval-regression > eval_regression.json
"""
import glob
import json
import re
import statistics as st
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor

import numpy as np

RULES = "L1,L3,P0,P5,P6,R2,E1,E2"
NET = "/mnt/synology_nas/minase/nnue/nets/415020879b9086772acb168397dfa406a1432350c151f87b635b3f271153d1a9.bin"
CACHE = "target/match-cache"
RUNS = {
    "v2.6.0": "diag-mating-v260-hachu",
    "v3.0.1fix": "diag-mating-v301fix-hachu",
    "v3.1.0": "diag-mating-v310-hachu",
}
COMMITS = {
    "v2.6.0": "bac8e2c9367107d47d8cb3c71ae6757044d0ddc8",
    "v3.0.1fix": "b74f11c45e7ba6cb43062014c4c34508398bf6a7",
    "v3.1.0": "532b6e34c99eed1789ddca4d06eb1f09ecce2b55",
}
# 局面の再生と駒の読み取りには、v3.1.0のバイナリのdコマンドを使う。
REPLAY = "v3.1.0"

# HaChuの駒価値を歩兵100へ換算した値（crates/minase/src/eval/handcrafted.rs）。王駒は0とする。
BASE = dict(P=100, I=125, L=375, A=375, M=500, V=500, B=625, R=750, H=875, D=1000,
            Q=1500, E=503, F=375, T=380, C=250, S=250, G=378, O=385, X=383, N=2500, K=0)
# 成駒の価値。SFENの成駒は「+元の駒の記号」で書かれる。醉象の成駒の太子は王駒なので0とする。
PROMOTED = dict(P=378, I=503, L=875, A=625, M=1000, V=1000, B=875, R=1000, H=1250, D=1375,
                E=0, F=625, T=750, C=500, S=500, G=750, O=2500, X=1500)
THRESHOLDS = (1500, 3000, 5000)
NEAR_RADIUS = 3


def engine(version):
    path = f"{CACHE}/{COMMITS[version]}/minase"
    if version == "v2.6.0":
        return [path, "--protocol", "usi", "--rules", RULES], ""
    return [path, "--protocol", "usi"], f"setoption name RuleSet value {RULES}\nsetoption name EvalFile value {NET}\n"


def position_command(moves):
    return "position startpos" + (" moves " + " ".join(moves) if moves else "") + "\n"


def pieces(board):
    """SFENの盤面から(筋, 段の添字, 元の駒の記号, 先手の駒か, 成駒か)の一覧を作る。"""
    out = []
    for rank, row in enumerate(board.split("/")):
        file, promoted, digits = 12, False, ""
        for ch in row:
            if ch.isdigit():
                digits += ch
                continue
            if digits:
                file -= int(digits)
                digits = ""
            if ch == "+":
                promoted = True
                continue
            out.append((file, rank, ch.upper(), ch.isupper(), promoted))
            file -= 1
            promoted = False
    return out


def is_royal(kind, promoted):
    return kind == "K" or (kind == "E" and promoted)


def material(board):
    """先手から見た盤上の駒価値の差（王駒を除く）。"""
    total = 0
    for _, _, kind, black, promoted in pieces(board):
        value = (PROMOTED if promoted else BASE)[kind]
        total += value if black else -value
    return total


def near_enemy_royal(board, black):
    """blackの側の王駒でない駒のうち、相手の王駒のどれかからチェビシェフ距離3以内にある枚数。"""
    ps = pieces(board)
    royals = [(f, r) for f, r, k, b, p in ps if b != black and is_royal(k, p)]
    if not royals:
        return None
    return sum(1 for f, r, k, b, p in ps
               if b == black and not is_royal(k, p)
               and min(max(abs(f - rf), abs(r - rr)) for rf, rr in royals) <= NEAR_RADIUS)


def boards(moves):
    """初期局面と各着手の後の局面のSFENの盤面。"""
    cmd, setup = engine(REPLAY)
    script = "usi\n" + setup + "isready\n"
    script += "".join(position_command(moves[:n]) + "d\n" for n in range(len(moves) + 1)) + "quit\n"
    out = subprocess.run(cmd, input=script, capture_output=True, text=True, check=True).stdout
    result = [line.split()[1] for line in out.splitlines() if line.startswith("sfen ")]
    if len(result) != len(moves) + 1:
        raise RuntimeError(f"replay produced {len(result)} boards for {len(moves)} moves")
    return result


def load(run):
    for path in sorted(glob.glob(f"data/matches/{run}/pairs/*.json")):
        pair = json.load(open(path))
        for game in pair["games"]:
            moves = list(pair["opening"]["moves"])
            for turn in game["turns"]:
                if turn["response"]["kind"] == "move":
                    moves.append(turn["response"]["usi"])
            yield pair["pair_number"], pair["opening"]["moves"], moves, game["candidate_color"], game["termination"]


def winner_of(termination):
    if "winner" in termination:
        return termination["winner"]
    if "loser" in termination:
        return {"black": "white", "white": "black"}[termination["loser"]]
    return None


def analyse_game(item):
    pair_number, opening, moves, color, termination = item
    bs = boards(moves)
    sign = 1 if color == "black" else -1
    start = len(opening)
    lead = [sign * material(b) - sign * material(bs[start]) for b in bs[start:]]
    near = [near_enemy_royal(b, color == "black") for b in bs[start:]]
    winner = winner_of(termination)
    result = "draw" if winner is None else ("win" if winner == color else "loss")
    game = dict(pair=pair_number, candidate_color=color, result=result, termination=termination["kind"],
                plies=len(moves) - start, final_lead=lead[-1])
    for th in THRESHOLDS:
        first = next((i for i, v in enumerate(lead) if v >= th), None)
        game[f"reach_{th}"] = first
        game[f"plies_after_{th}"] = None if first is None else len(moves) - start - first
        values = [] if first is None else [x for x in near[first:] if x is not None]
        game[f"near_after_{th}"] = st.mean(values) if values else None
    return game


def permutation_median(x, y, rounds=20000, seed=1):
    """中央値の差 median(y) - median(x) と、両側の並べ替え検定のp値。"""
    rng = np.random.default_rng(seed)
    x, y = np.asarray(x, float), np.asarray(y, float)
    observed = np.median(y) - np.median(x)
    pooled = np.concatenate([x, y])
    hits = 0
    for _ in range(rounds):
        rng.shuffle(pooled)
        hits += abs(np.median(pooled[len(x):]) - np.median(pooled[:len(x)])) >= abs(observed)
    return float(observed), hits / rounds


def games():
    per_version = {}
    for version, run in RUNS.items():
        with ThreadPoolExecutor(8) as pool:
            per_version[version] = list(pool.map(analyse_game, list(load(run))))
    summary = {}
    for version, gs in per_version.items():
        s = dict(games=len(gs), wins=sum(g["result"] == "win" for g in gs),
                 losses=sum(g["result"] == "loss" for g in gs),
                 median_plies=st.median(g["plies"] for g in gs),
                 median_final_lead=st.median(g["final_lead"] for g in gs))
        for th in THRESHOLDS:
            tail = [g[f"plies_after_{th}"] for g in gs if g[f"plies_after_{th}"] is not None and g["result"] == "win"]
            near = [g[f"near_after_{th}"] for g in gs if g[f"near_after_{th}"] is not None]
            s[f"reached_{th}"] = sum(g[f"reach_{th}"] is not None for g in gs)
            s[f"median_plies_after_{th}"] = st.median(tail) if tail else None
            s[f"mean_near_after_{th}"] = st.mean(near) if near else None
        summary[version] = s
    tests = {}
    for a, b in (("v2.6.0", "v3.0.1fix"), ("v2.6.0", "v3.1.0"), ("v3.0.1fix", "v3.1.0")):
        t = {}
        for key in [f"plies_after_{th}" for th in (1500, 5000)] + ["final_lead"]:
            xa = [g[key] for g in per_version[a] if g[key] is not None]
            xb = [g[key] for g in per_version[b] if g[key] is not None]
            diff, p = permutation_median(xa, xb)
            t[key] = dict(median_difference=diff, p=p)
        tests[f"{a} -> {b}"] = t
    json.dump(dict(summary=summary, tests=tests, games=per_version), sys.stdout, ensure_ascii=False, indent=1)


def search_score_kind(cmd, setup, moves, nodes):
    script = "usi\n" + setup + "isready\n" + position_command(moves) + f"go nodes {nodes}\n"
    proc = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    proc.stdin.write(script)
    proc.stdin.flush()
    last = None
    for line in proc.stdout:
        if line.startswith("info") and " score " in line:
            last = line
        if line.startswith("bestmove"):
            break
    proc.stdin.write("quit\n")
    proc.stdin.flush()
    proc.wait()
    found = re.search(r"score (cp|mate) ", last or "")
    if not found:
        raise RuntimeError("search printed no score")
    return found.group(1)


def mate_probe(nodes=300000, distances=(1, 3, 5, 7, 9, 11, 13)):
    """minaseが勝った局の終局k手前でminaseの手番の局面を、各版に固定ノード数で探索させる。"""
    probes = []
    for run in RUNS.values():
        for _, _, moves, color, termination in load(run):
            if winner_of(termination) != color:
                continue
            for k in distances:
                prefix = moves[:len(moves) - k]
                if (len(prefix) % 2 == 0) == (color == "black"):
                    probes.append((k, prefix))
    found = {}
    for version in RUNS:
        cmd, setup = engine(version)
        with ThreadPoolExecutor(4) as pool:
            kinds = list(pool.map(lambda p: search_score_kind(cmd, setup, p[1], nodes), probes))
        found[version] = {str(k): sum(kind == "mate" for (kk, _), kind in zip(probes, kinds) if kk == k)
                          for k in distances}
    counts = {str(k): sum(kk == k for kk, _ in probes) for k in distances}
    json.dump(dict(nodes=nodes, positions=counts, mate_scores=found), sys.stdout, indent=1)


def static_evals(version, positions):
    cmd, setup = engine(version)
    script = "usi\n" + setup + "isready\n" + "".join(position_command(m) + "eval\n" for m in positions) + "quit\n"
    out = subprocess.run(cmd, input=script, capture_output=True, text=True, check=True).stdout
    values = [int(line.split()[-1]) for line in out.splitlines() if line.startswith("info string evaluation ")]
    if len(values) != len(positions):
        raise RuntimeError(f"{version} printed {len(values)} evaluations for {len(positions)} positions")
    return values


def eval_regression():
    """minaseの手番の局面の静的評価値を、駒得差と王駒の近くの駒の枚数へ回帰する。"""
    bands = {"lead>=3000": lambda m: m >= 3000, "|lead|<1500": lambda m: abs(m) < 1500}
    rows = {name: [] for name in bands}
    for run in RUNS.values():
        for _, opening, moves, color, _ in load(run):
            black = color == "black"
            for n, board in enumerate(boards(moves)):
                if n < len(opening) or (n % 2 == 0) != black:
                    continue
                lead = (1 if black else -1) * material(board)
                for name, test in bands.items():
                    if test(lead):
                        rows[name].append((moves[:n], lead, near_enemy_royal(board, black) or 0,
                                           near_enemy_royal(board, not black) or 0))
    result = {}
    for name, rs in rows.items():
        x = np.array([[1, r[1], r[2], r[3]] for r in rs], float)
        result[name] = dict(positions=len(rs), mean_own_near=float(x[:, 2].mean()))
        for version, label in (("v2.6.0", "v2.6.0 network"), ("v3.1.0", "generation 3 network")):
            y = np.array(static_evals(version, [r[0] for r in rs]), float)
            coef = np.linalg.lstsq(x, y, rcond=None)[0]
            result[name][label] = dict(intercept=coef[0], per_material=coef[1], per_own_near=coef[2],
                                       per_enemy_near=coef[3], own_near_in_material=coef[2] / coef[1])
    json.dump(result, sys.stdout, indent=1)


{"games": games, "mate-probe": mate_probe, "eval-regression": eval_regression}[sys.argv[1]]()
