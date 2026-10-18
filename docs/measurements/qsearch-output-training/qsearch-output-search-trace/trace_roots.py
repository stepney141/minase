"""事前登録した6根を追跡し、探索機構の介入結果を分類する。

判定の正は ../qsearch-output-search-trace.md の「追跡と分類の事前登録」。
標準ライブラリだけを使う。実行条件は固定し、--jobs だけを受け付ける。
実行例: python3 docs/measurements/qsearch-output-search-trace/trace_roots.py

事前登録の解釈:
* 根が経路の初手を探索しない場合、N1 は存在しないため D=0 として
  根の停止理由を記録し、根の再探索で確認する。訪問の欠落は補完しない。
* 「単独の介入はどれも成功せず」は11機構すべてについて判定する。
  追跡外の単独成功があれば、同時成功だけによる分類には進まない。
* 乖離なしには D がないため機構集合を空とする。未確認も空とする。
* 駒の損得や名目深さだけから機構を推定せず、追跡と介入の両方を要する。
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
from decimal import Decimal
import hashlib
import json
from pathlib import Path
import subprocess
import threading


WT = Path(__file__).resolve().parents[3]
BIN = WT / "target/search-trace/release/minase"
SA = Path("/home/stepney141/board-games/minase/data/search-aware-evaluation")
OUT = Path("/home/stepney141/board-games/minase/data/qsearch-output-training/phase0")
REFERENCE = WT / "docs/measurements/relative-pair-prep/residual_pv.json"
M = Decimal("142.1")
NODES = 10_000_000
ROOT_IDS = {
    "202610280-278", "202610489-236", "202612388-210",
    "202612852-212", "202613669-264", "202614203-172",
}
MECHANISMS = (
    "tt_cutoff", "iir", "null_move", "futility", "see", "lmr",
    "qs_delta", "qs_see", "qs_tt", "aspiration", "correction",
)
STOP_MECHANISMS = {
    "tt_cutoff": "tt_cutoff", "null_cutoff": "null_move",
    "futility": "futility", "see": "see", "qs_stand_pat": None,
    "qs_delta_node": "qs_delta", "qs_delta_move": "qs_delta",
    "qs_see": "qs_see", "qs_tt_cutoff": "qs_tt",
}
STOP_REASONS = {
    "sibling_beta_cutoff": "兄弟手による打ち切り",
    "qs_not_generated": "地平線", "max_ply": "地平線",
    "repetition": "反復", "last_royal_capture": "王駒の捕獲",
    "path_end": "読み筋の末端",
}


def score_value(kind, value):
    """USI の詰み距離を内部値に直す。文字列の -0 の符号も保持する。"""
    if kind == "cp":
        return int(value)
    if kind == "mate":
        distance = int(value)
        return (-1 if str(value).startswith("-") else 1) * (30000 - abs(distance))
    raise ValueError(f"未知の評価値の種類: {kind}")


def bound_kind(result, alpha, beta):
    if alpha >= beta:
        raise ValueError(f"不正な窓: ({alpha}, {beta})")
    if result <= alpha:
        return "upper"
    if result >= beta:
        return "lower"
    return "exact"


def diverges(node, teacher):
    """上界・下界・正確な値の3式。m と等しい場合も乖離に含める。"""
    r, t = Decimal(str(node["result"])), Decimal(str(teacher))
    bound = bound_kind(r, Decimal(str(node["alpha"])), Decimal(str(node["beta"])))
    if bound == "upper":
        return t >= r + M
    if bound == "lower":
        return t <= r - M
    return abs(t - r) >= M


def final_pass(events, depth):
    """d0 の最後に完了した窓を選び、不完全な経路の記録は拒否する。"""
    passes = [e for e in events if e["type"] == "pass" and e["iter"] == depth]
    completed = [e for e in passes if not e["aborted"]]
    if not completed:
        raise ValueError(f"深さ {depth} の完了した窓がない")
    selected = completed[-1]
    nodes = [e for e in events if e["type"] == "node"
             and e["iter"] == depth and e["pass"] == selected["pass"]]
    if any(n["aborted"] or n["path_move_outcome"] in ("not_reached", "aborted")
           for n in nodes):
        raise ValueError("最後の窓に未到達または中断した経路がある")
    seqs = [n["seq"] for n in nodes]
    if seqs != sorted(set(seqs)):
        raise ValueError("node の seq が一意な昇順ではない")
    roots = [n for n in nodes if n["ply"] == 0 and n["kind"] == "root"]
    if len(roots) != 1 or any(n["seq"] > roots[0]["seq"] for n in nodes):
        raise ValueError("窓ごとの根の訪問が一意な最後の行ではない")
    for node in nodes:
        if node["bound"] != bound_kind(node["result"], node["alpha"], node["beta"]):
            raise ValueError("node の値、窓と bound が一致しない")
    return selected, nodes, any(p["pass"] > 0 for p in passes)


def last_visits(nodes):
    """親の直前の終了 seq と当該終了 seq の間から最後の子を選ぶ。

    祖先が定めた下限も引き継ぎ、古い親の下の子を再利用しない。
    """
    by_ply = {}
    for node in nodes:
        by_ply.setdefault(node["ply"], []).append(node)
    for visits in by_ply.values():
        visits.sort(key=lambda n: n["seq"])
    chain = [by_ply[0][-1]]
    lower = -1
    while True:
        parent = chain[-1]
        previous = [n["seq"] for n in by_ply[parent["ply"]]
                    if n["seq"] < parent["seq"]]
        if previous:
            lower = max(lower, previous[-1])
        child_ply = parent["ply"] + 1
        if child_ply not in by_ply:
            break
        children = [n for n in by_ply[child_ply] if lower < n["seq"] < parent["seq"]]
        if not children:
            break
        chain.append(children[-1])
    return chain


def involved_mechanisms(chain, aspiration, outcome):
    mechanisms = {"aspiration"} if aspiration else set()
    for node in chain:
        searches = node["path_move_searches"]
        if searches and searches[-1]["reduction"] > 0:
            mechanisms.add("lmr")
        if node["depth_after_iir"] < node["depth_req"]:
            mechanisms.add("iir")
        if node["null"] is not None:
            mechanisms.add("null_move")
    if outcome in STOP_MECHANISMS and STOP_MECHANISMS[outcome] is not None:
        mechanisms.add(STOP_MECHANISMS[outcome])
    return sorted(mechanisms)


def analyze_trace(events, depth, value):
    """最後の訪問に沿って乖離の連鎖を歩き、D と停止理由を返す。"""
    selected, nodes, aspiration = final_pass(events, depth)
    chain = last_visits(nodes)
    root = chain[0]
    if root["path_move_outcome"] == "searched":
        if len(chain) < 2:
            raise ValueError("根の探索に対応する N1 の訪問がない")
        if not diverges(chain[1], -value):
            return {"D": None, "reason": "乖離なし", "outcome": None,
                    "t_D": None, "t_prime_D": None, "node": None,
                    "mechanisms": [], "pass": selected, "visits": chain[:2]}
        index = 1
    else:
        index = 0
    while True:
        node = chain[index]
        outcome = node["path_move_outcome"]
        if outcome in STOP_MECHANISMS:
            reason = "経路上の機構"
            break
        if outcome in STOP_REASONS:
            reason = STOP_REASONS[outcome]
            break
        if outcome != "searched" or not node["path_move_searches"]:
            raise ValueError(f"不正な探索結果: {outcome}")
        if node["path_move_searches"][-1]["reduction"] > 0:
            reason = "減深"
            break
        if index + 1 == len(chain):
            raise ValueError("探索された経路の子訪問がない")
        if not diverges(chain[index + 1], (-1) ** (index + 1) * value):
            reason = "兄弟手の誤り"
            break
        index += 1
    visited = chain[:index + 1]
    return {"D": index, "reason": reason, "outcome": outcome,
            "t_D": (-1) ** index * value, "t_prime_D": None, "node": node,
            "mechanisms": involved_mechanisms(visited, aspiration, outcome),
            "pass": selected, "visits": visited}


def confirm_divergence(analysis, score):
    """N_D の手番側の再探索値で確認し、未確認なら関与機構を取り消す。"""
    if analysis["D"] is None:
        raise ValueError("乖離なしの経路に確認値は不要")
    confirmed = diverges(analysis["node"], score)
    return dict(analysis, t_prime_D=score, confirmed=confirmed,
                original_reason=analysis["reason"],
                reason=analysis["reason"] if confirmed else "乖離点の未確認",
                mechanisms=analysis["mechanisms"] if confirmed else [])


def classify(involved, singles, combined):
    """事前登録の順序で分類し、追跡外の成功を別に記録する。"""
    successful = {name for name, result in singles.items() if result["success"]}
    supported = sorted(set(involved) & successful)
    if len(supported) == 1:
        category = "1機構の特定"
    elif len(supported) >= 2 or (not successful and combined is not None
                                and combined["success"]):
        category = "複数機構の疑い"
    else:
        category = "特定できない"
    return {"category": category, "supported": supported,
            "intervention_only": sorted(successful - set(involved))}


def aggregate(classifications):
    counts = dict.fromkeys(MECHANISMS, 0)
    for classification in classifications:
        if classification["category"] in ("1機構の特定", "複数機構の疑い"):
            for name in classification["supported"]:
                counts[name] += 1
    return {"counts": counts, "three_or_more": [m for m in MECHANISMS if counts[m] >= 3],
            "qsearch_note_required": any(counts[m] >= 1 for m in ("qs_delta", "qs_see", "qs_tt"))}


def parse_search(lines):
    infos = [line.split() for line in lines if line.startswith("info ")
             and " score " in line and " pv " in line]
    if not infos or not lines[-1].startswith("bestmove "):
        raise ValueError("最終 info または bestmove がない")
    last = infos[-1]
    score_index = last.index("score")
    return {"depth": int(last[last.index("depth") + 1]),
            "nodes": int(last[last.index("nodes") + 1]),
            "score": score_value(last[score_index + 1], last[score_index + 2]),
            "score_usi": last[score_index + 1:score_index + 3],
            "pv": last[last.index("pv") + 1:], "bestmove": lines[-1].split()[1]}


def run_search(moves, limit, disabled=(), path=None, trace_out=None):
    """探索ごとに新規 USI プロセスを起動する。ビルドは行わない。"""
    if path is not None:
        trace_out.parent.mkdir(parents=True, exist_ok=True)
        # エンジンは追記するため、同名の前回の探索を混ぜない。
        trace_out.write_text("", encoding="utf-8")
    with subprocess.Popen([str(BIN), "--protocol", "usi", "--rules", "engine-default"],
                          cwd=WT, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          text=True, bufsize=1) as process:
        def send(command):
            process.stdin.write(command + "\n")
            process.stdin.flush()

        def receive(prefix):
            lines = []
            while True:
                line = process.stdout.readline()
                if not line:
                    raise RuntimeError("エンジンが終了した: " + "\n".join(lines[-5:]))
                line = line.rstrip("\n")
                if line.startswith("error") or line.startswith("info string error"):
                    raise RuntimeError(line)
                lines.append(line)
                if line == prefix or line.startswith(prefix + " "):
                    return lines

        try:
            send("usi")
            handshake = receive("usiok")
            for option in ("TracePath", "TraceOut", "TraceDisable"):
                if not any(line.startswith(f"option name {option} ") for line in handshake):
                    raise RuntimeError(f"search-trace のオプションがない: {option}")
            send("setoption name USI_Hash value 64")
            send("setoption name Threads value 1")
            send("isready")
            receive("readyok")
            send("usinewgame")
            send("position startpos moves " + " ".join(moves))
            if path is not None:
                send("setoption name TracePath value " + " ".join(path))
                send("setoption name TraceOut value " + str(trace_out))
            if path is not None or disabled:
                send("setoption name TraceDisable value " + ",".join(disabled))
            send("go " + limit)
            result = parse_search(receive("bestmove"))
            send("quit")
        except BaseException:
            process.kill()
            raise
    if process.returncode != 0:
        raise RuntimeError(f"エンジンの終了コード: {process.returncode}")
    return result


def read_jsonl(path):
    with path.open(encoding="utf-8") as stream:
        return [json.loads(line) for line in stream]


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


class SearchCache:
    """履歴を含む同一局面の10,000,000ノード探索だけを再利用する。"""

    def __init__(self, path, engine_hash):
        self.path = path
        self.lock = threading.Lock()
        self.conditions = {"engine_sha256": engine_hash, "nodes": NODES,
                           "rules": "engine-default", "hash_mib": 64, "threads": 1}
        if path.exists():
            saved = json.loads(path.read_text(encoding="utf-8"))
            if saved["conditions"] != self.conditions:
                raise ValueError("searches.json の探索条件が異なる。別の場所へ退避して再実行すること")
            self.searches = saved["searches"]
        else:
            self.searches = {}

    def search(self, moves, fresh=False):
        key = " ".join(moves)
        with self.lock:
            if not fresh and key in self.searches:
                return self.searches[key]
        result = run_search(moves, f"nodes {NODES}")
        with self.lock:
            self.searches[key] = result
            write_json(self.path, {"conditions": self.conditions, "searches": self.searches})
        return result


def load_roots():
    report = json.loads((SA / "phase4/final/report.json").read_text())["record"]
    targets = [d for d in report["decisions"]
               if d["category"] == "stable" and Decimal(str(d["loss"]["S0"])) >= M]
    if len(targets) != 6 or {d["id"].removesuffix("-r") for d in targets} != ROOT_IDS:
        raise ValueError("選ばれた根が事前登録の6根と一致しない")
    ids = {d["id"] for d in targets}
    finals = {r["record"]["id"]: r["record"]
              for r in read_jsonl(SA / "phase4/final/roots.jsonl")}
    roots = {r["record"]["data"]["id"]: r["record"]["data"]
             for r in read_jsonl(SA / "phase3/roots.jsonl")}
    seeds = {roots[rid]["game_seed"] for rid in ids}
    games = {}
    for game_file in sorted(SA.glob("phase3/games-*.jsonl")):
        for event in read_jsonl(game_file):
            game = event["record"]["data"]
            if game["seed"] in seeds:
                if game["seed"] in games:
                    raise ValueError("対局の seed が重複している")
                # moves は開始手順 opening を先頭に含む。
                if game["moves"][:len(game["opening"])] != game["opening"]:
                    raise ValueError("対局の開始手順が着手列と一致しない")
                games[game["seed"]] = game["moves"]
    references = {r["id"]: r for r in json.loads(REFERENCE.read_text())}
    result = []
    for rid in sorted(ids):
        root, final, reference = roots[rid], finals[rid], references[rid]
        if root["path"]:
            raise ValueError(f"{rid}: 標本の根に追加の経路がある")
        prefix = games[root["game_seed"]][:root["ply"]]
        if len(prefix) != root["ply"] or len(prefix) != reference["moves_prefix_len"]:
            raise ValueError(f"{rid}: 履歴の長さが一致しない")
        values = {c["mv"]: score_value(c["searches"][1]["score"]["kind"],
                                      c["searches"][1]["score"]["value"])
                  for c in final["children"]}
        best = max(values, key=values.__getitem__)
        proposal = final["proposals"][0]
        if (best, proposal["best_move"], proposal["depth"], root["extended_sfen"]) != (
                reference["best"], reference["s0"], reference["d0"], reference["sfen"]):
            raise ValueError(f"{rid}: 標本と参照値の条件が一致しない")
        result.append({"id": rid, "prefix": prefix, "a": best,
                       "s": proposal["best_move"], "d0": proposal["depth"],
                       "proposal": proposal, "reference": reference, "values": values})
    return result


def check_child(root, move, result):
    reference = root["reference"]["lines"][move]
    if -result["score"] != reference["score_root_view"] or result["pv"] != reference["pv"]:
        raise ValueError(f"前提条件A不一致: {root['id']} {move}: {result}")
    value = reference["score_root_view"]
    assert -value == result["score"], "t1 と子局面の値が一致しない"


def check_proposal(root, result):
    proposal = root["proposal"]
    expected = (proposal["best_move"], proposal["depth"],
                score_value(proposal["score"]["kind"], proposal["score"]["value"]))
    if (result["bestmove"], result["depth"], result["score"]) != expected:
        raise ValueError(f"前提条件B不一致: {root['id']}: 期待値={expected}, 実測={result}")


def root_move_value(events, depth):
    _, nodes, _ = final_pass(events, depth)
    root = next(n for n in nodes if n["kind"] == "root")
    searches = root["path_move_searches"]
    last = searches[-1] if searches else None
    return {"move": root["path_move"], "outcome": root["path_move_outcome"],
            "last_search": last,
            "bound": bound_kind(last["score"], last["alpha"], last["beta"]) if last else None}


def trace_pair(root, paths, disabled, name):
    traced = {}
    for route in ("s", "a"):
        output = OUT / "traces" / f"{root['id']}-{route}-{name}.jsonl"
        search = run_search(root["prefix"], f"depth {root['d0']}", disabled,
                            paths[route], output)
        events = read_jsonl(output)
        starts = [e for e in events if e["type"] == "search_start"]
        expected = {"type": "search_start", "path": paths[route],
                    "disabled": list(disabled), "depth_limit": root["d0"], "node_limit": None}
        if len(starts) != 1:
            raise ValueError("追跡ファイルの search_start が一意でない")
        # seq は全イベントに付く通し番号であり、探索条件の比較から除く。
        start = {key: starts[0][key] for key in expected}
        start["disabled"] = sorted(start["disabled"])
        if start != dict(expected, disabled=sorted(disabled)):
            raise ValueError("追跡ファイルの探索条件が一致しない")
        final, _, _ = final_pass(events, root["d0"])
        if (search["depth"] != root["d0"] or final["best_move"] != search["bestmove"]
                or final["score"] != search["score"]):
            raise ValueError("追跡の最後の窓と USI の結果が一致しない")
        traced[route] = {"search": search, "events": events, "file": str(output)}
    assert traced["s"]["search"]["bestmove"] == traced["a"]["search"]["bestmove"], (
        "2本の追跡の最善手が一致しない")
    return traced


def move_loss(root, move, cache):
    if move in ("resign", "win", "none", "0000"):
        raise ValueError(f"{root['id']}: 子局面を評価できない bestmove: {move}")
    if move not in root["values"]:
        root["values"][move] = -cache.search(root["prefix"] + [move])["score"]
    return root["values"][root["a"]] - root["values"][move]


def intervention(root, paths, cache, disabled, name):
    traced = trace_pair(root, paths, disabled, name)
    bestmove = traced["s"]["search"]["bestmove"]
    loss = move_loss(root, bestmove, cache)
    return {"disabled": list(disabled), "bestmove": bestmove, "loss": loss,
            "success": Decimal(str(loss)) < M,
            "root_moves": {p: root_move_value(t["events"], root["d0"])
                           for p, t in traced.items()},
            "searches": {p: t["search"] for p, t in traced.items()},
            "traces": {p: t["file"] for p, t in traced.items()}}


def process_root(root, cache):
    paths = {p: [root[p]] + root["children"][root[p]]["pv"] for p in ("s", "a")}
    baseline = trace_pair(root, paths, (), "none")
    analyses = {}
    for route, trace in baseline.items():
        check_proposal(root, trace["search"])
        analysis = analyze_trace(trace["events"], root["d0"], root["values"][root[route]])
        if analysis["D"] is not None:
            confirmation = cache.search(root["prefix"] + paths[route][:analysis["D"]])
            analysis = confirm_divergence(analysis, confirmation["score"])
        analyses[route] = analysis
    involved = sorted(set(analyses["s"]["mechanisms"]) | set(analyses["a"]["mechanisms"]))
    singles = {m: intervention(root, paths, cache, (m,), m) for m in MECHANISMS}
    combined = intervention(root, paths, cache, involved, "combined") if involved else None
    classification = classify(involved, singles, combined)
    classification["stop_reasons"] = {p: a["reason"] for p, a in analyses.items()}
    fixed_nodes = {}
    for mechanism, result in singles.items():
        if result["success"]:
            search = run_search(root["prefix"], "nodes 100000", (mechanism,))
            fixed_nodes[mechanism] = dict(search, loss=move_loss(root, search["bestmove"], cache))
    return {"id": root["id"], "d0": root["d0"], "s0": root["s"], "a_star": root["a"],
            "prefix": root["prefix"], "paths": paths, "values_root_view": root["values"],
            "prerequisites": root["prerequisites"], "analyses": analyses, "involved": involved,
            "baseline": {p: {"search": t["search"], "trace": t["file"]}
                         for p, t in baseline.items()},
            "interventions": singles, "combined": combined,
            "classification": classification, "fixed_nodes": fixed_nodes}


def summary_markdown(result):
    lines = ["# 探索の見落としの追跡結果", "",
             "値と損失の単位はセンチポーンである。判定差 m は142.1とする。",
             "経路の s は提案手、a は教師最善手から始まる。上下界は各訪問の入口の窓で判定する。",
             "機構名は TraceDisable の識別子であり、事前登録の介入の表に対応する。", ""]

    def table(headers, rows):
        lines.append("| " + " | ".join(headers) + " |")
        lines.append("| " + " | ".join(["---"] * len(headers)) + " |")
        for row in rows:
            lines.append("| " + " | ".join("該当なし" if v is None else str(v) for v in row) + " |")
        lines.append("")

    for root in result["roots"]:
        lines.extend([f"## 根 {root['id']}", ""])
        table(["d0", "s0", "a*", "分類", "介入だけで成功"],
              [[root["d0"], root["s0"], root["a_star"], root["classification"]["category"],
                ", ".join(root["classification"]["intervention_only"]) or "なし"]])
        rows = []
        for route, a in root["analyses"].items():
            n = a["node"]
            rows.append([route, a["D"], a["reason"], a["outcome"], a["t_D"], a["t_prime_D"],
                         n["result"] if n else None, f"({n['alpha']}, {n['beta']})" if n else None,
                         n["bound"] if n else None, ", ".join(a["mechanisms"]) or "なし"])
        table(["経路", "D", "停止の理由", "停止時の outcome", "t_D（N_D手番側）",
               "t'_D（N_D手番側）", "N_Dの値（N_D手番側）", "窓（N_D手番側）", "上下界", "関与機構"], rows)
        interventions = list(root["interventions"].items())
        if root["combined"] is not None:
            interventions.append(("同時: " + ", ".join(root["involved"]), root["combined"]))
        table(["無効化した機構", "最善手", "損失（根手番側）", "成功"],
              [[name, r["bestmove"], r["loss"], r["success"]] for name, r in interventions])
        if root["combined"] is None:
            lines.extend(["関与機構がないため、同時の介入は行わない。", ""])
        rows = []
        for name, r in interventions:
            for route, v in r["root_moves"].items():
                s = v["last_search"]
                rows.append([name, route, v["move"], v["outcome"], s["score"] if s else None,
                             f"({s['alpha']}, {s['beta']})" if s else None, v["bound"]])
        table(["介入", "経路", "根の手", "outcome", "最後の値（根手番側）", "窓（根手番側）", "上下界"], rows)
        lines.extend(["次の固定100,000ノードの結果は分類に使わない。", ""])
        table(["無効化した機構", "最善手", "完了深さ", "損失（根手番側）"],
              [[name, r["bestmove"], r["depth"], r["loss"]] for name, r in root["fixed_nodes"].items()])
    lines.extend(["## 機構ごとの件数", ""])
    table(["機構", "件数（根）"], result["aggregate"]["counts"].items())
    candidates = ", ".join(result["aggregate"]["three_or_more"]) or "なし"
    lines.extend([f"3根以上で数えられた機構は、{candidates}。", ""])
    if result["aggregate"]["qsearch_note_required"]:
        lines.append("静止探索の3機構のいずれかが1根以上で数えられた。フェーズ1の末端の取り出しも同じ静止探索を使うため、この関与を注記する必要がある。")
    else:
        lines.append("静止探索の3機構はいずれも0根であり、件数規則による注記は不要である。")
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jobs", type=int, default=1, help="独立な探索の最大並列数（既定1）")
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs は1以上にすること")
    if not BIN.is_file():
        raise SystemExit(f"実行ファイルがない: {BIN}\nworktree で次を実行すること:\n"
                         "CARGO_TARGET_DIR=target/search-trace cargo build --release --features search-trace --bin minase")
    roots = load_roots()
    engine_hash = sha256(BIN)
    OUT.mkdir(parents=True, exist_ok=True)
    cache = SearchCache(OUT / "searches.json", engine_hash)
    # 全根の前提条件A、次いでBを通過してから追跡と介入に進む。
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        child_tasks = [(r, m) for r in roots for m in dict.fromkeys((r["a"], r["s"]))]

        def verify_child(task):
            root, move = task
            search = cache.search(root["prefix"] + [move], fresh=True)
            check_child(root, move, search)
            return root["id"], move, search

        children = list(pool.map(verify_child, child_tasks))
        for root in roots:
            root["children"] = {move: s for rid, move, s in children if rid == root["id"]}
            root["prerequisites"] = {"A": root["children"], "B": {}}
            for move, search in root["children"].items():
                root["values"][move] = -search["score"]

        def verify_proposal(task):
            root, limit = task
            search = run_search(root["prefix"], limit)
            check_proposal(root, search)
            return root["id"], limit, search

        proposals = list(pool.map(verify_proposal, [(r, limit) for r in roots
                              for limit in ("nodes 100000", f"depth {r['d0']}")]))
        for root in roots:
            root["prerequisites"]["B"] = {limit: s for rid, limit, s in proposals if rid == root["id"]}
        results = list(pool.map(lambda r: process_root(r, cache), roots))
    result = {"engine_sha256": engine_hash, "script_sha256": sha256(Path(__file__)),
              "reference_sha256": sha256(REFERENCE), "margin_cp": str(M), "roots": results,
              "aggregate": aggregate(r["classification"] for r in results)}
    write_json(OUT / "result.json", result)
    (OUT / "summary.md").write_text(summary_markdown(result), encoding="utf-8")
    print(f"result.json SHA-256: {sha256(OUT / 'result.json')}")
    print(f"trace_roots.py SHA-256: {result['script_sha256']}")


if __name__ == "__main__":
    main()
