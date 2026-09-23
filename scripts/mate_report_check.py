#!/usr/bin/env python3
"""投了診断で見つかった「詰まされる報告の後に逆転した局」を検証する。

保存記録から、`mated_in`を報告した側が最終的に勝った局を抽出し、報告した局面の
次の局面（記録された着手の後）を攻撃側として反復深化で探索して、詰みを初めて
見つける深さと、実戦で攻撃側が到達した深さを並べる。
結果の読み方は docs/research/usi-resignation-diagnosis.md の「詰み帯の誤投了の内訳」。

    python3 scripts/mate_report_check.py --binary target/release/minase [--max-depth 14]
"""
import argparse
import glob
import json
import os
import subprocess
import time

MATE_SENTINEL = 10**9


def own_score(turn):
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
    raise ValueError(score["kind"])


def winner_of(termination):
    kind = termination["kind"]
    if kind == "adjudicated_win":
        return termination["winner"]
    if kind == "resigned":
        return "white" if termination["loser"] == "black" else "black"
    if kind == "adjudicated_draw":
        return None
    raise KeyError(kind)


def extract_cases(root):
    """`mated_in`を報告した側が勝った局（駒枯れの裁定を除く）を、最初の報告の着手で抽出する。"""
    cases = []
    for run in sorted(os.listdir(root)):
        manifest_path = os.path.join(root, run, "manifest.json")
        if not os.path.exists(manifest_path):
            continue
        manifest = json.load(open(manifest_path))
        for pair_file in sorted(glob.glob(os.path.join(root, run, "pairs", "*.json"))):
            pair = json.load(open(pair_file))
            for game in pair["games"]:
                try:
                    winner = winner_of(game["termination"])
                except KeyError:
                    continue
                turns = game["turns"]
                for index, turn in enumerate(turns):
                    score = own_score(turn)
                    if score is None or score > -MATE_SENTINEL:
                        continue
                    if winner == turn["side"] and game["termination"].get("reason") != "PieceExhaustion":
                        moves = list(pair["opening"]["moves"]) + [t["response"]["usi"] for t in turns[: index + 1]]
                        attacker = turns[index + 1] if index + 1 < len(turns) else None
                        cases.append({
                            "run": run,
                            "pair": pair["pair_number"],
                            "rules": manifest["rules_source"],
                            "defender_depth": turn["evaluation"]["depth"],
                            "mated_in": turn["evaluation"]["score"].get("moves"),
                            "attacker_depth": (attacker or {}).get("evaluation", {}).get("depth") if attacker else None,
                            "attacker_stop": (attacker or {}).get("stop_reason"),
                            "attacker_think_ms": attacker["think_time_ns"] // 10**6 if attacker else None,
                            "moves_after_report": moves,
                        })
                    break
    return cases


class Engine:
    def __init__(self, binary, rules):
        self.process = subprocess.Popen(
            [binary, "--protocol", "usi", "--rules", rules],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1,
        )
        self.send("usi")
        self.send("isready")
        self.wait("readyok")
        self.send("usinewgame")

    def send(self, line):
        self.process.stdin.write(line + "\n")
        self.process.stdin.flush()

    def wait(self, token):
        while True:
            line = self.process.stdout.readline()
            if not line or line.strip() == token:
                return

    def search(self, moves, depth, timeout):
        """固定深さで探索し、最後の`info`行の評価値と経過時間を返す。"""
        self.send("position startpos moves " + " ".join(moves))
        self.send(f"go depth {depth}")
        last = None
        started = time.time()
        stopped = False
        while True:
            line = self.process.stdout.readline()
            if not line:
                return None
            line = line.strip()
            if line.startswith("info depth "):
                last = line
            if line.startswith("bestmove"):
                words = last.split() if last else []
                if "score" not in words:
                    return None
                score_index = words.index("score")
                return (words[score_index + 1], int(words[score_index + 2]), int(words[words.index("time") + 1]))
            if not stopped and time.time() - started > timeout:
                self.send("stop")
                stopped = True

    def quit(self):
        self.send("quit")
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", default="data/matches")
    parser.add_argument("--binary", required=True, help="攻撃側として探索するminaseの実行ファイル")
    parser.add_argument("--max-depth", type=int, default=14)
    parser.add_argument("--timeout", type=float, default=150, help="1回の探索の上限秒数（超過時はstop）")
    args = parser.parse_args()
    cases = extract_cases(args.root)
    print(f"cases={len(cases)}")
    print("run pair | defender depth mated_in | attacker depth stop think_ms | mate first seen at depth (iteration ms)")
    for case in cases:
        engine = Engine(args.binary, case["rules"])
        found = None
        for depth in range(1, args.max_depth + 1):
            result = engine.search(case["moves_after_report"], depth, args.timeout)
            if result is None:
                break
            if result[0] == "mate" and result[1] > 0:
                found = (depth, result[1], result[2])
                break
        engine.quit()
        found_text = f"{found[0]} (mate {found[1]}, {found[2]} ms)" if found else f"none up to {args.max_depth}"
        print(
            f"{case['run']} p{case['pair']} | d{case['defender_depth']} {case['mated_in']} | "
            f"d{case['attacker_depth']} {case['attacker_stop']} {case['attacker_think_ms']} | {found_text}",
            flush=True,
        )


if __name__ == "__main__":
    main()
