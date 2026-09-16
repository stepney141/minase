"""秒読み診断で共有する、整数時計の再構成と丸めなしの集計。"""

from collections import Counter
from collections.abc import Iterable
from typing import Any

NS_PER_MS = 1_000_000
COLORS = ("black", "white")
FORMULAS = ("quadruple-soft", "quadruple-main")
STOP_REASONS = ("depth", "nodes", "soft", "hard", "external")


def budget_ms(
    remaining_ms: int, increment_ms: int, byoyomi_ms: int, ply: int, formula: str
) -> tuple[int, int]:
    """指定した予算式を整数ミリ秒で評価し、softとhardを返す。"""
    if formula not in FORMULAS:
        raise ValueError(f"未知の予算式: {formula}")
    if min(remaining_ms, increment_ms, byoyomi_ms, ply) < 0:
        raise ValueError("時計とplyは非負でなければならない")
    moves_to_go = max(100, max(450 - ply, 0) // 2)
    main = remaining_ms // moves_to_go + increment_ms * 7 // 10
    byoyomi = byoyomi_ms * 8 // 10
    soft_raw = main + byoyomi
    ceiling = 4 * soft_raw if formula == "quadruple-soft" else 4 * main + byoyomi
    safe_hard = max(1, remaining_ms + byoyomi_ms - 30)
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


def summarize(records: list[dict[str, Any]]) -> dict[str, Any]:
    """局・側の記録をまとめ、丸めずにJSON化できる集計を返す。"""
    turns = [turn for record in records for turn in record["turns"]]
    phases = {}
    for phase in ("main", "byoyomi"):
        selected = [turn for turn in turns if turn["phase"] == phase]
        depths = []
        reasons: Counter[str] = Counter()
        ratios = []
        for turn in selected:
            evaluation = turn.get("evaluation")
            if evaluation is not None and evaluation.get("depth") is not None:
                depths.append(evaluation["depth"])
            reason = turn.get("stop_reason")
            if reason is not None and reason not in STOP_REASONS:
                raise ValueError(f"未知の停止理由: {reason}")
            reasons["missing" if reason is None else reason] += 1
            if turn["think_soft_ratio"] is not None:
                ratios.append(turn["think_soft_ratio"])
        median, p90 = quantiles(ratios)
        known_stops = len(selected) - reasons["missing"]
        phases[phase] = {
            "turns": len(selected),
            "mean_depth": sum(depths) / len(depths) if depths else None,
            "depth_samples": len(depths),
            "depth_missing": len(selected) - len(depths),
            "stop_reasons": dict(sorted(reasons.items())),
            "stop_reason_missing": reasons["missing"],
            "stop_reason_fractions": {
                reason: count / known_stops
                for reason, count in sorted(reasons.items()) if reason != "missing"
            },
            "think_soft_median": median,
            "think_soft_p90": p90,
            "think_soft_samples": len(ratios),
            "think_soft_missing": len(selected) - len(ratios),
            "hard_fraction": reasons["hard"] / known_stops if known_stops else None,
            "hard_denominator": known_stops,
        }
    return {
        "phases": phases,
        "game_sides": len(records),
        "depletion": [
            {"game": r["game"], "side": r["side"], "move": r["depletion_move"]}
            for r in records
        ],
        # 両側の集計では1局につき2件、役割・先手だけなら1局につき1件。
        "not_depleted_game_sides": sum(r["depletion_move"] is None for r in records),
        "time_forfeits": sum(r["time_forfeits"] for r in records),
    }


def format_summary(summary: dict[str, Any]) -> str:
    """集計を表示時だけ丸め、局面別の表と枯渇・時間切れ件数を返す。"""
    def number(value: float | None) -> str:
        return "欠測" if value is None else f"{value:.6f}"

    lines = [
        "phase turns mean-depth depth-missing ratio-median ratio-p90 ratio-n "
        "ratio-missing hard-fraction hard-denominator stop-reasons"
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
    lines.append("枯渇手数（計時開始後、各側の1手目から数える）:")
    for item in summary["depletion"]:
        if item["move"] is not None:
            lines.append(f"  {item['game']} {item['side']}: {item['move']}手目")
    lines.append(
        f"枯渇なし={summary['not_depleted_game_sides']}/{summary['game_sides']}件（各局の側ごと） "
        f"time_forfeits={summary['time_forfeits']}"
    )
    return "\n".join(lines)
