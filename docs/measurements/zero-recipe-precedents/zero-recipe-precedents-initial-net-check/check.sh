#!/usr/bin/env bash
# USIのnn-evalでCPUとTensorRTのバッチ1、7、256の出力を書き、指定したPyTorchの出力と既定の許容誤差で照合する。
set -euo pipefail
if [[ $# -ne 3 ]]; then
    echo "usage: $0 <network.onnx> <pytorch.json> <output-dir>" >&2
    exit 2
fi
repo=$(git -C "$(dirname -- "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
network=$(realpath -- "$1")
reference=$(realpath -- "$2")
out=$(realpath -m -- "$3")
mkdir -p -- "$out"
cd "$repo/tools/train/zero"
# shellcheck source=/dev/null
source "$repo/tools/train/zero/env.sh"
run() {  # 実行プロバイダ、バッチの大きさ、出力のパス
    { printf 'usi\nsetoption name RuleSet value JCSA\nsetoption name NnModel value %s\n' "$network"
      printf 'setoption name NnBackend value %s\nsetoption name NnBatchSize value %s\n' "$1" "$2"
      if [[ $1 == tensorrt ]]; then printf 'setoption name NnCacheDir value %s\n' "$out/trt-cache"; fi
      printf 'isready\nnn-eval out %s\nquit\n' "$3"
    } | "$repo/target/release/minase" --protocol usi --search mcts > "$3.log" 2>&1
    grep -q '^info string end$' "$3.log"
}
started=$SECONDS
for batch in 1 7 256; do
    run cpu "$batch" "$out/cpu-b$batch.json"
    run tensorrt "$batch" "$out/trt-b$batch.json"
done
failed=0
for batch in 1 7 256; do
    echo "== cpu batch $batch"
    uv run --frozen python "$repo/tools/train/zero/nn_compare.py" \
        --reference "$reference" --candidate "$out/cpu-b$batch.json" \
        --policy fp16-logits --policy-tolerance 1e-4 --value-tolerance 1e-4 || failed=1
    echo "== tensorrt batch $batch"
    uv run --frozen python "$repo/tools/train/zero/nn_compare.py" \
        --reference "$reference" --candidate "$out/trt-b$batch.json" \
        --policy probabilities --policy-tolerance 1e-2 --value-tolerance 5e-2 || failed=1
done
echo "elapsed_seconds $((SECONDS - started))"
echo "failed $failed"
exit "$failed"
