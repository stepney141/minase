#!/usr/bin/env python3
"""配置の距離Mと、その差の区間を保存済みの対局から求める。

集計の定義は docs/measurements/adhoc/piece-placement-census.md に従う。
持ち主から見た筋と段で、手数帯の中の4手ごとの局面の、成っていない駒の升を数える。
基準の分布は census.py の counts.json から読み、学習には使わない。

使い方:
  placement_distance.py check   --data <repoのdata>
  placement_distance.py power   --data <repoのdata> --run <実行ディレクトリ名>
  placement_distance.py compare --data <repoのdata> --reference <実行ディレクトリ名> --placement <実行ディレクトリ名>
"""
import argparse
import glob
import json
import os
import re

import numpy as np

START = "lfcsgekgscfl/a1b1txot1b1a/mvrhdqndhrvm/pppppppppppp/3i4i3/12/12/3I4I3/PPPPPPPPPPPP/MVRHDNQDHRVM/A1B1TOXT1B1A/LFCSGKEGSCFL"
SQ = re.compile(r"(\d{1,2})([a-l])")
RANKS = "abcdefghijkl"
KINDS = "lfcsgekabtxomvrhdqnpi"
KIND_INDEX = {kind: index for index, kind in enumerate(KINDS)}
NAMES = dict(l="香車", f="猛豹", c="銅将", s="銀将", g="金将", e="醉象", k="王将", a="反車", b="角行", t="盲虎", x="鳳凰", o="麒麟", m="横行", v="竪行", r="飛車", h="龍馬", d="龍王", q="奔王", n="獅子", p="歩兵", i="仲人")
PHASES = [(20, 60), (60, 100), (100, 160)]
STEP = 4
REFERENCES = ("human2000", "hachu", "c0032")
RESAMPLES = 1000
BOOTSTRAP_SEED = 1
# 配置の名前、駒、および持ち主から見た筋と段の条件。
PLACEMENTS = (
    ("奔王が1〜2段", "q", lambda file, rank: rank <= 2),
    ("龍馬が1〜2段", "h", lambda file, rank: rank <= 2),
    ("醉象が3〜5筋", "e", lambda file, rank: 3 <= file <= 5),
    ("金将が3段以上", "g", lambda file, rank: rank >= 3),
    ("麒麟が右端2筋", "o", lambda file, rank: file <= 2),
)
# 検出したい減少0.03に検出力80%を与える区間の半幅。
TARGET_HALF_WIDTH = 0.021
POWER_PAIRS = (200, 400, 800)


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


def owner_square(side, file, rank):
    """持ち主から見た筋（1が右端）と段（1が自陣の最下段）を返す。"""
    return (file, 12 - rank) if side == 0 else (13 - file, rank + 1)


def histogram_index(phase, kind, file, rank):
    return phase, KIND_INDEX[kind], (file - 1) * 12 + (rank - 1)


def count_game(moves, color):
    """1局のcolor側の駒の升を数え、(手数帯, 駒種, 升)の配列で返す。"""
    histogram = np.zeros((len(PHASES), len(KINDS), 144), dtype=np.int32)
    board = parse_board(START)
    for ply, move in enumerate(moves, start=1):
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
        if ply % STEP:
            continue
        phase = next((i for i, (low, high) in enumerate(PHASES) if low <= ply < high), None)
        if phase is None:
            if ply >= PHASES[-1][1]:
                break
            continue
        for (file, rank), (kind, side, promoted) in board.items():
            if side != color or promoted:
                continue
            histogram[histogram_index(phase, kind, *owner_square(side, file, rank))] += 1
    return histogram


def load_run(data, run):
    """対局ごとの候補側と基準側の配列を、(ペア数, 手数帯, 駒種, 升)の形で返す。"""
    paths = sorted(glob.glob(os.path.join(data, "matches", run, "pairs", "*.json")))
    if not paths:
        raise SystemExit(f"{run}: ペアの記録がない")
    pair_numbers, opening_seeds = [], []
    candidate = np.zeros((len(paths), len(PHASES), len(KINDS), 144), dtype=np.int32)
    baseline = np.zeros_like(candidate)
    for row, path in enumerate(paths):
        with open(path, encoding="utf-8") as handle:
            pair = json.load(handle)
        pair_numbers.append(pair["pair_number"])
        opening_seeds.append(pair["opening"]["seed"])
        opening = pair["opening"]["moves"]
        for game in pair["games"]:
            moves = opening + [turn["response"]["usi"] for turn in game["turns"] if turn["response"]["kind"] == "move"]
            color = 0 if game["candidate_color"] == "black" else 1
            candidate[row] += count_game(moves, color)
            baseline[row] += count_game(moves, 1 - color)
    return np.array(pair_numbers), np.array(opening_seeds, dtype=np.uint64), candidate, baseline


def load_counts(data):
    with open(os.path.join(data, "piece-placement-census", "counts.json"), encoding="utf-8") as handle:
        return json.load(handle)["counts"]


def group_histogram(counts, group):
    histogram = np.zeros((len(PHASES), len(KINDS), 144), dtype=np.int64)
    for phase in range(len(PHASES)):
        for kind in KINDS:
            values = counts[group].get(str(phase), {}).get(kind)
            if values:
                histogram[phase, KIND_INDEX[kind]] = values
    return histogram


def normalized(histogram):
    totals = histogram.sum(axis=-1, keepdims=True)
    with np.errstate(divide="ignore", invalid="ignore"):
        return histogram / totals, totals[..., 0] > 0


class Distance:
    """基準の分布を固定し、配列からMと駒種ごとの距離を求める。"""

    def __init__(self, counts, references=REFERENCES):
        pairs = [normalized(group_histogram(counts, group)) for group in references]
        self.references = np.stack([p for p, _ in pairs])
        self.reference_present = np.stack([m for _, m in pairs])

    def tv(self, histogram):
        distribution, present = normalized(histogram)
        distances = 0.5 * np.abs(distribution[None] - self.references).sum(axis=-1)
        return np.where(present[None] & self.reference_present, distances, np.nan)

    def m(self, histogram):
        return float(np.nanmean(self.tv(histogram)))


def placement_rates(histogram):
    """20〜99手の2つの手数帯を合わせた5配置の割合を返す。"""
    merged = histogram[:2].sum(axis=0)
    rates = {}
    for name, kind, condition in PLACEMENTS:
        counts = merged[KIND_INDEX[kind]]
        selected = sum(counts[(file - 1) * 12 + (rank - 1)] for file in range(1, 13) for rank in range(1, 13) if condition(file, rank))
        rates[name] = float(selected / counts.sum())
    return rates


def bootstrap_difference(distance, first, second, seed=BOOTSTRAP_SEED):
    """同じ添字のペアをまとめて復元抽出し、M(second)−M(first)の分布を返す。"""
    if len(first) != len(second):
        raise SystemExit("ペア数が一致しない")
    generator = np.random.default_rng(seed)
    count = len(first)
    differences = np.empty(RESAMPLES)
    for index in range(RESAMPLES):
        weights = np.bincount(generator.integers(0, count, count), minlength=count)
        selected = weights > 0
        chosen = weights[selected].reshape(-1, 1, 1, 1)
        differences[index] = distance.m((second[selected] * chosen).sum(axis=0)) - distance.m((first[selected] * chosen).sum(axis=0))
    return differences


def interval(differences):
    low, high = np.percentile(differences, [2.5, 97.5])
    return float(low), float(high), float(high - low) / 2


def command_check(args):
    counts = load_counts(args.data)
    distance = Distance(counts)
    for group, expected in (("nnue_now", 0.461), ("pst_fm", 0.432), ("pst_gen1", 0.330)):
        print(f"M({group}) = {distance.m(group_histogram(counts, group)):.3f}（設計書の値 {expected:.3f}）")
    _, _, _, baseline = load_run(args.data, "nnue-material-linear-init-noninferiority-ltc")
    replayed = baseline.sum(axis=0)
    recorded = group_histogram(counts, "nnue_now")
    print(f"再生の配列と集計の配列の差の絶対値の和: {int(np.abs(replayed - recorded).sum())}（総数 {int(recorded.sum())}）")
    for name, rate in placement_rates(replayed).items():
        print(f"{name}: {rate:.1%}")


def command_power(args):
    """同じ実行の互いに素なペアを2組に分け、同じ候補どうしの差の半幅を見積もる。"""
    distance = Distance(load_counts(args.data))
    _, _, candidate, baseline = load_run(args.data, args.run)
    print(f"{args.run}: {len(candidate)}ペア")
    print("同じネットの互いに素なペアの比較。開始局面が揃わないので、半幅は保守的な見積もりである。")
    print("区間の上限が0未満になる回数は、差が0のとき2.5%程度の確率で生じるので、回数が0でも誤判定がないことの証明にはならない。")
    chosen = None
    for pairs in POWER_PAIRS:
        splits = len(candidate) // (2 * pairs)
        if splits == 0:
            raise SystemExit(f"{pairs}ペアの比較には{2 * pairs}ペアが要る")
        half_widths, false_alarms = [], 0
        for split in range(splits):
            first = candidate[2 * split * pairs:(2 * split + 1) * pairs]
            second = candidate[(2 * split + 1) * pairs:(2 * split + 2) * pairs]
            differences = bootstrap_difference(distance, first, second)
            low, high, half = interval(differences)
            half_widths.append(half)
            false_alarms += high < 0
            print(f"  {pairs}ペア 分割{split}: 差 {distance.m(second.sum(axis=0)) - distance.m(first.sum(axis=0)):+.4f} 区間 [{low:+.4f}, {high:+.4f}] 半幅 {half:.4f}")
        mean_half = float(np.mean(half_widths))
        print(f"{pairs}ペア: 半幅の平均 {mean_half:.4f}、上限が0未満になった回数 {false_alarms}/{splits}")
        if chosen is None and mean_half <= TARGET_HALF_WIDTH:
            chosen = pairs
    print(f"半幅{TARGET_HALF_WIDTH}以下を満たす最小のペア数: {chosen}")
    # 同じ対局の候補側と基準側は、線形経路の初期値だけを変えて学び直した2つのネットの比較である。
    # 2つのネットの配置が等しい根拠はないので、この比較は差が0の対照ではない。
    for pairs in POWER_PAIRS:
        differences = bootstrap_difference(distance, baseline[:pairs], candidate[:pairs])
        low, high, half = interval(differences)
        verdict = "上限が0未満" if high < 0 else "区間が0をまたぐ"
        print(f"学び直した2つのネット {pairs}ペア: 差 {distance.m(candidate[:pairs].sum(axis=0)) - distance.m(baseline[:pairs].sum(axis=0)):+.4f} 区間 [{low:+.4f}, {high:+.4f}] 半幅 {half:.4f} {verdict}")


def command_compare(args):
    distance = Distance(load_counts(args.data))
    reference_numbers, reference_openings, reference, _ = load_run(args.data, args.reference)
    placement_numbers, placement_openings, placement, _ = load_run(args.data, args.placement)
    if not np.array_equal(reference_numbers, placement_numbers):
        raise SystemExit("2回の対局のペア番号が一致しない")
    if not np.array_equal(reference_openings, placement_openings):
        raise SystemExit("2回の対局の開始局面のシードが一致しない")
    first, second = reference.sum(axis=0), placement.sum(axis=0)
    differences = bootstrap_difference(distance, reference, placement)
    low, high, half = interval(differences)
    print(f"ペア数 {len(reference)}")
    print(f"M(基準の対局) = {distance.m(first):.4f}")
    print(f"M(補正ネット) = {distance.m(second):.4f}")
    print(f"Mの差 = {distance.m(second) - distance.m(first):+.4f}、95%区間 [{low:+.4f}, {high:+.4f}]、半幅 {half:.4f}")
    print("判定: " + ("偏りが減った" if high < 0 else "偏りが減ったとは言えない"))
    print("5配置の割合（20〜99手）")
    base_rates, fitted_rates = placement_rates(first), placement_rates(second)
    for name in base_rates:
        print(f"  {name}: 基準 {base_rates[name]:.1%}  補正 {fitted_rates[name]:.1%}")
    print("駒種ごとの総変動距離（手数帯と基準で平均）")
    base_tv, fitted_tv = distance.tv(first), distance.tv(second)
    for index, kind in enumerate(KINDS):
        base_value = np.nanmean(base_tv[:, :, index])
        fitted_value = np.nanmean(fitted_tv[:, :, index])
        print(f"  {NAMES[kind]}: 基準 {base_value:.3f}  補正 {fitted_value:.3f}  差 {fitted_value - base_value:+.3f}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    subparsers = parser.add_subparsers(dest="command", required=True)
    check = subparsers.add_parser("check")
    check.add_argument("--data", required=True)
    check.set_defaults(run=command_check)
    power = subparsers.add_parser("power")
    power.add_argument("--data", required=True)
    power.add_argument("--run", dest="run_name", required=True)
    power.set_defaults(run=lambda args: command_power(argparse.Namespace(data=args.data, run=args.run_name)))
    compare = subparsers.add_parser("compare")
    compare.add_argument("--data", required=True)
    compare.add_argument("--reference", required=True)
    compare.add_argument("--placement", required=True)
    compare.set_defaults(run=command_compare)
    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
