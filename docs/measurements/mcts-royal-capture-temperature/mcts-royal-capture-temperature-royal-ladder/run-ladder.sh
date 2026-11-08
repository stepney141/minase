#!/usr/bin/env bash
# サイクル32のネットのMCTS（候補のコミット）を800回、コミット45d5db54のαβ探索を500ノードに固定して対局させる（bashで実行する）。
# 使い方: run-ladder.sh <ハーネスのバイナリ> <MCTSのコミット> <測定名> <シード> <ペア数> [MCTSのUSIオプション ...]
# USIオプションは NAME=VALUE の形で、候補側だけに与える。
set -euo pipefail
runner=$1; commit=$2; name=$3; seed=$4; pairs=$5; shift 5
R=/home/stepney141/board-games/minase
source "$R/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
N=/mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx
B=45d5db5498e1739b2307b3719891a1eec24f7fbd
options=(--candidate-option NnBatchSize=16)
for option in "$@"; do options+=(--candidate-option "$option"); done
cd "$R"
date -Is
"$runner" match run --run-dir "data/matches/$name" --seed "$seed" \
    --candidate "commit:$commit" --baseline "commit:$B" --candidate-args "--search mcts --nn-model $N" \
    "${options[@]}" --each nodes=800 --baseline-limit nodes=500 \
    --concurrency 4 elo --pairs "$pairs"
date -Is
