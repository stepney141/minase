#!/usr/bin/env bash
# 作業機で、サイクル32のネットのMCTSを候補、同じコミットのαβ探索を基準として、STCで固定200ペア対局させる（bashで実行する）。
set -euo pipefail
R=/home/stepney141/board-games/minase
source "$R/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
N=/mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx
C=45d5db5498e1739b2307b3719891a1eec24f7fbd
cd "$R"
printf 'usi\nsetoption name NnBatchSize value 16\nisready\nquit\n' | target/release/minase --protocol usi --rules engine-default --search mcts --nn-model "$N" > /dev/null
date -Is
target/release/minase match run --run-dir data/matches/alphazero-rl-restart-stc-c0032 --seed 2026110232 \
    --candidate "commit:$C" --baseline "commit:$C" --candidate-args "--search mcts --nn-model $N" \
    --candidate-option NnBatchSize=16 --each time=10000+100 --concurrency 1 elo --pairs 200
date -Is
echo stc-done
