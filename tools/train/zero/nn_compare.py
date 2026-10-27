"""固定局面集の推論結果のlogitと価値を照合する。

各logitの差が許容誤差t以下なら、候補が最大とした手の基準logitは基準の最大から2t以内に収まる。
この範囲は要素ごとの検査で保証されるため、最大の手の不一致は件数と差のみを報告し、合否に使わない。
"""

import argparse
import json
import math
from pathlib import Path
import sys


def read_result(path: Path) -> dict:
    """比較に必要なJSONの構造を検査する。有限性は比較時に両方の全値を検査する。"""
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict) or not isinstance(data["network_sha256"], str):
        raise ValueError(f"{path}: expected an object with network_sha256")
    if not isinstance(data["positions"], list):
        raise ValueError(f"{path}: positions must be a list")
    for index, position in enumerate(data["positions"]):
        if not isinstance(position, dict) or not isinstance(position["sfen"], str):
            raise ValueError(f"{path}: position {index} must have a string sfen")
        if type(position["value"]) not in (int, float) or not isinstance(position["moves"], list):
            raise ValueError(f"{path}: position {index} must have a numeric value and a moves list")
        for move in position["moves"]:
            if (
                not isinstance(move, list)
                or len(move) != 2
                or type(move[0]) is not int
                or type(move[1]) not in (int, float)
            ):
                raise ValueError(f"{path}: position {index} must have [integer label, numeric logit] pairs")
    return data


def compare(reference: dict, candidate: dict, tolerance: float) -> tuple[dict, list[str]]:
    problems = []
    if reference["network_sha256"] != candidate["network_sha256"]:
        problems.append("network_sha256 differs")
    expected = reference["positions"]
    actual = candidate["positions"]
    if len(expected) != len(actual):
        problems.append(f"position count differs: reference={len(expected)}, candidate={len(actual)}")

    finite_logits = True
    finite_values = True
    for name, positions in (("reference", expected), ("candidate", actual)):
        for index, position in enumerate(positions):
            if not math.isfinite(position["value"]):
                finite_values = False
                problems.append(f"{name} position {index}: non-finite value")
            if any(not math.isfinite(logit) for _, logit in position["moves"]):
                finite_logits = False
                problems.append(f"{name} position {index}: non-finite logit")

    max_logit_diff = 0.0
    max_value_diff = 0.0
    best_move_mismatches = 0
    best_move_mismatch_gaps = []
    same_move_counts = True
    same_move_labels = True
    for index, (left, right) in enumerate(zip(expected, actual)):
        if left["sfen"] != right["sfen"]:
            problems.append(f"position {index}: sfen differs")
        left_moves, right_moves = left["moves"], right["moves"]
        labels_match = [move[0] for move in left_moves] == [move[0] for move in right_moves]
        if not labels_match:
            problems.append(f"position {index}: policy label sequence differs")
        same_move_counts &= len(left_moves) == len(right_moves)
        same_move_labels &= labels_match
        if finite_values:
            max_value_diff = max(max_value_diff, abs(left["value"] - right["value"]))
        if finite_logits:
            for (_, left_logit), (_, right_logit) in zip(left_moves, right_moves):
                max_logit_diff = max(max_logit_diff, abs(left_logit - right_logit))
            # 各logitの差がt以下なら、候補が選んだ手の基準logitと基準の最大との差は2t以下。
            # この範囲は要素ごとの検査で保証されるので、不一致の件数と差は報告だけに使う。
            # 同率なら列の先頭を採る。ラベル列の一致も別に検査する。
            left_best = max(range(len(left_moves)), key=lambda i: left_moves[i][1]) if left_moves else None
            right_best = max(range(len(right_moves)), key=lambda i: right_moves[i][1]) if right_moves else None
            left_label = left_moves[left_best][0] if left_best is not None else None
            right_label = right_moves[right_best][0] if right_best is not None else None
            if left_label != right_label:
                best_move_mismatches += 1
                if labels_match:
                    gap = left_moves[left_best][1] - left_moves[right_best][1]
                    best_move_mismatch_gaps.append([index, gap])

    if max_logit_diff > tolerance:
        problems.append(f"maximum logit difference {max_logit_diff} exceeds tolerance {tolerance}")
    if max_value_diff > tolerance:
        problems.append(f"maximum value difference {max_value_diff} exceeds tolerance {tolerance}")

    # 比較できない指標を0と報告しない。JSONで表せない非有限値もnullにする。
    comparable = len(expected) == len(actual)
    summary = {
        "positions": len(expected),
        "max_logit_abs_diff": max_logit_diff
        if comparable and same_move_counts and finite_logits and math.isfinite(max_logit_diff)
        else None,
        "max_value_abs_diff": max_value_diff
        if comparable and finite_values and math.isfinite(max_value_diff)
        else None,
        "best_move_mismatches": best_move_mismatches if comparable and finite_logits else None,
        "best_move_mismatch_gaps": best_move_mismatch_gaps
        if comparable and same_move_labels and finite_logits
        and all(math.isfinite(gap) for _, gap in best_move_mismatch_gaps)
        else None,
    }
    return summary, problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--tolerance", type=float, required=True)
    args = parser.parse_args()
    summary = {
        "positions": None,
        "max_logit_abs_diff": None,
        "max_value_abs_diff": None,
        "best_move_mismatches": None,
        "best_move_mismatch_gaps": None,
    }
    try:
        if not math.isfinite(args.tolerance) or args.tolerance < 0:
            raise ValueError("tolerance must be finite and nonnegative")
        reference = read_result(args.reference)
        candidate = read_result(args.candidate)
        summary, problems = compare(reference, candidate, args.tolerance)
    except (OSError, ValueError, KeyError, TypeError, OverflowError) as error:
        problems = [str(error)]
    print(json.dumps(summary, allow_nan=False, separators=(",", ":")))
    for problem in problems:
        print(f"comparison failed: {problem}", file=sys.stderr)
    return int(bool(problems))


if __name__ == "__main__":
    raise SystemExit(main())
