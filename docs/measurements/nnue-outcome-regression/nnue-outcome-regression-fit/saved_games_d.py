#!/usr/bin/env python3
"""教師より新しいエンジンの対局で、基点のネットと補正ネットの予測の損失の差Dを求める。

保存済みのペアの棋譜を再生し、1局あたり最大5局面の静的評価を2つのバイナリのUSIの`eval`で求める。
局面の取り方は docs/measurements/adhoc/nnue-placement-outcome-regression/check2.py と同じである。
ペア番号の偶数側で手数帯ごとの勝率尺度K_tと手番の切片c_uを推定し直し、奇数側でDを求める。

使い方:
  saved_games_d.py --data <repoのdata> --base-bin <path> --corrected-bin <path> --runs <実行ディレクトリ名>...
"""
import argparse
import glob
import json
import os
import re
import subprocess
import tempfile

import numpy as np

START = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL"
SQ = re.compile(r"(\d{1,2})([a-l])")
RANKS = "abcdefghijkl"
SNAPSHOTS = [(60, 81, 100, 121, 140), (61, 80, 101, 120, 141)]
SCORE_LIMIT = 3000
RESAMPLES = 1000
BOOTSTRAP_SEED = 1
K_RANGE = (50.0, 4000.0)


def parse_board(text):
    board = {}
    for rank, row in enumerate(text.split("/")):
        file = 12
        index = 0
        while index < len(row):
            char = row[index]
            if char.isdigit():
                end = index
                while end < len(row) and row[end].isdigit():
                    end += 1
                file -= int(row[index:end])
                index = end
                continue
            board[(file, rank)] = (char.lower(), 0 if char.isupper() else 1, False)
            file -= 1
            index += 1
    return board


def emit_board(board):
    rows = []
    for rank in range(12):
        text = ""
        empty = 0
        for file in range(12, 0, -1):
            piece = board.get((file, rank))
            if piece is None:
                empty += 1
                continue
            if empty:
                text += str(empty)
                empty = 0
            kind, side, promoted = piece
            text += ("+" if promoted else "") + (kind.upper() if side == 0 else kind)
        if empty:
            text += str(empty)
        rows.append(text)
    return "/".join(rows)


def extract(data, runs):
    rows = []
    for run in runs:
        paths = sorted(glob.glob(os.path.join(data, "matches", run, "pairs", "*.json")))
        if not paths:
            raise SystemExit(f"{run}: ペアの記録がない")
        for path in paths:
            with open(path, encoding="utf-8") as handle:
                pair = json.load(handle)
            for game_index, game in enumerate(pair["games"]):
                termination = game["termination"]
                if termination["kind"] == "cutoff":
                    continue
                winner = termination.get("winner")
                draw = termination["kind"] == "adjudicated_draw"
                if not draw and winner not in ("black", "white"):
                    raise SystemExit(f"{path}: 結果を読めない {termination}")
                snapshot = SNAPSHOTS[(pair["pair_number"] + game_index) % 2]
                board = parse_board(START)
                sequence = [(move, None) for move in pair["opening"]["moves"]]
                sequence += [(turn["response"].get("usi"), turn) for turn in game["turns"] if turn["response"].get("kind") == "move"]
                for ply, (move, turn) in enumerate(sequence):
                    if ply in snapshot and turn is not None:
                        side = 0 if turn["side"] == "black" else 1
                        score = (turn.get("evaluation") or {}).get("score") or {}
                        if score.get("kind") == "cp" and abs(score["value"]) < SCORE_LIMIT:
                            result = 0.5 if draw else (1.0 if (winner == "black") == (side == 0) else 0.0)
                            rows.append(dict(
                                pair=(run, pair["pair_number"]),
                                game=(run, pair["pair_number"], game_index),
                                ply=ply,
                                side=side,
                                result=result,
                                sfen=f"{emit_board(board)} {'b' if side == 0 else 'w'} - 1",
                            ))
                    if ply >= 141:
                        break
                    squares = [(int(file), RANKS.index(rank)) for file, rank in SQ.findall(move)]
                    source, target = squares[0], squares[-1]
                    piece = board.pop(source)
                    for middle in squares[1:-1]:
                        if middle != source:
                            board.pop(middle, None)
                    board.pop(target, None)
                    if move.endswith("+"):
                        piece = (piece[0], piece[1], True)
                    board[target] = piece
    return rows


def usi_eval(binary, sfens):
    with tempfile.TemporaryFile("w+", encoding="utf-8") as commands:
        commands.write("usi\nisready\nusinewgame\n")
        for sfen in sfens:
            commands.write(f"position sfen {sfen}\neval\n")
        commands.write("quit\n")
        commands.seek(0)
        output = subprocess.run([binary, "--protocol", "usi", "--rules", "engine-default"], stdin=commands, stdout=subprocess.PIPE, text=True, check=True).stdout
    values = [int(line.split()[-1]) for line in output.splitlines() if line.startswith("info string evaluation ")]
    if len(values) != len(sfens):
        raise SystemExit(f"{binary}: 評価値の数が一致しない {len(values)} != {len(sfens)}")
    return np.array(values, dtype=float)


def fit_calibration(evaluation, band, side, result, weight):
    """β_tとc_uを重みつきのロジスティック回帰で推定する。"""
    design = np.zeros((len(result), 7))
    design[np.arange(len(result)), band] = evaluation / 1000
    design[np.arange(len(result)), 5 + side] = 1
    theta = np.zeros(7)
    for _ in range(100):
        probability = 1 / (1 + np.exp(-design @ theta))
        gradient = design.T @ (weight * (result - probability))
        hessian = (design * (weight * probability * (1 - probability))[:, None]).T @ design
        step = np.linalg.solve(hessian, gradient)
        theta += step
        if np.abs(step).max() < 1e-6:
            break
    else:
        raise SystemExit("K_tとc_uの推定が収束しない")
    scales = 1000 / theta[:5]
    if np.any(scales < K_RANGE[0]) or np.any(scales > K_RANGE[1]):
        raise SystemExit(f"K_tが範囲の外にある: {scales}")
    return scales, theta[5:]


def losses(evaluation, band, side, result, scales, intercepts):
    logit = evaluation / scales[band] + intercepts[side]
    return np.logaddexp(0, -logit) * result + np.logaddexp(0, logit) * (1 - result)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--data", required=True)
    parser.add_argument("--base-bin", required=True)
    parser.add_argument("--corrected-bin", required=True)
    parser.add_argument("--runs", required=True, nargs="+")
    args = parser.parse_args()

    rows = extract(args.data, args.runs)
    sfens = [row["sfen"] for row in rows]
    base = usi_eval(args.base_bin, sfens)
    corrected = usi_eval(args.corrected_bin, sfens)
    keep = np.abs(base) < SCORE_LIMIT
    rows = [row for row, flag in zip(rows, keep) if flag]
    base, corrected = base[keep], corrected[keep]

    band = np.array([(row["ply"] - 60) // 20 for row in rows])
    side = np.array([row["side"] for row in rows])
    result = np.array([row["result"] for row in rows])
    _, game = np.unique([f"{row['game']}" for row in rows], return_inverse=True)
    weight = 1 / np.bincount(game)[game]
    odd = np.array([row["pair"][1] % 2 == 1 for row in rows])
    print(f"局面 {len(rows)}  対局 {game.max() + 1}  評価値が変わった局面の割合 {(base != corrected).mean():.3f}  平均変化の絶対値 {np.abs(base - corrected).mean():.1f} cp")

    scales, intercepts = fit_calibration(base[~odd], band[~odd], side[~odd], result[~odd], weight[~odd])
    print("K_t", np.round(scales, 1), "c_u（先手番、後手番）", np.round(intercepts, 4))
    base_loss = losses(base[odd], band[odd], side[odd], result[odd], scales, intercepts)
    corrected_loss = losses(corrected[odd], band[odd], side[odd], result[odd], scales, intercepts)
    games = game[odd]
    size = game.max() + 1
    counts = np.bincount(games, minlength=size)
    present = counts > 0
    base_by_game = np.bincount(games, base_loss, minlength=size)[present] / counts[present]
    corrected_by_game = np.bincount(games, corrected_loss, minlength=size)[present] / counts[present]
    difference = corrected_by_game - base_by_game
    generator = np.random.default_rng(BOOTSTRAP_SEED)
    samples = [difference[generator.integers(0, len(difference), len(difference))].mean() for _ in range(RESAMPLES)]
    low, high = np.percentile(samples, [2.5, 97.5])
    print(f"Dを求めた対局 {len(difference)}局  補正なし {base_by_game.mean():.6f}  補正あり {corrected_by_game.mean():.6f}")
    print(f"D = {difference.mean():+.6f}  95%区間 [{low:+.6f}, {high:+.6f}]")
    print("判定: " + ("補正の向きが教師の対局と食い違う（利用者へ報告）" if low > 0 else "食い違いは見えない"))


if __name__ == "__main__":
    main()
