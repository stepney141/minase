"""秒読み診断で共有する、整数時計の再構成と丸めなしの集計。"""

from collections import Counter
from collections.abc import Iterable
from typing import Any

NS_PER_MS = 1_000_000
COLORS = ("black", "white")
FORMULAS = ("quadruple-soft", "quadruple-main", "target", "total", "byoyomi-opening")
STOP_REASONS = ("depth", "nodes", "soft", "hard", "external")


def budget_ms(
    remaining_ms: int, increment_ms: int, byoyomi_ms: int, ply: int, formula: str
) -> tuple[int, int]:
    """指定した予算式を整数ミリ秒で評価し、softとhardを返す。

    設計書の代表時計を、targetとtotalの順に確認する。
    totalのsoftは局面適応の係数を掛ける前の標準予算である。

    >>> [budget_ms(300000, 0, 10000, 0, f) for f in ("target", "total")]
    [(933, 933), (933, 1466)]
    >>> [budget_ms(300000, 0, 10000, 36, f) for f in ("target", "total")]
    [(9449, 9449), (9449, 15245)]
    >>> [budget_ms(0, 0, 10000, 0, f) for f in ("target", "total")]
    [(8000, 8000), (8000, 8000)]
    >>> [budget_ms(10000, 0, 10000, 0, f) for f in ("target", "total")]
    [(19970, 19970), (19970, 19970)]
    >>> [budget_ms(10000, 100, 0, 0, f) for f in ("target", "total")]
    [(11, 11), (11, 57)]
    >>> [budget_ms(60000, 200, 0, 0, f) for f in ("target", "total")]
    [(40, 40), (40, 203)]
    >>> [budget_ms(0, 100, 10000, 0, f) for f in ("target", "total")]
    [(8070, 8070), (8070, 8070)]
    >>> [budget_ms(*c, "byoyomi-opening") for c in ((300000, 0, 10000, 0), (300000, 0, 10000, 36), (10000, 100, 0, 0), (0, 0, 10000, 0))]
    [(2133, 8532), (9449, 37796), (114, 456), (8000, 8000)]
    """
    if formula not in FORMULAS:
        raise ValueError(f"未知の予算式: {formula}")
    if min(remaining_ms, increment_ms, byoyomi_ms, ply) < 0:
        raise ValueError("時計とplyは非負でなければならない")
    moves_to_go = max(100, (450 - ply) // 2)
    main = remaining_ms // moves_to_go + increment_ms * 7 // 10
    byoyomi = byoyomi_ms * 8 // 10
    soft_raw = main + byoyomi
    safe_hard = max(1, remaining_ms + byoyomi_ms - 30)
    if formula in ("target", "total"):
        hard_raw = 5 * main + byoyomi if formula == "total" else soft_raw
        if byoyomi_ms > 0 and 0 < remaining_ms < byoyomi_ms * 12 // 10:
            soft_raw = hard_raw = safe_hard
        elif remaining_ms > 0:
            weight = min(40, ply + 4)
            soft_raw = soft_raw * weight // 40
            hard_raw = hard_raw * weight // 40
        else:
            hard_raw = soft_raw
        return max(1, min(soft_raw, safe_hard)), max(1, min(hard_raw, safe_hard))
    if formula == "byoyomi-opening":
        # 秒読みの項にだけ序盤の係数を掛ける（time-management-efficiency.md）。
        weight = min(40, ply + 4) if remaining_ms > 0 else 40
        soft_raw = main + byoyomi_ms * 8 * weight // 400
    ceiling = 4 * main + byoyomi if formula == "quadruple-main" else 4 * soft_raw
    hard = max(1, min(ceiling, remaining_ms // 4 + byoyomi, safe_hard))
    return min(soft_raw, hard), hard


def advance_clock(
    remaining_ns: int, elapsed_ns: int, increment_ns: int, byoyomi_ns: int
) -> tuple[int, bool]:
    """時計を進め、時間切れも返す。診断では時間切れ後も時計を進める。"""
    if min(remaining_ns, elapsed_ns, increment_ns, byoyomi_ns) < 0:
        raise ValueError("時計と思考時間は非負でなければならない")
    return (
        max(remaining_ns - elapsed_ns, 0) + increment_ns,
        elapsed_ns > remaining_ns + byoyomi_ns,
    )


def replay_game(
    turns: list[dict[str, Any]],
    initial_clock_by_color: dict[str, dict[str, int]],
    ply_origin: int,
    formula: str,
    *,
    game_id: str,
) -> list[dict[str, Any]]:
    """1局を再構成し、側ごとの手記録・枯渇手数・時間切れ件数を返す。"""
    if formula not in FORMULAS:
        raise ValueError(f"未知の予算式: {formula}")
    if ply_origin < 0:
        raise ValueError("ply_originは非負でなければならない")
    remaining = {
        side: initial_clock_by_color[side]["base_ms"] * NS_PER_MS for side in COLORS
    }
    records = {
        side: {
            "game": game_id,
            "side": side,
            "turns": [],
            "depletion_move": None,
            "time_forfeits": 0,
        }
        for side in COLORS
    }
    for index, turn in enumerate(turns):
        side = turn["side"]
        clock = initial_clock_by_color[side]
        record = records[side]
        ply = ply_origin + index
        soft, hard = budget_ms(
            remaining[side] // NS_PER_MS,
            clock["increment_ms"],
            clock["byoyomi_ms"],
            ply,
            formula,
        )
        elapsed = turn["think_time_ns"]
        after, forfeit = advance_clock(
            remaining[side], elapsed,
            clock["increment_ms"] * NS_PER_MS, clock["byoyomi_ms"] * NS_PER_MS,
        )
        record["turns"].append({
            **turn,
            "ply": ply,
            "remaining_ns": remaining[side],
            "remaining_after_ns": after,
            "soft_ms": soft,
            "hard_ms": hard,
            "phase": "main" if remaining[side] > 0 else "byoyomi",
            # soft=0の比は未定義であり、集計の有効標本へ含めない。
            "think_soft_ratio": elapsed / (soft * NS_PER_MS) if soft > 0 else None,
            "think_hard_ratio": elapsed / (hard * NS_PER_MS),
            "hard_overrun_ms": (elapsed - hard * NS_PER_MS) / NS_PER_MS,
            "byoyomi_utilization": (
                elapsed / (clock["byoyomi_ms"] * NS_PER_MS)
                if remaining[side] == 0 and clock["byoyomi_ms"] > 0 else None
            ),
            "time_forfeit": forfeit,
        })
        if after == 0 and record["depletion_move"] is None:
            record["depletion_move"] = len(record["turns"])
        record["time_forfeits"] += int(forfeit)
        remaining[side] = after
    return list(records.values())


def quantiles(values: Iterable[float]) -> tuple[float | None, float | None]:
    """中央値と、補間しないceil(0.9n)番目の90パーセンタイルを返す。"""
    ordered = sorted(values)
    n = len(ordered)
    if n == 0:
        return None, None
    median = ordered[n // 2] if n % 2 else (ordered[n // 2 - 1] + ordered[n // 2]) / 2
    return median, ordered[(9 * n + 9) // 10 - 1]


def summarize_turns(turns: list[dict[str, Any]]) -> dict[str, Any]:
    """選択された手の深さ・予算比・超過・秒読み利用率を集計する。"""
    depths = [
        turn["evaluation"]["depth"] for turn in turns
        if turn.get("evaluation") is not None and turn["evaluation"].get("depth") is not None
    ]
    soft_ratios = [
        turn["think_soft_ratio"] for turn in turns if turn["think_soft_ratio"] is not None
    ]
    soft_median, soft_p90 = quantiles(soft_ratios)
    hard_median, hard_p90 = quantiles(turn["think_hard_ratio"] for turn in turns)
    utilization = [
        turn["byoyomi_utilization"] for turn in turns if turn["byoyomi_utilization"] is not None
    ]
    utilization_median, _ = quantiles(utilization)
    return {
        "turns": len(turns),
        "mean_depth": sum(depths) / len(depths) if depths else None,
        "depth_samples": len(depths),
        "depth_missing": len(turns) - len(depths),
        "depth_zero": depths.count(0),
        "think_soft_median": soft_median,
        "think_soft_p90": soft_p90,
        "think_soft_samples": len(soft_ratios),
        "think_soft_missing": len(turns) - len(soft_ratios),
        "think_hard_median": hard_median,
        "think_hard_p90": hard_p90,
        "hard_overrun_max_ms": max(turn["hard_overrun_ms"] for turn in turns) if turns else None,
        "byoyomi_utilization_median": utilization_median,
        "byoyomi_utilization_samples": len(utilization),
    }


def summarize(records: list[dict[str, Any]]) -> dict[str, Any]:
    """局・側の記録をまとめ、丸めずにJSON化できる集計を返す。"""
    turns = [turn for record in records for turn in record["turns"]]
    phases = {}
    for phase in ("main", "byoyomi"):
        selected = [turn for turn in turns if turn["phase"] == phase]
        groups: dict[str, list[dict[str, Any]]] = {reason: [] for reason in ("hard", "soft", "forced")}
        for turn in selected:
            reason = turn.get("stop_reason")
            if reason is not None and reason not in STOP_REASONS:
                raise ValueError(f"未知の停止理由: {reason}")
            # ハーネス記録にはforced欄がなく、その場合はstop行の分類を保つ。
            if turn.get("forced") is True:
                reason = "forced"
            key = "missing" if reason is None else reason
            groups.setdefault(key, []).append(turn)
        reasons = {reason: len(group) for reason, group in groups.items() if group}
        missing = len(groups["missing"]) if "missing" in groups else 0
        known_stops = len(selected) - missing
        phases[phase] = {
            **summarize_turns(selected),
            "by_stop_reason": {reason: summarize_turns(group) for reason, group in groups.items()},
            "stop_reasons": dict(sorted(reasons.items())),
            "stop_reason_missing": missing,
            "stop_reason_fractions": {
                reason: count / known_stops
                for reason, count in sorted(reasons.items()) if reason != "missing"
            },
            "hard_fraction": len(groups["hard"]) / known_stops if known_stops else None,
            "hard_denominator": known_stops,
        }
    depletion_counts = Counter(record["depletion_move"] for record in records)
    return {
        "phases": phases,
        "game_sides": len(records),
        "depletion": [
            {"game": r["game"], "side": r["side"], "move": r["depletion_move"]}
            for r in records
        ],
        "depletion_distribution": [
            {"move": move, "game_sides": depletion_counts[move]}
            for move in sorted(move for move in depletion_counts if move is not None)
        ] + [{"move": None, "game_sides": depletion_counts[None]}],
        # 両側の集計では1局につき2件、役割・先手だけなら1局につき1件。
        "not_depleted_game_sides": sum(r["depletion_move"] is None for r in records),
        "time_forfeits": sum(r["time_forfeits"] for r in records),
    }


def format_summary(summary: dict[str, Any]) -> str:
    """集計を表示時だけ丸め、局面別の表と枯渇・時間切れ件数を返す。"""
    def number(value: float | None) -> str:
        return "欠測" if value is None else f"{value:.6f}"

    lines = [
        "phase turns mean-depth depth-missing soft-ratio-median soft-ratio-p90 soft-ratio-n "
        "soft-ratio-missing hard-fraction hard-denominator stop-reasons"
    ]
    for phase, stats in summary["phases"].items():
        reasons = " ".join(
            f"{key}={value}"
            + (f"({stats['stop_reason_fractions'][key]:.2%})" if key != "missing" else "")
            for key, value in stats["stop_reasons"].items()
        )
        lines.append(
            f"{phase} {stats['turns']} {number(stats['mean_depth'])} "
            f"{stats['depth_missing']} {number(stats['think_soft_median'])} "
            f"{number(stats['think_soft_p90'])} {stats['think_soft_samples']} "
            f"{stats['think_soft_missing']} {number(stats['hard_fraction'])} "
            f"{stats['hard_denominator']} {reasons}"
        )
    lines.append(
        "phase stop turns hard-ratio-median hard-ratio-p90 hard-overrun-max-ms "
        "byoyomi-utilization-median byoyomi-utilization-n depth-zero depth-missing"
    )
    for phase, stats in summary["phases"].items():
        for reason, group in [("all", stats), *stats["by_stop_reason"].items()]:
            lines.append(
                f"{phase} {reason} {group['turns']} {number(group['think_hard_median'])} "
                f"{number(group['think_hard_p90'])} {number(group['hard_overrun_max_ms'])} "
                f"{number(group['byoyomi_utilization_median'])} "
                f"{group['byoyomi_utilization_samples']} {group['depth_zero']} {group['depth_missing']}"
            )
    lines.append("枯渇手数の分布（計時開始後、各側の1手目から数える）:")
    for item in summary["depletion_distribution"]:
        label = "未枯渇" if item["move"] is None else f"{item['move']}手目"
        lines.append(f"  {label}: {item['game_sides']}件")
    lines.append(
        f"未枯渇={summary['not_depleted_game_sides']}/{summary['game_sides']}件（各局の側ごと） "
        f"time_forfeits={summary['time_forfeits']}"
    )
    return "\n".join(lines)
