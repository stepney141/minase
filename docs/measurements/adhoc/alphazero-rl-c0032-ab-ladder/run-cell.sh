#!/usr/bin/env bash
# サイクル32のネットのMCTSを固定シミュレーション数、αβ探索を固定ノード数で対局させる（bashで実行する）。
# 使い方: run-cell.sh <ハーネスのバイナリ> <MCTSのシミュレーション数> <αβのノード数> <シード> <ペア数>
# 同じ条件の実行ディレクトリがあれば、ペア数まで延長する。
set -euo pipefail
runner=$1; mcts=$2; ab=$3; seed=$4; pairs=$5
R=/home/stepney141/board-games/minase
source "$R/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
N=/mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx
C=45d5db5498e1739b2307b3719891a1eec24f7fbd
run=data/matches/alphazero-rl-c0032-ab-ladder-m$mcts-a$ab
cd "$R"
printf 'usi\nsetoption name NnBatchSize value 16\nisready\nquit\n' | "target/match-cache/$C/minase" --protocol usi --rules engine-default --search mcts --nn-model "$N" > /dev/null
date -Is
if [ -d "$run" ]; then
    "$runner" match run --resume "$run" --target-pairs "$pairs"
else
    "$runner" match run --run-dir "$run" --seed "$seed" \
        --candidate "commit:$C" --baseline "commit:$C" --candidate-args "--search mcts --nn-model $N" \
        --candidate-option NnBatchSize=16 --each "nodes=$mcts" --baseline-limit "nodes=$ab" \
        --concurrency 4 elo --pairs "$pairs"
fi
date -Is
