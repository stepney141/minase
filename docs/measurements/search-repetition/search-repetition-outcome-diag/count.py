"""保存済みの対局記録から、反復（RULES.md第31条R1）による終局の件数と直前の評価値を数える。

使い方: python3 count.py <data/matchesのパス> > totals.json

対象は、規則にR1を含み、候補と基準の両方が`commit:`指定の実行ディレクトリである。
"""

import glob
import json
import os
import statistics
import sys

# 敗者の直前の形勢として、自分の最後の20手から最後の非0の評価値を読む。
LOOKBACK_OWN_MOVES = 20


def centipawns(evaluation):
    """評価値をcp相当の整数へ写す。詰みの値は±30000とする。"""
    if evaluation is None:
        return None
    score = evaluation["score"]
    if score["kind"] == "cp":
        return score["value"]
    return 30000 if score["kind"] == "mate_in" else -30000


def own_scores(turns, side):
    return [centipawns(turn["evaluation"]) for turn in turns if turn["side"] == side]


def last_nonzero(scores):
    for value in reversed(scores[-LOOKBACK_OWN_MOVES:]):
        if value not in (None, 0):
            return value
    return None


def summarize(values):
    values = [value for value in values if value is not None]
    if not values:
        return None
    return {
        "count": len(values),
        "median": statistics.median(values),
        "share_at_most_minus_300": sum(value <= -300 for value in values) / len(values),
        "share_at_least_plus_300": sum(value >= 300 for value in values) / len(values),
    }


def scan_run(run_dir):
    manifest = json.load(open(os.path.join(run_dir, "manifest.json")))
    rules = manifest.get("canonical_rules") or []
    kinds = [manifest[engine]["identity"]["kind"] for engine in ("candidate", "baseline")]
    if "R1" not in rules or kinds != ["commit", "commit"]:
        return None

    counts = {
        "games": 0,
        "repetition_draws": 0,
        "repetition_losses": 0,
        "other_draws": 0,
        "cutoffs": 0,
        "loser_is_candidate": 0,
        "loser_last_score_is_zero": 0,
        "loser_made_final_move": 0,
    }
    loser_before, winner_before, drawer_before = [], [], []
    for pair_path in glob.glob(os.path.join(run_dir, "pairs", "*.json")):
        for game in json.load(open(pair_path))["games"]:
            termination = game["termination"]
            turns = game["turns"]
            counts["games"] += 1
            if termination["kind"] == "cutoff":
                counts["cutoffs"] += 1
            elif termination["kind"] == "adjudicated_draw":
                if termination["reason"] == "Repetition":
                    counts["repetition_draws"] += 1
                    for side in ("black", "white"):
                        drawer_before.append(last_nonzero(own_scores(turns, side)))
                else:
                    counts["other_draws"] += 1
            elif termination.get("reason") == "Repetition":
                winner = termination["winner"]
                loser = "white" if winner == "black" else "black"
                loser_scores = own_scores(turns, loser)
                counts["repetition_losses"] += 1
                counts["loser_is_candidate"] += loser == game["candidate_color"]
                counts["loser_last_score_is_zero"] += loser_scores[-1] == 0
                counts["loser_made_final_move"] += turns[-1]["side"] == loser
                loser_before.append(last_nonzero(loser_scores))
                winner_before.append(last_nonzero(own_scores(turns, winner)))
    return counts, loser_before, winner_before, drawer_before


def main():
    matches_dir = sys.argv[1]
    runs = {}
    total = None
    before = ([], [], [])
    for run_dir in sorted(glob.glob(os.path.join(matches_dir, "*", ""))):
        if not os.path.exists(os.path.join(run_dir, "manifest.json")):
            continue
        scanned = scan_run(run_dir)
        if scanned is None:
            continue
        counts, *lists = scanned
        runs[os.path.basename(os.path.dirname(run_dir))] = counts
        total = counts.copy() if total is None else {k: total[k] + v for k, v in counts.items()}
        for target, source in zip(before, lists):
            target.extend(source)
    json.dump(
        {
            "run_count": len(runs),
            "total": total,
            "loser_score_before_repetition_loss": summarize(before[0]),
            "winner_score_before_repetition_loss": summarize(before[1]),
            "score_before_repetition_draw": summarize(before[2]),
            "runs": runs,
        },
        sys.stdout,
        ensure_ascii=False,
        indent=2,
    )


if __name__ == "__main__":
    main()
