#!/usr/bin/env bash
# αβ戦の局面から、サイクル32のネット同士をPUCTで指し継がせる（bashで実行する）。
# 使い方: run-continuation.sh <一時的なハーネスのバイナリ> <シード> <ペア数>
# 同じネット同士が同じ探索回数なら先後を入れ替えた2局が同じ棋譜になるので、基準側を801回にして分岐させる。
set -euo pipefail
runner=$1; seed=$2; pairs=$3
R=/home/stepney141/board-games/minase
source "$R/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
export MINASE_DIAG_OPENINGS=$R/docs/measurements/adhoc/alphazero-rl-c0032-continuation/openings.txt
export MINASE_DIAG_BASE_SEED=$seed
N=/mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx
C=45d5db5498e1739b2307b3719891a1eec24f7fbd
run=data/matches/alphazero-rl-c0032-continuation
cd "$R"
printf 'usi\nsetoption name NnBatchSize value 16\nisready\nquit\n' | "target/match-cache/$C/minase" --protocol usi --rules engine-default --search mcts --nn-model "$N" > /dev/null
date -Is
if [ -d "$run" ]; then
    "$runner" match run --resume "$run" --target-pairs "$pairs"
else
    "$runner" match run --run-dir "$run" --seed "$seed" \
        --candidate "commit:$C" --baseline "commit:$C" \
        --candidate-args "--search mcts --nn-model $N" --baseline-args "--search mcts --nn-model $N" \
        --candidate-option NnBatchSize=16 --baseline-option NnBatchSize=16 \
        --each nodes=800 --baseline-limit nodes=801 --concurrency 2 elo --pairs "$pairs"
fi
date -Is
