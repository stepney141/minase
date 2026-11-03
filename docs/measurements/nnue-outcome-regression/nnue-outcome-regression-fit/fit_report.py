#!/usr/bin/env python3
"""fit-linearのsummary.jsonから、判定の区間と補正の内容を出力する。

使い方: fit_report.py --summary <summary.json> --data <repoのdata>
"""
import argparse
import json
import os

import numpy as np

RESAMPLES = 1000
BOOTSTRAP_SEED = 1
SFEN = dict(FreeKing="q", King="k", Lion="n", Pawn="p", GoBetween="i", Lance="l", ReverseChariot="a", SideMover="m", VerticalMover="v", Bishop="b", Rook="r", DragonHorse="h", DragonKing="d", DrunkElephant="e", FerociousLeopard="f", BlindTiger="t", CopperGeneral="c", SilverGeneral="s", GoldGeneral="g", Kirin="o", Phoenix="x")
NAMES = dict(FreeKing="奔王", King="王将", Lion="獅子", Pawn="歩兵", GoBetween="仲人", Lance="香車", ReverseChariot="反車", SideMover="横行", VerticalMover="竪行", Bishop="角行", Rook="飛車", DragonHorse="龍馬", DragonKing="龍王", DrunkElephant="醉象", FerociousLeopard="猛豹", BlindTiger="盲虎", CopperGeneral="銅将", SilverGeneral="銀将", GoldGeneral="金将", Kirin="麒麟", Phoenix="鳳凰")
PLACEMENTS = (
    ("奔王が1〜2段", "FreeKing", lambda file, rank: rank <= 2),
    ("龍馬が1〜2段", "DragonHorse", lambda file, rank: rank <= 2),
    ("醉象が3〜5筋", "DrunkElephant", lambda file, rank: 3 <= file <= 5),
    ("金将が3段以上", "GoldGeneral", lambda file, rank: rank >= 3),
    ("麒麟が右端2筋", "Kirin", lambda file, rank: file <= 2),
)


def correction(summary, kind, file, rank):
    """持ち主から見た筋と段にある駒1枚の補正（cp、q = 90）を返す。"""
    index = summary["kinds"].index(kind)
    distance = int(abs(file - 6.5) - 0.5)
    return summary["rank_coefficients"][index][min(rank, 7) - 1] + summary["file_coefficients"][index][distance]


def occupancy(counts, kind):
    """現行のネットの20〜99手の配置の分布を、持ち主から見た(筋, 段)の辞書で返す。"""
    merged = np.zeros(144)
    for phase in ("0", "1"):
        merged += np.array(counts["nnue_now"][phase][SFEN[kind]])
    merged /= merged.sum()
    return {(index // 12 + 1, index % 12 + 1): value for index, value in enumerate(merged)}


def weighted_mean(summary, kind, weights, squares):
    total = sum(weights[square] for square in squares)
    if total == 0:
        return float("nan")
    return sum(weights[square] * correction(summary, kind, *square) for square in squares) / total


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary", required=True)
    parser.add_argument("--data", required=True)
    args = parser.parse_args()
    with open(args.summary, encoding="utf-8") as handle:
        summary = json.load(handle)
    with open(os.path.join(args.data, "piece-placement-census", "counts.json"), encoding="utf-8") as handle:
        counts = json.load(handle)["counts"]

    print("対局数", summary["games"], "局面数", summary["positions"])
    print("K_t", [round(value, 1) for value in summary["calibration"]["cp_per_logit"]], "c_u", summary["calibration"]["side_intercept"])
    print("τの選択の対局の補正なしの損失", summary["tau_selection_base_loss"])
    for row in summary["tau_losses"]:
        print(f"  τ={row['tau']}: 損失 {row['loss']:.6f}  差 {row['loss'] - summary['tau_selection_base_loss']:+.6f}")
    print("選んだτ", summary["selected_tau"])

    games = summary["decision"]["games"]
    base = np.array([game["base_loss"] for game in games])
    fitted = np.array([game["corrected_loss"] for game in games])
    difference = fitted - base
    generator = np.random.default_rng(BOOTSTRAP_SEED)
    samples = [difference[generator.integers(0, len(games), len(games))].mean() for _ in range(RESAMPLES)]
    low, high = np.percentile(samples, [2.5, 97.5])
    print(f"判定の対局 {len(games)}局  補正なし {base.mean():.6f}  補正あり {fitted.mean():.6f}")
    print(f"D = {difference.mean():+.6f}  95%区間 [{low:+.6f}, {high:+.6f}]（summary.jsonのd {summary['decision']['d']:+.6f}）")
    print("判定: " + ("先へ進む" if high < 0 else "止める"))

    print("5配置の補正（現行のネットの20〜99手の配置で重みづけた平均、q = 90のcp）")
    every = [(file, rank) for file in range(1, 13) for rank in range(1, 13)]
    for name, kind, condition in PLACEMENTS:
        weights = occupancy(counts, kind)
        inside = [square for square in every if condition(*square)]
        outside = [square for square in every if not condition(*square)]
        inner, outer = weighted_mean(summary, kind, weights, inside), weighted_mean(summary, kind, weights, outside)
        print(f"  {name}: 配置の内 {inner:+.1f}  外 {outer:+.1f}  差 {inner - outer:+.1f}")

    print("補正の大きい駒種と升（現行のネットの配置で1%以上の升、q = 90のcp）")
    rows = []
    for kind in summary["kinds"]:
        weights = occupancy(counts, kind)
        for square in every:
            if weights[square] >= 0.01:
                rows.append((abs(correction(summary, kind, *square)), kind, square, correction(summary, kind, *square), weights[square]))
    for _, kind, (file, rank), value, weight in sorted(rows, reverse=True)[:20]:
        print(f"  {NAMES[kind]} {file}筋{rank}段: {value:+.1f}（配置の{weight:.1%}）")

    print("駒種ごとの補正の散らばり（現行のネットの配置で重みづけた標準偏差、cp）")
    spreads = []
    for kind in summary["kinds"]:
        weights = occupancy(counts, kind)
        mean = weighted_mean(summary, kind, weights, every)
        variance = sum(weights[square] * (correction(summary, kind, *square) - mean) ** 2 for square in every)
        spreads.append((variance**0.5, kind, mean))
    for spread, kind, mean in sorted(spreads, reverse=True):
        print(f"  {NAMES[kind]}: 標準偏差 {spread:.1f}  平均 {mean:+.1f}")


if __name__ == "__main__":
    main()
