#!/usr/bin/env bash
# 新しい構成のランダム初期化のネットを、PyTorch、ONNX RuntimeのCPU、およびTensorRTのFP16で照合する。
# 使い方: bash run.sh <リポジトリの根> <出力先>
set -euo pipefail
W=$1; O=$2
N=$O/cycle-0000/model.onnx
source "$W/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$O/xdg-cache
mkdir -p "$O/out" "$O/trt-cache"
cd "$W/tools/train/zero"
[ -e "$N" ] || uv run --frozen python -m minase_zero init --blocks 10 --channels 128 --seed 20261031 \
  --out-dir "$O/cycle-0000" --trainer-commit "$(git rev-parse HEAD)"
sha256sum "$N"
uv run --frozen python -m minase_zero fixture-eval --checkpoint "$O/cycle-0000/checkpoint.pt" --onnx "$N" \
  --fixture fixtures/nn-positions.jsonl --weights raw --out "$O/out/pytorch.json"
for b in 1 7 256; do
  "$W/target/release/minase" dev nn-eval --nn-model "$N" --backend cpu --batch $b --out "$O/out/cpu-b$b.json"
  "$W/target/release/minase" dev nn-eval --nn-model "$N" --backend tensorrt --cache-dir "$O/trt-cache" --batch $b --out "$O/out/trt-b$b.json"
done
for b in 1 7 256; do
  echo "cpu-b$b"; uv run --frozen python nn_compare.py --reference "$O/out/pytorch.json" --candidate "$O/out/cpu-b$b.json" \
    --policy logits --policy-tolerance 1e-4 --value-tolerance 1e-4
  echo "trt-b$b"; uv run --frozen python nn_compare.py --reference "$O/out/pytorch.json" --candidate "$O/out/trt-b$b.json" \
    --policy probabilities --policy-tolerance 1e-2 --value-tolerance 5e-2
done
