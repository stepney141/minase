"""固定局面集の推論結果を、合法手の方策と価値で照合する。

ONNX RuntimeのCPUでは--policy logitsを選び、logitと価値をそれぞれ許容誤差1×10⁻⁴で比べる。
TensorRTのFP16（16ビット浮動小数点）では--policy probabilitiesを選び、探索が使う合法手の確率で比べる。
確率は、各局面の合法手のlogitから、基準と候補のそれぞれで倍精度のsoftmaxにより計算する。
--policy-toleranceと--value-toleranceは必須であり、FP16では確率1×10⁻²、価値2×10⁻²などを指定する。

学習したネットの測定では、FP16の丸めの累積によるlogitの差の最大は4.3×10⁻²、価値の差の最大は1.1×10⁻²だったが、確率の差の最大は2.8×10⁻³であった。
各logitを列の隣の着手から読み違えた場合、局面ごとの確率の差の最大は中央値3.5×10⁻²、全体で0.21となった。
この測定では丸めによる差と読み違えによる差が分かれており、確率の許容誤差1×10⁻²でも出力の並びのずれや読み違えを検出できる。
測定の詳細はdocs/measurements/alphazero-trainer/alphazero-trainer-trained-net-output-check.mdにある。

最大のlogitを持つ手の不一致は件数と基準のlogitの差を報告し、合否に使わない。
logitで比べる場合、各logitの差が許容誤差t以下なら、候補が最大とした手の基準logitは基準の最大から2t以内に収まる。
この範囲は要素ごとの検査に含まれるため、最大の手を別途検査する必要はない。
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
    if not data["positions"]:
        raise ValueError(f"{path}: positions must not be empty")
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


def softmax(logits: list[float]) -> list[float]:
    """有限な合法手のlogitを倍精度で確率へ正規化する。合法手がなければ空列を返す。"""
    if not logits:
        return []
    maximum = max(logits)
    weights = [math.exp(logit - maximum) for logit in logits]
    total = sum(weights)
    return [weight / total for weight in weights]


def compare(
    reference: dict, candidate: dict, policy: str, policy_tolerance: float, value_tolerance: float,
) -> tuple[dict, list[str]]:
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
    max_probability_diff = 0.0
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
            left_probabilities = softmax([float(logit) for _, logit in left_moves])
            right_probabilities = softmax([float(logit) for _, logit in right_moves])
            for left_probability, right_probability in zip(left_probabilities, right_probabilities):
                max_probability_diff = max(max_probability_diff, abs(left_probability - right_probability))
            # logitで比べる場合、各logitの差がt以下なら、候補が選んだ手の基準logitと基準の最大との差は2t以下。
            # どちらの比較方法でも、不一致の件数と差は報告だけに使う。
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

    if policy == "logits":
        policy_name, max_policy_diff = "logit", max_logit_diff
    else:
        policy_name, max_policy_diff = "probability", max_probability_diff
    if max_policy_diff > policy_tolerance:
        problems.append(f"maximum {policy_name} difference {max_policy_diff} exceeds tolerance {policy_tolerance}")
    if max_value_diff > value_tolerance:
        problems.append(f"maximum value difference {max_value_diff} exceeds tolerance {value_tolerance}")

    # 比較できない指標を0と報告しない。JSONで表せない非有限値もnullにする。
    comparable = len(expected) == len(actual)
    summary = {
        "positions": len(expected),
        "max_logit_abs_diff": max_logit_diff
        if comparable and same_move_counts and finite_logits and math.isfinite(max_logit_diff)
        else None,
        "max_probability_abs_diff": max_probability_diff
        if comparable and same_move_counts and finite_logits and math.isfinite(max_probability_diff)
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


def finite_nonnegative_float(value: str) -> float:
    tolerance = float(value)
    if not math.isfinite(tolerance) or tolerance < 0:
        raise argparse.ArgumentTypeError("tolerance must be finite and nonnegative")
    return tolerance


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--policy", choices=("logits", "probabilities"), required=True)
    parser.add_argument("--policy-tolerance", type=finite_nonnegative_float, required=True)
    parser.add_argument("--value-tolerance", type=finite_nonnegative_float, required=True)
    args = parser.parse_args()
    summary = {
        "positions": None,
        "max_logit_abs_diff": None,
        "max_probability_abs_diff": None,
        "max_value_abs_diff": None,
        "best_move_mismatches": None,
        "best_move_mismatch_gaps": None,
    }
    try:
        reference = read_result(args.reference)
        candidate = read_result(args.candidate)
        summary, problems = compare(reference, candidate, args.policy, args.policy_tolerance, args.value_tolerance)
    except (OSError, ValueError, KeyError, TypeError, OverflowError) as error:
        problems = [str(error)]
    print(json.dumps(summary, allow_nan=False, separators=(",", ":")))
    for problem in problems:
        print(f"comparison failed: {problem}", file=sys.stderr)
    return int(bool(problems))


if __name__ == "__main__":
    raise SystemExit(main())
