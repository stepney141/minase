#!/usr/bin/env bash
# Usage: run-train.sh <out-dir>. Runs one cycle of the zero trainer on the work machine GPU.
set -euo pipefail
D=/home/stepney141/board-games/minase/data/alphazero-trainer
cd /home/stepney141/board-games/minase/data/worktrees/alphazero-trainer/tools/train/zero
exec uv run --frozen python -m minase_zero train \
  --init "$D/cycle-0000/checkpoint.pt" \
  --data "$D"/selfplay-c1/chunk-0000{0,1,2,3}.mnsd "$D"/selfplay-c1/chunk-0000{0,1,2,3}.resigned.mnsd \
  --out-dir "$1" --cycle 1 --seed 20261101 \
  --batch-size 256 --lr 0.01 --momentum 0.9 --weight-decay 0.0001 --clip-grad-norm 10 \
  --value-lambda 0.333 --precision bf16 --checkpoint-steps 50 --device cuda \
  --trainer-commit "$(cat "$D/trainer-commit.txt")"
