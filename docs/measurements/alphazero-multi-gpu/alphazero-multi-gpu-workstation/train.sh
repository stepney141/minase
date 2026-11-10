#!/usr/bin/env bash
# Throwaway: one N=1 training cycle on the workstation GPU with 4 microbatches per update.
# usage: train.sh <tools/train/zero of the commit> <out> <commit> <checkpoint-steps>
set -euo pipefail
Z=$1; out=$2; commit=$3; steps=$4
S=${S:?S must name the work directory holding model.onnx, init-checkpoint.pt and data/}
D=$S/data
printf '["%s","%s"]\n' "$D/chunk-00000.mnsd" "$D/chunk-00000.resigned.mnsd" > "$out-data.json"
cd "$Z"
exec /usr/bin/time -v uv run --frozen torchrun --standalone --nproc-per-node 1 -m minase_zero train \
  --init "$S/init-checkpoint.pt" --data-list "$out-data.json" \
  --latest-data "$D/chunk-00000.mnsd" "$D/chunk-00000.resigned.mnsd" \
  --out-dir "$out" --cycle 1 --seed 20261101 --batch-size 1024 --micro-batch-size 256 \
  --replay-threads 8 --lr 0.04 --momentum 0.9 --weight-decay 0.0001 --clip-grad-norm 10 \
  --value-lambda 0.333 --precision bf16 --checkpoint-steps "$steps" --device cuda \
  --trainer-commit "$commit"
