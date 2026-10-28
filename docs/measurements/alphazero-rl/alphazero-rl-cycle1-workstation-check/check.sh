#!/usr/bin/env bash
# 作業機で、学習したネットのONNX RuntimeのCPUとTensorRTのFP16の出力を、学習したColab機で
# fixture-evalが書いたPyTorchの出力（cycles/cycle-N/check/pytorch.json）と照合する（bashで実行する）。
# fixture-evalはチェックポイントからONNXを書き出し直してバイト列を比べるが、書き出しの定数畳み込みの
# 丸めはCPUによって変わるので、作業機では書き出し直さない。手順と許容誤差は
# docs/guides/alphazero-network-check.md の3節から5節による。
# 使い方: check.sh <サイクル番号（4桁）>
set -uo pipefail
cycle=$1
W=/home/stepney141/board-games/minase/data/worktrees/alphazero-rl
A=/mnt/synology_nas/minase/alphazero
TRAIN=$A/cycles/cycle-$cycle/train
OUT=$A/cycles/cycle-$cycle/check-workstation
mkdir -p "$OUT"
cd "$W/tools/train/zero"
started=$SECONDS
REFERENCE=$A/cycles/cycle-$cycle/check/pytorch.json
source "$W/tools/train/zero/env.sh"
for b in 1 7 256; do
  "$W/target/release/minase" dev nn-eval --nn-model "$TRAIN/model.onnx" --backend cpu \
    --batch "$b" --out "$OUT/cpu-b$b.json" || exit 1
  "$W/target/release/minase" dev nn-eval --nn-model "$TRAIN/model.onnx" --backend tensorrt \
    --cache-dir "$OUT/trt-cache" --batch "$b" --out "$OUT/trt-b$b.json" || exit 1
done
failed=0
for b in 1 7 256; do
  echo "== cpu batch $b"
  uv run --frozen python nn_compare.py --reference "$REFERENCE" --candidate "$OUT/cpu-b$b.json" \
    --policy logits --policy-tolerance 1e-4 --value-tolerance 1e-4 || failed=1
  echo "== tensorrt batch $b"
  uv run --frozen python nn_compare.py --reference "$REFERENCE" --candidate "$OUT/trt-b$b.json" \
    --policy probabilities --policy-tolerance 1e-2 --value-tolerance 2e-2 || failed=1
done
echo "elapsed_seconds $((SECONDS - started))"
echo "failed $failed"
exit "$failed"
