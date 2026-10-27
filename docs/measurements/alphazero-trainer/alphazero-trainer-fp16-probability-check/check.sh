#!/usr/bin/env bash
# Checks the trained net across PyTorch, ONNX Runtime CPU, and TensorRT FP16 with the revised criteria (run with bash).
set -uo pipefail
D=/home/stepney141/board-games/minase/data/alphazero-fp16-check
W=/home/stepney141/board-games/minase/data/worktrees/alphazero-fp16-check
OUT=$D/check
mkdir -p "$OUT"
cd "$W/tools/train/zero"
uv run --frozen python -m minase_zero fixture-eval --checkpoint "$D/train-a/checkpoint.pt" \
  --onnx "$D/train-a/model.onnx" --fixture fixtures/nn-positions.jsonl --out "$OUT/pytorch.json"
source "$W/tools/train/zero/env.sh"
for b in 1 7 256; do
  "$W/target/release/minase" dev nn-eval --nn-model "$D/train-a/model.onnx" --backend cpu \
    --batch "$b" --out "$OUT/cpu-b$b.json"
  "$W/target/release/minase" dev nn-eval --nn-model "$D/train-a/model.onnx" --backend tensorrt \
    --cache-dir "$OUT/trt-cache" --batch "$b" --out "$OUT/trt-b$b.json"
done
for b in 1 7 256; do
  echo "== cpu batch $b"
  uv run --frozen python nn_compare.py --reference "$OUT/pytorch.json" --candidate "$OUT/cpu-b$b.json" \
    --policy logits --policy-tolerance 1e-4 --value-tolerance 1e-4
  echo "exit $?"
  echo "== tensorrt batch $b"
  uv run --frozen python nn_compare.py --reference "$OUT/pytorch.json" --candidate "$OUT/trt-b$b.json" \
    --policy probabilities --policy-tolerance 1e-2 --value-tolerance 2e-2
  echo "exit $?"
done
