"""200局の末端標本を解析する。条件は docs/plans/qsearch-output-training.md に従う。

整数の推論値で予測を作り、勾配には学習時の連続な2端点PSTの特徴を使う。
整数切捨てと評価上限は微分しない。詰み根には末端の特徴がないので、特徴・
損失・勾配の比較から除外する。比の分母が0の根と未定義のコサインは別記する。
履歴は記録のある対局だけを含むため、費用の分母には --sample-games を使う。
"""

from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import re
import subprocess
import sys

import numpy as np

WT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(WT / "tools/train/pst"))
from features import MIRRORED_FEATURE_COUNT, canonical_feature_indices, feature_indices
from lookahead import compute_lookahead
from mnsd import map_records, read_header, sha256_file
from taper import phase_numerators, phase_ratios
from train_pst import build_targets, estimate_k, integer_evaluate, read_mnpt

DEFAULT_DIR = Path("/home/stepney141/board-games/minase/data/qsearch-output-training/phase1")
OUTPUT_K = 1072.6529541015625
NODES = 10_000_000
ROOT_KINDS = ("quiet", "capture", "promotion")
LEAF_KINDS = ("static", "max_ply", "mate")
TRAIN_FRACTION = 19 / 20  # mnsd.py: 対局ハッシュ % 20 == 0 が検証集合。


def distribution(values):
    """有限値の分布を返す。空集合には0を補わずnullを記録する。"""
    values = np.asarray(values, dtype=np.float64)
    if values.ndim != 1 or not np.all(np.isfinite(values)):
        raise ValueError("distribution requires a finite vector")
    return {"count": len(values), **{
        name: float(operation(values)) if len(values) else None
        for name, operation in (
            ("mean", np.mean), ("median", np.median), ("min", np.min),
            ("max", np.max), ("p95", lambda v: np.quantile(v, .95)),
            ("abs_p95", lambda v: np.quantile(np.abs(v), .95)),
            ("nonzero_fraction", lambda v: np.mean(v != 0)),
        )}}


def feature_vectors(records):
    """mirroredモデルの2端点重みに対する特徴を手番側視点で返す。"""
    indices = canonical_feature_indices(feature_indices(
        records["board"], records["stm"], records["lion"]))
    counts = np.zeros((len(records), MIRRORED_FEATURE_COUNT + 1), dtype=np.float64)
    np.add.at(counts, (np.arange(len(records))[:, None], indices), 1)
    phi = phase_ratios(records["board"])
    return (counts[:, :-1, None] * np.stack((phi, 1 - phi), axis=1)[:, None, :]).reshape(len(records), -1)


def regression_changes(root_phi, leaf_phi, root_value, leaf_value, k, targets, scale=OUTPUT_K):
    """根の視点へ符号をそろえ、BCEと各重みの勾配を比較する。"""
    sign = np.where(np.asarray(k) % 2 == 0, 1, -1)
    q_phi = sign[:, None] * leaf_phi
    r_logits = np.asarray(root_value) / scale
    q_logits = sign * np.asarray(leaf_value) / scale
    r_loss = np.logaddexp(0, r_logits) - targets * r_logits
    q_loss = np.logaddexp(0, q_logits) - targets * q_logits
    r_gradient = ((1 / (1 + np.exp(-r_logits)) - targets) / scale)[:, None] * root_phi
    q_gradient = ((1 / (1 + np.exp(-q_logits)) - targets) / scale)[:, None] * q_phi
    norm = np.linalg.norm(r_gradient, axis=1)
    difference = np.linalg.norm(q_gradient - r_gradient, axis=1)
    return {"feature_changed": np.any(root_phi != q_phi, axis=1),
            "loss_difference": q_loss - r_loss,
            "gradient_changed": np.any(r_gradient != q_gradient, axis=1),
            "gradient_difference_norm": difference,
            "static_gradient_norm": norm,
            "gradient_ratio": np.divide(difference, norm, out=np.full(len(norm), np.nan), where=norm != 0),
            "static_gradient": r_gradient, "q_gradient": q_gradient}


def estimate_cost(kinds, nodes, cpu_seconds, sample_games):
    """ノード単価から15,000局・訓練95%・10回・同時16の費用を見積もる。"""
    kinds, nodes = np.asarray(kinds), np.asarray(nodes, dtype=np.int64)
    if (sample_games <= 0 or not np.isfinite(cpu_seconds) or cpu_seconds <= 0
            or nodes.shape != kinds.shape or np.any(nodes <= 0) or nodes.sum() <= 0):
        raise ValueError("cost requires positive games, CPU seconds and node counts")
    per_node = cpu_seconds / int(nodes.sum())
    factor = 15000 / sample_games * TRAIN_FRACTION
    groups = {}
    for kind in ROOT_KINDS:
        selected = kinds == kind
        count = int(np.count_nonzero(selected))
        total = int(nodes[selected].sum())
        groups[kind] = {"count": count, "nodes": total, "cpu_seconds": total * per_node,
                        "cpu_seconds_per_root": total * per_node / count if count else None,
                        "estimated_training_records": count * factor}
    minutes = {"Q": groups["quiet"]["cpu_seconds"] * factor * 10 / 16 / 60,
               "Qc": (groups["quiet"]["cpu_seconds"] + groups["capture"]["cpu_seconds"]) * factor * 10 / 16 / 60}
    minutes["combined"] = minutes["Q"] + minutes["Qc"]
    within = {"Q": minutes["Q"] <= 60, "Qc": minutes["Qc"] <= 60,
              "combined": minutes["combined"] <= 120}
    return {"cpu_seconds_per_node": per_node, "sample_games": sample_games,
            "target_games": 15000, "training_fraction": TRAIN_FRACTION,
            "extractions": 10, "concurrency": 16, "by_kind": groups,
            "minutes": minutes, "within_budget": within,
            "qc_available": groups["capture"]["count"] > 0,
            "budget_pass": all(within.values())}


def select_roots(kinds, info):
    """静かな根と捕獲根を合わせた母集団から、シード1で最大256根を選ぶ。"""
    eligible = np.array([i for i, (kind, leaf) in enumerate(zip(kinds, info))
                         if kind in ("quiet", "capture") and leaf["k"] >= 1
                         and leaf["kind"] in ("static", "max_ply")], dtype=np.int64)
    if len(eligible) <= 256:
        return eligible
    return np.sort(np.random.default_rng(1).choice(eligible, 256, replace=False))


def score_value(kind, value):
    """USIの詰み距離を根の値へ換算し、-0の負号を保持する。"""
    if kind == "cp":
        return int(value)
    if kind == "mate":
        return (-1 if str(value).startswith("-") else 1) * (30000 - abs(int(value)))
    raise ValueError(f"unknown score kind: {kind}")


def parse_search(lines):
    infos = [line.split() for line in lines if line.startswith("info ")
             and " score " in line and " pv " in line]
    if not infos or not lines[-1].startswith("bestmove "):
        raise ValueError("search lacks final info or bestmove")
    last = infos[-1]
    if "lowerbound" in last or "upperbound" in last:
        raise ValueError("final search score is a bound")
    offset = last.index("score")
    return {"value": score_value(last[offset + 1], last[offset + 2]),
            "score_usi": last[offset + 1:offset + 3],
            "mate": last[offset + 1] == "mate",
            "depth": int(last[last.index("depth") + 1]),
            "nodes": int(last[last.index("nodes") + 1]),
            "bestmove": lines[-1].split()[1]}


def run_search(engine, moves):
    """履歴つき局面を、新しいUSIプロセスで10,000,000ノード探索する。"""
    with subprocess.Popen([str(engine), "--protocol", "usi", "--rules", "engine-default"],
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
                    raise RuntimeError("engine closed output before " + prefix)
                line = line.rstrip("\n")
                if line.startswith(("error", "info string error")):
                    raise RuntimeError(line)
                lines.append(line)
                if line == prefix or line.startswith(prefix + " "):
                    return lines

        try:
            send("usi")
            handshake = receive("usiok")
            if any(line.startswith("option name Trace") for line in handshake):
                raise ValueError("independent search requires a build without search-trace")
            for name in ("USI_Hash", "Threads"):
                if not any(line.startswith(f"option name {name} ") for line in handshake):
                    raise ValueError(f"engine lacks {name}")
            send("setoption name USI_Hash value 64")
            send("setoption name Threads value 1")
            send("isready")
            receive("readyok")
            send("usinewgame")
            send("position startpos" + (" moves " + " ".join(moves) if moves else ""))
            send(f"go nodes {NODES}")
            result = parse_search(receive("bestmove"))
            send("quit")
        except BaseException:
            process.kill()
            raise
    if process.returncode != 0:
        raise RuntimeError(f"engine exit status: {process.returncode}")
    return result


def independent_comparison(records, kinds, info, histories, engine, jobs):
    """独立探索の差を種類別と全体で集計し、選んだ行と各探索値も保存する。"""
    if jobs < 1:
        raise ValueError("jobs must be positive")
    selected = select_roots(kinds, info)

    def compare(index):
        row, leaf = records[index], info[index]
        prefix = histories[int(row["game"])][:int(row["ply"])]
        root = run_search(engine, prefix)
        end = run_search(engine, prefix + leaf["moves"])
        return {"index": int(index), "kind": str(kinds[index]), "k": leaf["k"],
                "game": int(row["game"]), "ply": int(row["ply"]),
                "root": root, "leaf": end,
                "difference_cp": root["value"] - (-1) ** leaf["k"] * end["value"],
                "contains_mate": root["mate"] or end["mate"]}

    with ThreadPoolExecutor(max_workers=jobs) as pool:
        comparisons = list(pool.map(compare, selected))
    groups = {}
    for kind in ("all", "quiet", "capture"):
        rows = [row for row in comparisons if kind == "all" or row["kind"] == kind]
        groups[kind] = {"difference_cp": distribution([r["difference_cp"] for r in rows]),
                        "mate_roots": sum(r["contains_mate"] for r in rows)}
    return {"seed": 1, "limit": 256, "nodes_per_search": NODES,
            "hash_mib": 64, "threads": 1, "jobs": jobs,
            "comparable": bool(len(selected)), "count": len(selected),
            "by_kind": groups, "roots": comparisons}


def read_jsonl(path):
    with Path(path).open(encoding="utf-8") as stream:
        return [json.loads(line) for line in stream]


def read_inputs(sample, kinds_path, history_path, leaves_path, info_path, sample_games):
    """対応する行・対局・末端の符号を検査する。履歴の合法性は抽出器が担う。"""
    header, leaf_header = read_header(sample), read_header(leaves_path)
    if header.rule_set != "L0,P0,R1,E0" or leaf_header.rule_set != header.rule_set:
        raise ValueError("sample and leaves must use engine-default rules")
    records, leaves = map_records(sample), map_records(leaves_path)
    kinds = np.array(Path(kinds_path).read_text(encoding="utf-8").splitlines())
    info = read_jsonl(info_path)
    if len(records) != len(leaves) or len(records) != len(kinds) or len(records) != len(info):
        raise ValueError("MNSD, kinds and info record counts differ")
    if not len(records) or not np.all(np.isin(kinds, ROOT_KINDS)):
        raise ValueError("empty sample or unknown root kind")
    for field in ("score", "result", "game", "ply"):
        if not np.array_equal(records[field], leaves[field]):
            raise ValueError(f"leaves changed source field: {field}")
    histories = {}
    for game in read_jsonl(history_path):
        number, moves = game["game"], game["moves"]
        if type(number) is not int or number in histories or not isinstance(moves, list):
            raise ValueError("invalid or duplicate history game")
        if any(not isinstance(move, str) or not move or any(c.isspace() for c in move) for move in moves):
            raise ValueError("history moves must be individual USI tokens")
        histories[number] = moves
    if list(histories) != list(dict.fromkeys(map(int, records["game"]))):
        raise ValueError("history games or their order do not match MNSD")
    if sample_games < len(histories):
        raise ValueError("sample-games is smaller than recorded games")
    for i, (row, leaf) in enumerate(zip(records, info)):
        if len(histories[int(row["game"])]) < int(row["ply"]):
            raise ValueError(f"row {i}: incomplete game history")
        if leaf["kind"] not in LEAF_KINDS or type(leaf["k"]) is not int or leaf["k"] < 0:
            raise ValueError(f"row {i}: invalid leaf kind or k")
        if (not isinstance(leaf["moves"], list) or len(leaf["moves"]) != leaf["k"]
                or any(not isinstance(m, str) or not m or any(c.isspace() for c in m) for m in leaf["moves"])):
            raise ValueError(f"row {i}: invalid leaf moves")
        if type(leaf["nodes"]) is not int or leaf["nodes"] <= 0 or type(leaf["value"]) is not int:
            raise ValueError(f"row {i}: invalid node count or value")
        if leaf["kind"] == "mate":
            if leaf["leaf_eval"] is not None or not 29000 <= abs(leaf["value"]) <= 30000:
                raise ValueError(f"row {i}: invalid mate result")
            if not np.array_equal(row, leaves[i]):
                raise ValueError(f"row {i}: mate placeholder differs from root")
        else:
            if type(leaf["leaf_eval"]) is not int:
                raise ValueError(f"row {i}: missing static leaf evaluation")
            assert (-1) ** leaf["k"] * leaf["leaf_eval"] == leaf["value"], f"row {i}: leaf/value mismatch"
            if int(leaves[i]["stm"]) != int(row["stm"]) ^ (leaf["k"] % 2):
                raise ValueError(f"row {i}: leaf side-to-move mismatch")
            if leaf["k"] == 0 and not np.array_equal(row, leaves[i]):
                raise ValueError(f"row {i}: k=0 leaf differs from root")
    return records, kinds, histories, leaves, info


def read_cpu_log(path, count, nodes):
    """抽出器の集計行を読み、件数と総ノード数も照合する。"""
    text = Path(path).read_text(encoding="utf-8")
    fields = {}
    for name in ("records", "nodes", "cpu_seconds"):
        matches = re.findall(rf"^{name}:\s*(\S+)\s*$", text, re.MULTILINE)
        if len(matches) != 1:
            raise ValueError(f"log requires exactly one {name} line")
        fields[name] = matches[0]
    if int(fields["records"]) != count or int(fields["nodes"]) != nodes:
        raise ValueError("log totals disagree with leaf info")
    cpu = float(fields["cpu_seconds"])
    if not np.isfinite(cpu) or cpu <= 0:
        raise ValueError("process CPU time must be finite and positive")
    return cpu


def analyze(records, kinds, leaves, info, middlegame, endgame, cpu_seconds, sample_games):
    """仕様1〜5と7を集計する。独立探索は別関数で実施する。"""
    quiet = kinds == "quiet"
    teacher_scores = compute_lookahead(records["game"], records["ply"], records["score"],
                                        .9, 40, average_mask=quiet)
    teacher_k = estimate_k(teacher_scores[quiet], records["result"][quiet]) if np.any(quiet) else None
    targets = (build_targets(records, [teacher_k], np.zeros(len(records), dtype=int), [.75],
                             scores=teacher_scores) if teacher_k is not None else None)
    leaf_kinds = np.array([r["kind"] for r in info])
    ks = np.array([r["k"] for r in info])
    values = np.array([r["value"] for r in info])
    accepted = leaf_kinds != "mate"
    root_eval = integer_evaluate(middlegame, endgame, feature_indices(
        records["board"], records["stm"], records["lion"]), phase_numerators(records["board"]))
    leaf_eval = integer_evaluate(middlegame, endgame, feature_indices(
        leaves["board"], leaves["stm"], leaves["lion"]), phase_numerators(leaves["board"]))
    assert np.array_equal(leaf_eval[accepted], [r["leaf_eval"] for r in info if r["kind"] != "mate"]), "Python leaf evaluation mismatch"
    assert np.array_equal(np.where(ks[accepted] % 2 == 0, 1, -1) * leaf_eval[accepted], values[accepted]), "leaf/value mismatch"
    assert np.array_equal(root_eval[ks == 0], values[ks == 0]), "k=0 root evaluation mismatch"
    costs = estimate_cost(kinds, [r["nodes"] for r in info], cpu_seconds, sample_games)
    groups = {}
    for kind in ROOT_KINDS:
        selected = kinds == kind
        indices = np.flatnonzero(selected & accepted)
        metrics = {name: [] for name in ("feature_changed", "loss_difference", "gradient_changed",
                                         "gradient_difference_norm", "static_gradient_norm", "gradient_ratio")}
        r_sum = np.zeros(2 * MIRRORED_FEATURE_COUNT)
        q_sum = np.zeros_like(r_sum)
        for start in range(0, len(indices), 128):
            chunk = indices[start:start + 128]
            root_phi, leaf_phi = feature_vectors(records[chunk]), feature_vectors(leaves[chunk])
            if targets is None:
                metrics["feature_changed"].extend(np.any(root_phi != (-1.) ** ks[chunk, None] * leaf_phi, axis=1))
                continue
            changes = regression_changes(root_phi, leaf_phi, root_eval[chunk], leaf_eval[chunk], ks[chunk], targets[chunk])
            for name in metrics:
                metrics[name].extend(changes[name])
            r_sum += changes["static_gradient"].sum(axis=0)
            q_sum += changes["q_gradient"].sum(axis=0)
        denominator = float(np.linalg.norm(r_sum) * np.linalg.norm(q_sum))
        ratios = np.asarray(metrics["gradient_ratio"])
        histogram = {str(int(k)): int(n) for k, n in zip(*np.unique(ks[selected], return_counts=True))}
        groups[kind] = {
            "count": int(np.count_nonzero(selected)),
            "leaf_kinds": {name: int(np.count_nonzero(selected & (leaf_kinds == name))) for name in LEAF_KINDS},
            "k": {"histogram": histogram, **distribution(ks[selected])},
            "output_difference_cp": distribution(values[selected] - root_eval[selected]),
            "feature_comparison_count": len(indices),
            "feature_changed_fraction": float(np.mean(metrics["feature_changed"])) if len(indices) else None,
            "regression_count": len(metrics["gradient_changed"]),
            "loss_difference": distribution(metrics["loss_difference"]),
            "gradient_changed_fraction": float(np.mean(metrics["gradient_changed"])) if metrics["gradient_changed"] else None,
            "gradient_ratio": distribution(ratios[np.isfinite(ratios)]),
            "zero_static_gradient_count": int(np.count_nonzero(np.asarray(metrics["static_gradient_norm"]) == 0)),
            "gradient_difference_norm": distribution(metrics["gradient_difference_norm"]),
            "gradient_sum_cosine": float(np.clip(np.dot(r_sum, q_sum) / denominator, -1, 1)) if denominator else None,
        }
    checks = {"leaf_value_consistency": True,
              "trainable_quiet": bool(np.any(quiet & accepted)), "cost_within_budget": costs["budget_pass"]}
    return {"conditions": {"output_k": OUTPUT_K, "teacher_k": teacher_k,
                           "teacher_k_records": int(np.count_nonzero(quiet)), "lambda": .75,
                           "gamma": .9, "plies": 40, "average_kind": "quiet", "split": "all records",
                           "model": "mirrored", "endpoints": 2,
                           "gradient": "integer prediction; continuous PST derivative; no rounding/clipping derivative"},
            "by_kind": groups, "cost": costs, "phase2_conditions": checks,
            "proceed_phase2": all(checks.values())}


def summary_markdown(result):
    """全集計を単位・視点・比較母数つきの表へまとめる。"""
    lines = ["# 静止探索の末端の小標本", "",
             "予測と評価差は根の手番側にそろえる。末端の評価値は末端の手番側で照合する。",
             "詰み根は特徴・損失・勾配の比較から除く。成り根の値は参考値であり学習には使わない。",
             "勾配には整数予測と連続なPSTの微分を使い、整数への切捨てと評価上限は微分しない。",
             "ノルム比は静的勾配が0の根を除き、コサインはどちらかの総勾配が0なら未定義とする。", ""]

    def table(headers, rows):
        def cell(value):
            if value is None:
                return "未定義"
            if isinstance(value, float):
                return f"{value:.9g}"
            return str(value)
        lines.append("| " + " | ".join(headers) + " |")
        lines.append("| " + " | ".join(["---"] * len(headers)) + " |")
        lines.extend("| " + " | ".join(map(cell, row)) + " |" for row in rows)
        lines.append("")

    conditions = result["conditions"]
    table(["出力尺度K（センチポーン）", "教師尺度K_g（センチポーン）", "尺度推定（静かな根、件）", "λ", "γ", "先読み（手）"],
          [[conditions["output_k"], conditions["teacher_k"], conditions["teacher_k_records"], .75, .9, 40]])
    groups = result["by_kind"]
    table(["根の種類", "根（件）", "static（件）", "max_ply（件）", "mate（件）", "k平均（手）", "k最大（手）"],
          [[kind, group["count"], *[group["leaf_kinds"][name] for name in LEAF_KINDS], group["k"]["mean"], group["k"]["max"]]
           for kind, group in groups.items()])
    table(["根の種類", "k（手）", "度数（件）"],
          [[kind, k, count] for kind, group in groups.items() for k, count in group["k"]["histogram"].items()])
    cost = result["cost"]
    table(["生成（局）", "訓練割合", "CPU時間（秒/ノード）", "取り出し（回）", "同時数"],
          [[cost["sample_games"], cost["training_fraction"], cost["cpu_seconds_per_node"], 10, 16]])
    table(["根の種類", "探索ノード数", "推計CPU時間（秒）", "推計CPU時間（秒/根）", "15,000局の訓練集合（件）"],
          [[kind, row["nodes"], row["cpu_seconds"], row["cpu_seconds_per_root"], row["estimated_training_records"]]
           for kind, row in cost["by_kind"].items()])
    table(["候補", "同時16で10回の推計時間（分）", "基準（分）", "基準内"],
          [[name, cost["minutes"][name], 120 if name == "combined" else 60, cost["within_budget"][name]]
           for name in ("Q", "Qc", "combined")])
    lines.extend([f"捕獲根がありQcを実施できる条件は {cost['qc_available']} である。", ""])
    table(["根の種類", "出力差平均（根手番側、センチポーン）", "出力差中央値（根手番側、センチポーン）",
           "出力差絶対値95%点（根手番側、センチポーン）", "出力差が非0の割合"],
          [[kind, *[group["output_difference_cp"][name] for name in ("mean", "median", "abs_p95", "nonzero_fraction")]]
           for kind, group in groups.items()])
    table(["根の種類", "特徴比較（件）", "特徴が異なる割合（根手番側）", "勾配比較（件）",
           "勾配が異なる割合（根手番側）", "勾配総和のコサイン（根手番側）", "静的勾配0（件）"],
          [[kind, group["feature_comparison_count"], group["feature_changed_fraction"], group["regression_count"],
            group["gradient_changed_fraction"], group["gradient_sum_cosine"], group["zero_static_gradient_count"]]
           for kind, group in groups.items()])
    for field, label in (("loss_difference", "二値交差エントロピー差 Q−R（根手番側、自然対数）"),
                         ("gradient_ratio", "勾配差ノルム/静的勾配ノルム（根手番側、無次元）")):
        table(["根の種類", "指標", "件数（件）", "平均", "中央値", "最小", "最大", "95%点"],
              [[kind, label, *[group[field][name] for name in ("count", "mean", "median", "min", "max", "p95")]]
               for kind, group in groups.items()])
    comparison = result["independent_search"]
    lines.extend([f"独立探索はシード1で選んだ {comparison['count']} 根を対象に、各局面10,000,000ノードで実施した。",
                  "根の探索値から、根の視点へ換算した末端の探索値を引く。この比較は進行判定に使わない。", ""])
    table(["根の種類", "比較（件）", "詰み値を含む根（件）", "平均（根手番側、センチポーン）",
           "中央値（根手番側、センチポーン）", "絶対値95%点（根手番側、センチポーン）", "非0の割合"],
          [[kind, group["difference_cp"]["count"], group["mate_roots"],
            *[group["difference_cp"][name] for name in ("mean", "median", "abs_p95", "nonzero_fraction")]]
           for kind, group in comparison["by_kind"].items()])
    table(["フェーズ2の条件", "成立"], result["phase2_conditions"].items())
    lines.append(f"3条件すべての成立は {result['proceed_phase2']} である。")
    return "\n".join(lines) + "\n"


def build_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    for argument, name in (("sample", "sample.bin"), ("kinds", "sample.kinds"),
                           ("history", "sample.history.jsonl"), ("leaves", "leaves.bin"),
                           ("info", "leaves.jsonl"), ("log", "leaves.log")):
        parser.add_argument("--" + argument, type=Path, default=DEFAULT_DIR / name)
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_DIR)
    parser.add_argument("--engine", type=Path, default=WT / "target/release/minase",
                        help="S0のPSTを埋め込んだフィーチャなしの実行ファイル")
    parser.add_argument("--sample-games", type=int, default=200,
                        help="記録0件の対局も含む生成局数（既定200）")
    parser.add_argument("--jobs", type=int, default=1)
    return parser


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    if args.jobs < 1 or args.sample_games < 1:
        parser.error("--jobs and --sample-games must be positive")
    records, kinds, histories, leaves, info = read_inputs(
        args.sample, args.kinds, args.history, args.leaves, args.info, args.sample_games)
    weights = WT / "nets/pst.bin"
    middlegame, endgame, _, scale = read_mnpt(weights)
    if scale != OUTPUT_K:
        raise ValueError("S0 output scale differs from the fixed analysis condition")
    checksum = weights.read_bytes()[48:80]
    if any(read_header(path).network_checksum != checksum for path in (args.sample, args.leaves)):
        raise ValueError("sample or leaf weights differ from S0")
    for endpoint in (middlegame, endgame):
        table = endpoint.reshape(-1, 12, 12)
        if not np.array_equal(table, table[:, :, ::-1]):
            raise ValueError("S0 weights are not mirrored")
    cpu = read_cpu_log(args.log, len(records), sum(row["nodes"] for row in info))
    result = analyze(records, kinds, leaves, info, middlegame, endgame, cpu, args.sample_games)
    result["independent_search"] = independent_comparison(
        records, kinds, info, histories, args.engine.resolve(), args.jobs)
    paths = {name: getattr(args, name) for name in ("sample", "kinds", "history", "leaves", "info", "log", "engine")}
    paths.update(weights=weights, script=Path(__file__))
    result["inputs"] = {name: {"path": str(path.resolve()), "sha256": sha256_file(path).hex()}
                        for name, path in paths.items()}
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report, summary = args.output_dir / "result.json", args.output_dir / "summary.md"
    report.write_text(json.dumps(result, ensure_ascii=False, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    summary.write_text(summary_markdown(result), encoding="utf-8")
    for path in (report, summary, Path(__file__)):
        print(f"{path} SHA-256: {sha256_file(path).hex()}")


if __name__ == "__main__":
    main()
