#!/usr/bin/env bash
# Generates the 1,000 self-play games used by the GPU training check (run with bash).
set -euo pipefail
D=/home/stepney141/board-games/minase/data/alphazero-trainer
W=/home/stepney141/board-games/minase/data/worktrees/alphazero-trainer
cd "$W/tools/train/zero"
uv run --frozen python -m minase_zero init --blocks 10 --channels 128 --seed 20261028 \
  --out-dir "$D/cycle-0000" --trainer-commit "$(cat "$D/trainer-commit.txt")"
source "$W/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
nice -n 10 "$W/target/release/minase" data zero-selfplay --output-dir "$D/selfplay-c1" \
  --nn-model "$D/cycle-0000/model.onnx" --nn-backend tensorrt \
  --generation-commit ae6a3723a82bc2ccf28c033f0659358d88e95e45 \
  --seed 20261100 --games 1000 --chunk-games 250 --simulations 32 \
  --playthrough-percent 100 --threads 2 --games-per-thread 64
