#!/usr/bin/env bash
# Generates 1,000 self-play games with the init net (run with bash).
set -euo pipefail
D=/home/stepney141/board-games/minase/data/alphazero-fp16-check
W=/home/stepney141/board-games/minase/data/worktrees/alphazero-fp16-check
source "$W/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
nice -n 10 "$W/target/release/minase" data zero-selfplay --output-dir "$D/selfplay-c1" \
  --nn-model "$D/cycle-0000/model.onnx" --nn-backend tensorrt \
  --generation-commit "$(cat "$D/trainer-commit.txt")" \
  --seed 20261100 --games 1000 --chunk-games 250 --simulations 32 \
  --playthrough-percent 100 --threads 2 --games-per-thread 64
