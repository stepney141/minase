"""固定した相手との保存記録を集計し、実行ごとのJSON配列を出力する。

使い方: python3 -I <このファイル> data/matches/<測定名> [<別の実行ディレクトリ> ...]
得点の集計は有効ペア、終局と反則と手数の集計は破棄ペアを含む全局を対象とする。
without_move_forfeitsは拒否・不正着手を含むペアを除いた値である。
discarded_as_lossesとdiscarded_as_winsは破棄ペアを2敗・2勝とした値である。
有効ペアがなければEloと得点率はnullにする。
試行ペアが0件なら割合を定義できないため、明示的にエラーにする。
"""

import argparse
from collections import Counter
import json
import math
from pathlib import Path
import statistics
import sys


MOVE_FORFEITS = {"illegal_move", "rejected_move"}
FAILURE_REASONS = ("illegal_move", "crash", "timeout", "time_forfeit", "rejected_move")
SCORES = (0, 0.25, 0.5, 0.75, 1)


def read_json(path):
    with Path(path).open(encoding="utf-8") as source:
        return json.load(source)


def pair_records(run_dir):
    directory = Path(run_dir) / "pairs"
    if not directory.is_dir():
        raise ValueError(f"ペア記録のディレクトリがありません: {directory}")
    for path in sorted(directory.glob("*.json")):
        yield read_json(path)


def engine_side(color, candidate_color):
    if color not in {"black", "white"} or candidate_color not in {"black", "white"}:
        raise ValueError(f"不正な手番の色: {color}, {candidate_color}")
    return "candidate" if color == candidate_color else "baseline"


def game_moves(pair, game):
    return pair["opening"]["moves"] + [
        turn["response"]["usi"] for turn in game["turns"]
        if turn["response"]["kind"] == "move"
    ]


def score_to_elo(score):
    if score == 0:
        return "-inf"
    if score == 1:
        return "+inf"
    return -400 * math.log10(1 / score - 1)


def estimate_elo(counts):
    """stats.rsの母分散による95%信頼区間を計算する。"""
    if len(counts) != 5 or any(type(value) is not int or value < 0 for value in counts):
        raise ValueError("ペンタノミアル度数は非負整数5個で指定してください")
    count = sum(counts)
    if count == 0:
        return None
    mean = sum(frequency * score for frequency, score in zip(counts, SCORES)) / count
    variance = sum(
        frequency * (score - mean) ** 2 for frequency, score in zip(counts, SCORES)
    ) / count
    margin = 1.96 * math.sqrt(variance / count)
    return {
        "elo": score_to_elo(mean),
        "lower": score_to_elo(max(0, mean - margin)),
        "upper": score_to_elo(min(1, mean + margin)),
    }


def score_statistics(counts):
    return {
        "pentanomial": counts,
        "elo": estimate_elo(counts),
        "score_rate": (
            sum(frequency * score for frequency, score in zip(counts, SCORES)) / sum(counts)
            if sum(counts) else None
        ),
    }


def summarize(run_dir):
    run_dir = Path(run_dir)
    manifest = read_json(run_dir / "manifest.json")
    summary = read_json(run_dir / "summary.json")
    candidate = manifest["candidate"]
    args = candidate["identity"]["args"]
    if not any(args[index:index + 2] == ["--search", "mcts"] for index in range(len(args) - 1)):
        raise ValueError(f"候補はMCTSである必要があります: {run_dir}")
    if summary["invocation_active"] is not False:
        raise ValueError(f"実行中の記録は集計できません: {run_dir}")
    if manifest["mode"]["kind"] != "elo":
        raise ValueError(f"固定ペア数のElo測定である必要があります: {run_dir}")
    baseline = manifest["baseline"]
    identity = baseline["identity"]
    if identity["kind"] == "commit":
        kind, limit_name = "alphabeta", "nodes"
    elif identity["kind"] == "command" and identity["protocol"] == "cecp":
        kind, limit_name = "hachu", "depth"
    else:
        raise ValueError(f"基準はコミットのαβ探索かCECPのHaChuである必要があります: {run_dir}")
    limit = baseline["limit"][limit_name]
    if limit is None:
        raise ValueError(f"基準の{limit_name}がありません: {run_dir}")

    counts = [0] * 5
    excluded_counts = [0] * 5
    discarded, move_forfeits, lengths = [], [], []
    terminations = Counter()
    forfeits = {reason: {"candidate": 0, "baseline": 0} for reason in FAILURE_REASONS}
    attempted = 0
    for pair in pair_records(run_dir):
        attempted += 1
        number, category = pair["pair_number"], pair["category"]
        if category is None:
            discarded.append(number)
        elif type(category) is int and 0 <= category <= 4:
            counts[category] += 1
        else:
            raise ValueError(f"ペア{number}のcategoryが不正です: {category}")
        has_move_forfeit = False
        for game in pair["games"]:
            lengths.append(len(game_moves(pair, game)))
            termination = game["termination"]
            termination_kind = termination["kind"]
            reason = None
            if termination_kind == "adjudicated_win":
                winner = engine_side(termination["winner"], game["candidate_color"])
                reason = termination["reason"]
            elif termination_kind in {"forfeit", "resigned"}:
                loser = engine_side(termination["loser"], game["candidate_color"])
                winner = "baseline" if loser == "candidate" else "candidate"
            elif termination_kind in {"adjudicated_draw", "cutoff"}:
                winner = None
                if termination_kind == "adjudicated_draw":
                    reason = termination["reason"]
            else:
                raise ValueError(f"未知の終局理由: {termination_kind}")
            terminations[(termination_kind, reason, winner)] += 1
            if termination_kind == "forfeit":
                failure = termination["reason"]
                if failure not in forfeits:
                    raise ValueError(f"未知の反則理由: {failure}")
                forfeits[failure][loser] += 1
                has_move_forfeit |= failure in MOVE_FORFEITS
        if has_move_forfeit:
            move_forfeits.append(number)
            if category is not None:
                excluded_counts[category] += 1

    if attempted == 0:
        raise ValueError(f"試行ペアが0件のため、割合と手数を集計できません: {run_dir}")
    losses, wins = counts.copy(), counts.copy()
    losses[0] += len(discarded)
    wins[4] += len(discarded)
    affected = sorted(set(discarded) | set(move_forfeits))
    fraction = len(affected) / attempted
    return {
        "run_name": run_dir.name,
        "baseline": {"kind": kind, "limit": {limit_name: limit}},
        "rules_source": manifest["rules_source"],
        "seed": manifest["seed"],
        "max_ply": manifest["max_ply"],
        "nn_model": dict(candidate["options"])["NnModel"],
        "network_sha256": candidate["network_sha256"],
        "attempted_pairs": attempted,
        "valid_pairs": sum(counts),
        "discarded_pairs": len(discarded),
        "discarded_pair_numbers": sorted(discarded),
        "valid": score_statistics(counts),
        "terminations": [
            {"kind": key[0], "reason": key[1], "winner": key[2], "count": value}
            for key, value in sorted(terminations.items())
        ],
        "forfeits": forfeits,
        "move_forfeit_pair_numbers": sorted(move_forfeits),
        "without_move_forfeits": score_statistics([
            total - excluded for total, excluded in zip(counts, excluded_counts)
        ]),
        "discarded_as_losses": score_statistics(losses),
        "discarded_as_wins": score_statistics(wins),
        "affected_pairs": len(affected),
        "affected_pair_fraction": fraction,
        "affected_pair_fraction_exceeds_5_percent": fraction > 0.05,
        "plies": {
            "min": min(lengths),
            "median": statistics.median(lengths),
            "max": max(lengths),
        },
        "active_wall_time_seconds": summary["active_wall_time_ns"] / 1_000_000_000,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run_dirs", nargs="+", type=Path)
    args = parser.parse_args()
    json.dump([summarize(path) for path in args.run_dirs], sys.stdout,
              ensure_ascii=False, indent=2, allow_nan=False)
    print()


if __name__ == "__main__":
    main()
