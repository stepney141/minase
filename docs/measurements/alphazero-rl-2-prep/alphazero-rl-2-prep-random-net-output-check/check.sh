#!/usr/bin/env bash
# CPUとTensorRTの出力を、指定したPyTorchの出力と既定の許容誤差で照合する。
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
started=$SECONDS
for batch in 1 7 256; do
    "$repo/target/release/minase" dev nn-eval --nn-model "$network" --backend cpu \
        --batch "$batch" --out "$out/cpu-b$batch.json" || exit 1
    "$repo/target/release/minase" dev nn-eval --nn-model "$network" --backend tensorrt \
        --cache-dir "$out/trt-cache" --batch "$batch" --out "$out/trt-b$batch.json" || exit 1
done
failed=0
for batch in 1 7 256; do
    echo "== cpu batch $batch"
    uv run --frozen python "$repo/tools/train/zero/nn_compare.py" \
        --reference "$reference" --candidate "$out/cpu-b$batch.json" \
        --policy logits --policy-tolerance 1e-4 --value-tolerance 1e-4 || failed=1
    echo "== tensorrt batch $batch"
    uv run --frozen python "$repo/tools/train/zero/nn_compare.py" \
        --reference "$reference" --candidate "$out/trt-b$batch.json" \
        --policy probabilities --policy-tolerance 1e-2 --value-tolerance 5e-2 || failed=1
done
echo "elapsed_seconds $((SECONDS - started))"
echo "failed $failed"
exit "$failed"
