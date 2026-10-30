#!/usr/bin/env bash
# 重みの移動平均の採否：サイクル25の平均のネットを候補、平均でないネットを基準として、
# PUCTの1手800シミュレーション、同時対局数2の既定の仮説のGSPRTにかける（作業機で実行する）。
# 上限3,000ペアに達した判定保留は採らない。
set -euo pipefail
W=/home/stepney141/board-games/minase/data/worktrees/alphazero-rl
source "$W/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
M=$W/target/release/minase
N=/mnt/synology_nas/minase/alphazero/nets
engine() { printf '%s' "$M --protocol usi --rules engine-default --search mcts --nn-model $N/$1"; }
for net in cycle-0025-ema.onnx cycle-0025.onnx; do
    printf 'usi\nsetoption name NnBatchSize value 16\nisready\nquit\n' | $(engine "$net") > /dev/null
done
date -Is
"$M" match run --run-dir /mnt/synology_nas/minase/alphazero/progress/ema-c0025 --seed 30025 \
    --candidate "$(engine cycle-0025-ema.onnx)" --baseline "$(engine cycle-0025.onnx)" \
    --candidate-option NnBatchSize=16 --baseline-option NnBatchSize=16 \
    --each nodes=800 --concurrency 2 gsprt --max-pairs 3000 > /mnt/synology_nas/minase/alphazero/progress/ema-c0025.out 2>&1
date -Is
echo ema-done
