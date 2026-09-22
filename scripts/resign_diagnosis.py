#!/usr/bin/env python3
"""保存済み対局記録へ仮想的な投了条件を当て、誤投了で放棄する得点と削減時間を集計する。

`table` は閾値ごとの発火数、誤投了数、放棄する得点の割合、および削減時間の割合を、
全記録と現行構成群（両エンジンが境界日以降のコミットで、時間制御、engine-default）
について出力する。`detail` は指定閾値の誤投了局を1局ずつ列挙する。
設計は docs/plans/usi-resignation.md、結果は docs/research/usi-resignation-diagnosis.md。
"""
import argparse
import collections
import glob
import json
import os
import subprocess

# "mate" は負の詰み帯全体（ResignValue 29,744）に相当する。29,744を超える閾値は診断しない。
THRESHOLDS = [1500, 2000, 2500, 3000, 3500, 4000, 5000, 6000, 8000, "mate"]
MATE_SENTINEL = 10**9


def commit_date(commit_hash):
    try:
        return subprocess.check_output(
            ["git", "log", "-1", "--format=%cs", commit_hash],
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()
    except subprocess.CalledProcessError:
        return None


def own_score(turn):
    """着手した側の視点の評価値。深さ0、欠測、上下界は判定に使わないのでNone。"""
    evaluation = turn.get("evaluation")
    if evaluation is None or evaluation.get("depth") is None or evaluation["depth"] < 1:
        return None
    if evaluation.get("bound") != "exact":
        return None
    score = evaluation["score"]
    sign = 1 if evaluation["perspective"] == turn["side"] else -1
    if score["kind"] == "cp":
        return sign * score["value"]
    if score["kind"] == "mate_in":
        return sign * MATE_SENTINEL
    if score["kind"] == "mated_in":
        return -sign * MATE_SENTINEL
    raise ValueError(f"unknown score kind {score['kind']}")


def fires(score, threshold):
    if score is None:
        return False
    if threshold == "mate":
        return score <= -MATE_SENTINEL
    return score <= -threshold


def winner_of(termination):
    """通常終局の勝者。引き分けはNone。時間切れ、異常、打ち切りはKeyErrorで除外する。"""
    kind = termination["kind"]
    if kind == "adjudicated_win":
        return termination["winner"]
    if kind == "resigned":
        return "white" if termination["loser"] == "black" else "black"
    if kind == "adjudicated_draw":
        return None
    raise KeyError(kind)


def is_current(manifest, boundary):
    if manifest.get("rules_source") != "engine-default":
        return False
    for engine in (manifest["candidate"], manifest["baseline"]):
        identity = engine["identity"]
        if identity["kind"] != "commit" or engine["limit"]["kind"] != "time":
            return False
        date = commit_date(identity["hash"])
        if date is None or date < boundary:
            return False
    return True


def iter_games(root):
    for run in sorted(os.listdir(root)):
        manifest_path = os.path.join(root, run, "manifest.json")
        pair_files = sorted(glob.glob(os.path.join(root, run, "pairs", "*.json")))
        if not os.path.exists(manifest_path) or not pair_files:
            continue
        manifest = json.load(open(manifest_path))
        for pair_file in pair_files:
            pair = json.load(open(pair_file))
            for index, game in enumerate(pair["games"]):
                yield run, manifest, pair["pair_number"], index, game


def first_firing(turns, threshold):
    for index, turn in enumerate(turns):
        if fires(own_score(turn), threshold):
            return index, turn["side"]
    return None


def table(args):
    stats = {"all": collections.defaultdict(collections.Counter), "current": collections.defaultdict(collections.Counter)}
    meta = {"all": collections.Counter(), "current": collections.Counter()}
    current_cache = {}
    pair_fired = collections.defaultdict(set)
    for run, manifest, pair_number, _, game in iter_games(args.root):
        if run not in current_cache:
            current_cache[run] = is_current(manifest, args.boundary)
            for group in ("all",) + (("current",) if current_cache[run] else ()):
                meta[group]["runs"] += 1
        groups = ("all",) + (("current",) if current_cache[run] else ())
        for group in groups:
            meta[group]["games"] += 1
        try:
            winner = winner_of(game["termination"])
        except KeyError as excluded:
            for group in groups:
                meta[group][f"excluded_{excluded.args[0]}"] += 1
            continue
        turns = game["turns"]
        total_time = sum(turn["think_time_ns"] for turn in turns)
        for threshold in THRESHOLDS:
            for group in groups:
                stat = stats[group][threshold]
                stat["games"] += 1
                stat["time_total"] += total_time
            firing = first_firing(turns, threshold)
            if firing is None:
                continue
            index, side = firing
            pair_fired[(run, pair_number, threshold)].update(groups)
            for group in groups:
                stat = stats[group][threshold]
                stat["fired"] += 1
                stat["time_saved"] += sum(turn["think_time_ns"] for turn in turns[index + 1 :])
                if winner is None:
                    stat["draw"] += 1
                elif winner == side:
                    stat["win"] += 1
    for (_, _, threshold), groups in pair_fired.items():
        for group in groups:
            stats[group][threshold]["fired_pairs"] += 1
    for group in ("all", "current"):
        print(f"== {group} {dict(meta[group])}")
        print("T      games  fired fired_pairs   win draw  loss%  saved%")
        for threshold in THRESHOLDS:
            stat = stats[group][threshold]
            games = stat["games"]
            if games == 0:
                continue
            loss = (stat["win"] + 0.5 * stat["draw"]) / games
            print(
                f"{str(threshold):>5} {games:7d} {stat['fired']:6d} {stat['fired_pairs']:6d} "
                f"{stat['win']:5d} {stat['draw']:4d} {100 * loss:6.3f} "
                f"{100 * stat['time_saved'] / stat['time_total']:5.1f}"
            )


def format_score(score):
    if score is None:
        return "?"
    if score >= MATE_SENTINEL:
        return "M+"
    if score <= -MATE_SENTINEL:
        return "M-"
    return str(score)


def detail(args):
    threshold = "mate" if args.threshold == "mate" else int(args.threshold)
    count = 0
    for run, _, pair_number, index, game in iter_games(args.root):
        try:
            winner = winner_of(game["termination"])
        except KeyError:
            continue
        turns = game["turns"]
        firing = first_firing(turns, threshold)
        if firing is None:
            continue
        fired_at, side = firing
        if winner is not None and winner != side:
            continue
        count += 1
        engine = "candidate" if game["candidate_color"] == side else "baseline"
        termination = game["termination"]
        print(
            f"{run} pair={pair_number} game={index} side={side}({engine}) fired_at={fired_at} "
            f"plies={len(turns)} termination={termination['kind']}:{termination.get('reason', '')} winner={winner}"
        )
        mine = [format_score(own_score(turn)) for turn in turns[fired_at:] if turn["side"] == side]
        opponent = [format_score(own_score(turn)) for turn in turns[fired_at:] if turn["side"] != side]
        print("   fired side:", " ".join(mine[: args.width]))
        print("   opponent  :", " ".join(opponent[: args.width]))
    print(f"mismatches: {count}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", default="data/matches", help="実行ディレクトリの親（既定 data/matches）")
    subparsers = parser.add_subparsers(dest="command", required=True)
    table_parser = subparsers.add_parser("table", help="閾値ごとの費用と削減時間の表")
    table_parser.add_argument("--boundary", required=True, help="現行構成群の境界日（YYYY-MM-DD、両コミットがこの日以降）")
    table_parser.set_defaults(func=table)
    detail_parser = subparsers.add_parser("detail", help="指定閾値の誤投了局の列挙")
    detail_parser.add_argument("threshold", help="cpの整数または mate")
    detail_parser.add_argument("--width", type=int, default=40, help="列挙する評価値の個数")
    detail_parser.set_defaults(func=detail)
    args = parser.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
