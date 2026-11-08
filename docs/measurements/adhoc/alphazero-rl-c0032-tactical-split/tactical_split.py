"""サイクル32の価値の偏りを、深さ3のαβ探索の読み筋に捕獲があるかで分ける。

TSVのN_cpとalpha_beta_valueには、確率に変換する前の評価値を保存する。
N_cpが欠測でもαβの結果は保存し、Nを使う集計からは除く。
空集合の中央値と、分母が0の比はNAと表示する。
"""

import argparse
from contextlib import ExitStack
import csv
from dataclasses import dataclass
from pathlib import Path
import random
import re
import statistics as st
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "alphazero-rl-c0032-value-bias"))

from common import Board, MATERIAL_BINS, load_pair, opening_moves, pair_files, replay, win_probability
from engine import Engine

PLY_BINS = ((40, 60), (60, 80), (80, 120), (120, 180), (180, 300))


def collect(run, who):
    """raw_net_value.pyと同じ順序・内容で局面を集め、履歴と手番を照合する。"""
    positions = []
    for path in pair_files(run):
        pair = load_pair(path)
        opening = opening_moves(pair)
        for game in pair["games"]:
            for ply, history, board, turn in replay(game, opening):
                if (turn["side"] == game["candidate_color"]) != (who == "candidate") or not 40 <= ply < 300:
                    continue
                score = (turn.get("evaluation") or {}).get("score")
                if score is not None and score["kind"] == "cp":
                    assert ("black" if len(history) % 2 == 0 else "white") == turn["side"]
                    positions.append((ply, board.material(turn["side"]), win_probability(score["value"]), list(history)))
    return positions


class AlphaBetaEngine(Engine):
    """既存Engineの送受信と終了処理を使い、αβ探索の最後の評価行を読む。"""

    def __init__(self, binary):
        self.process = subprocess.Popen(
            [binary, "--protocol", "usi", "--rules", "engine-default"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1,
        )
        try:
            self.send("usi")
            self.until("usiok")
            self.send("setoption name Threads value 1")
            self.send("setoption name USI_Hash value 256")
            self.send("isready")
            self.until("readyok")
        except BaseException:
            self.process.kill()
            self.process.wait()
            raise

    def search(self, history):
        self.send("usinewgame")
        self.send("position startpos moves " + " ".join(history))
        self.send("go depth 3")
        infos = [line for line in self.until("bestmove") if line.startswith("info ") and " score " in line]
        if not infos:
            raise RuntimeError("alpha-beta returned no info line with a score")
        info = infos[-1]
        score = re.search(r" score (cp|mate) ([+-]?\d+)(?=\s|$)", info)
        if score is None:
            raise ValueError("invalid alpha-beta score: " + info.rstrip())
        # 文字列のまま保持し、mate -0とmate +0を区別する。
        return score[1], score[2], info.partition(" pv ")[2].split()


def replay_pv(history, pv, mate):
    """読み筋全体を再生し、捕獲の有無と元の手番側の損得を返す。"""
    board = Board()
    mover = "black"
    for move in history:
        board.play(move, mover)
        mover = "white" if mover == "black" else "black"
    side = "black" if len(history) % 2 == 0 else "white"
    after = board.copy()
    tactical = mate
    for move in pv:
        captured = after.play(move, mover)
        if captured:
            tactical = True
        mover = "white" if mover == "black" else "black"
    return "tactical" if tactical else "quiet", after.material(side)


@dataclass
class Row:
    ply: int
    diff: int
    pv_diff: int
    category: str
    mate: bool
    n: float
    q: float


def median(values):
    return st.median(values) if values else None


def number(value):
    return "NA" if value is None else f"{value:.3f}"


def table(rows):
    """raw_net_value.pyの再現確認用の表。10局面以上の区間だけを表示する。"""
    for lo, hi in MATERIAL_BINS:
        values = [v for _, d, v in rows if lo <= d < hi]
        if len(values) >= 10:
            print(f"  material [{lo:4d},{hi:4d}) n={len(values):6d} win probability median {st.median(values):.3f} "
                  f"share >= 0.5 {sum(v >= 0.5 for v in values) / len(values):.3f}")
    for lo, hi in PLY_BINS:
        values = [v for p, d, v in rows if lo <= p < hi and -5 <= d < 5]
        if len(values) >= 10:
            print(f"  plies [{lo:3d},{hi:3d}) material within 5 n={len(values):5d} win probability median {st.median(values):.3f}")


def comparison_table(rows, material_field):
    for lo, hi in MATERIAL_BINS:
        group = [row for row in rows if lo <= getattr(row, material_field) < hi]
        n = median([row.n for row in group])
        q = median([row.q for row in group])
        gap = median([row.n - row.q for row in group])
        print(f"  material [{lo:4d},{hi:4d}) n={len(group):6d} "
              f"N median {number(n)} Q median {number(q)} N-Q median {number(gap)}")


def report_set(name, rows, skipped):
    print(f"== c0032 nodes=1 on {name}: n={len(rows)} overall median {number(median([row.n for row in rows]))}")
    table([(row.ply, row.diff, row.n) for row in rows])
    evaluated = len(rows) + skipped
    print(f"== counts on {name}: evaluated={evaluated}")
    skipped_share = skipped / evaluated if evaluated else None
    valid_share = len(rows) / evaluated if evaluated else None
    print(f"  N missing: n={skipped} share of evaluated {number(skipped_share)}")
    print(f"  N available: n={len(rows)} share of evaluated {number(valid_share)}")
    for label, count in (
        ("quiet", sum(row.category == "quiet" for row in rows)),
        ("tactical", sum(row.category == "tactical" for row in rows)),
        ("mate", sum(row.mate for row in rows)),
    ):
        share = count / len(rows) if rows else None
        print(f"  {label}: n={count} share of N available {number(share)}")
    for category in ("quiet", "tactical"):
        group = [row for row in rows if row.category == category]
        print(f"== {category} on {name}, recorded material")
        comparison_table(group, "diff")
        if category == "tactical":
            print(f"== tactical on {name}, material after PV")
            comparison_table(group, "pv_diff")
    print(f"== quiet on {name}, recorded material within 5 by plies")
    for lo, hi in PLY_BINS:
        values = [row.n for row in rows
                  if row.category == "quiet" and -5 <= row.diff < 5 and lo <= row.ply < hi]
        print(f"  plies [{lo:3d},{hi:3d}) n={len(values):6d} N median {number(median(values))}")
    print(flush=True)


def compare(target_all, target_quiet, base_all, base_quiet):
    all_medians = median(target_all), median(base_all)
    quiet_medians = median(target_quiet), median(base_quiet)
    g_all = None if None in all_medians else all_medians[0] - all_medians[1]
    g_quiet = None if None in quiet_medians else quiet_medians[0] - quiet_medians[1]
    ratio = None if g_all is None or g_all == 0 or g_quiet is None else g_quiet / g_all
    if any(len(group) < 100 for group in (target_all, target_quiet, base_all, base_quiet)):
        verdict = "読めない"
    elif abs(g_all) < 0.1:
        verdict = "偏りが小さく読めない"
    elif ratio >= 2 / 3:
        verdict = "局面の型が主"
    elif ratio <= 1 / 3:
        verdict = "戦術が主"
    else:
        verdict = "混合"
    return g_all, g_quiet, ratio, verdict


def report_verdict(results):
    print("== 判定（互角は記録時の損得が-5以上5未満、Nの欠測は除外）")
    print("  4部分集合のいずれかが100局面未満なら「読めない」。")
    print("  それ以外で |G_all| < 0.1 なら「偏りが小さく読めない」。")
    print("  それ以外は R >= 2/3 なら「局面の型が主」、R <= 1/3 なら「戦術が主」、間は「混合」。")
    balanced = {}
    for name, rows in results.items():
        all_values = [row.n for row in rows if -5 <= row.diff < 5]
        quiet_values = [row.n for row in rows if -5 <= row.diff < 5 and row.category == "quiet"]
        balanced[name] = all_values, quiet_values
        print(f"  {name}: all n={len(all_values)} N median {number(median(all_values))}; "
              f"quiet n={len(quiet_values)} N median {number(median(quiet_values))}")
    baseline = "c0030 vs c0020"
    print(f"  基準は {baseline}。G_allとG_quietは対象から基準を引いた中央値の差、RはG_quiet/G_all。")
    for name, (all_values, quiet_values) in balanced.items():
        if name == baseline:
            continue
        g_all, g_quiet, ratio, verdict = compare(all_values, quiet_values, *balanced[baseline])
        print(f"  {name}: G_all={number(g_all)} G_quiet={number(g_quiet)} R={number(ratio)} 判定={verdict}")


def positive_int(value):
    count = int(value)
    if count <= 0:
        raise argparse.ArgumentTypeError("sample count must be a positive integer")
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", help="minaseのバイナリ")
    parser.add_argument("net", help="サイクル32のONNXネット")
    parser.add_argument("stc", help="サイクル32対αβのSTC実行ディレクトリ")
    parser.add_argument("progress", help="サイクル30対サイクル20の実行ディレクトリ")
    parser.add_argument("alpha_beta", help="αβ同士の実行ディレクトリ")
    parser.add_argument("sample_count", type=positive_int, help="各集合から抽出する局面数")
    parser.add_argument("tsv", type=Path, help="局面ごとの値を書き出すTSV")
    args = parser.parse_args()

    sets = {
        "c0032 vs alpha-beta, MCTS to move": collect(args.stc, "candidate"),
        "c0032 vs alpha-beta, alpha-beta to move": collect(args.stc, "baseline"),
        "alpha-beta vs alpha-beta": collect(args.alpha_beta, "candidate") + collect(args.alpha_beta, "baseline"),
        "c0030 vs c0020": collect(args.progress, "candidate") + collect(args.progress, "baseline"),
    }
    rng = random.Random(20261103)
    samples = {name: rng.sample(positions, min(args.sample_count, len(positions)))
               for name, positions in sets.items()}
    results = {}
    with args.tsv.open("w", newline="", encoding="utf-8") as output, ExitStack() as engines:
        writer = csv.writer(output, delimiter="\t", lineterminator="\n")
        writer.writerow(("set", "ply", "recorded_material", "pv_material", "classification", "mate",
                         "N_cp", "alpha_beta_kind", "alpha_beta_value", "pv"))
        alpha_engine = AlphaBetaEngine(args.binary)
        engines.callback(alpha_engine.quit)
        net_engine = Engine(args.binary, args.net, 16)
        engines.callback(net_engine.quit)
        for name, sample in samples.items():
            rows = []
            skipped = 0
            started = time.monotonic()
            for processed, (ply, diff, _, history) in enumerate(sample, 1):
                cp = net_engine.search(history, "go nodes 1")[0]
                kind, value, pv = alpha_engine.search(history)
                mate = kind == "mate"
                category, pv_diff = replay_pv(history, pv, mate)
                q = (0.0 if value.startswith("-") else 1.0) if mate else win_probability(int(value))
                writer.writerow((name, ply, diff, pv_diff, category, int(mate),
                                 cp, kind, value, " ".join(pv)))
                if cp is None:
                    skipped += 1
                else:
                    rows.append(Row(ply, diff, pv_diff, category, mate, win_probability(cp), q))
                if processed % 500 == 0:
                    print(f"{name}: processed={processed} elapsed={time.monotonic() - started:.1f}s",
                          file=sys.stderr, flush=True)
            results[name] = rows
            report_set(name, rows, skipped)
    report_verdict(results)


if __name__ == "__main__":
    main()
