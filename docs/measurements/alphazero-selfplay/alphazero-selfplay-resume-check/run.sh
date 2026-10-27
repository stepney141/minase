#!/usr/bin/env bash
# フェーズ2の再開の確認に使う生成コマンド。同じ引数で2回起動する。
set -euo pipefail
source /home/stepney141/board-games/minase/data/worktrees/alphazero-network/tools/train/zero/env.sh
export XDG_CACHE_HOME=/home/stepney141/board-games/minase/data/alphazero-selfplay/cache
WT=/home/stepney141/board-games/minase/data/worktrees/alphazero-selfplay
BIN=/home/stepney141/board-games/minase/data/alphazero-selfplay/target/release/minase
exec nice -n 10 "$BIN" data zero-selfplay \
  --output-dir /home/stepney141/board-games/minase/data/alphazero-selfplay/resume-check-final \
  --nn-model /home/stepney141/board-games/minase/data/alphazero-network/nets/zero-b10c128.onnx \
  --nn-backend tensorrt \
  --generation-commit "$(git -C "$WT" rev-parse HEAD)" \
  --seed 20261027 --games 24 --chunk-games 8 --simulations 32 \
  --playthrough-percent 100 --threads 2 --games-per-thread 8
