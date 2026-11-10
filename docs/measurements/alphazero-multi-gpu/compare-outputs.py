"""別々に学んだ2つのネットの固定局面集の出力を、FP16の照合と同じ許容誤差で比べる。

使い方（作業ディレクトリは学習器のtools/train/zero）:
    uv run --frozen python <このファイル> --reference <基準の出力JSON> --candidate <候補の出力JSON> --out <結果JSON>

出力JSONは、どちらも`minase_zero fixture-eval`がPyTorchのfloat32で書いたものとする。
照合はtools/train/zero/nn_compare.pyのcompareを、合法手の確率の許容誤差1×10⁻²、価値の許容誤差5×10⁻²で呼ぶ。
2つのネットは別物なので、compareが返す問題のうち「network_sha256 differs」だけを除き、残りの問題がなければ基準内とする。
"""

import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path.cwd()))
from nn_compare import compare, read_result  # noqa: E402

POLICY_TOLERANCE = 1e-2
VALUE_TOLERANCE = 5e-2

parser = argparse.ArgumentParser()
for name in ("reference", "candidate", "out"):
    parser.add_argument(f"--{name}", type=Path, required=True)
args = parser.parse_args()
reference, candidate = read_result(args.reference), read_result(args.candidate)
summary, problems = compare(reference, candidate, "probabilities", POLICY_TOLERANCE, VALUE_TOLERANCE)
remaining = [problem for problem in problems if problem != "network_sha256 differs"]
result = {
    "reference": str(args.reference), "reference_network_sha256": reference["network_sha256"],
    "candidate": str(args.candidate), "candidate_network_sha256": candidate["network_sha256"],
    "policy_tolerance": POLICY_TOLERANCE, "value_tolerance": VALUE_TOLERANCE,
    **summary, "problems": remaining, "within_tolerance": not remaining,
}
args.out.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
print(json.dumps(result, ensure_ascii=False, indent=2))
