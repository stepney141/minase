#!/usr/bin/env bash
# Throwaway: compare zero self-play chunks of master and the branch on the workstation GPU.
# usage: selfplay.sh <binary> <out> <threads> [extra args...]
set -euo pipefail
source /home/stepney141/board-games/minase/tools/train/zero/env.sh
S=${S:?S must name the work directory holding model.onnx, init-checkpoint.pt and data/}
export XDG_CACHE_HOME=$S/cache
binary=$1; out=$2; threads=$3; shift 3
mkdir -p "$XDG_CACHE_HOME"
/usr/bin/time -v "$binary" data zero-selfplay --output-dir "$out" --nn-model "$S/model.onnx" \
  --nn-backend tensorrt --generation-commit 0123456789abcdef0123456789abcdef01234567 \
  --seed 777000 --start-position shuffled-setup --games 8 --target-positions 1000000 \
  --chunk-games 4 --simulations 32 --playthrough-percent 100 \
  --threads "$threads" --games-per-thread 4 "$@"
