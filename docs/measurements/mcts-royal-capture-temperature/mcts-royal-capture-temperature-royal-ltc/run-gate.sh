#!/usr/bin/env bash
# 両側にサイクル32のネットのMCTSを使う、コミット対コミットの時間制御の対局（bashで実行する）。
# 使い方: run-gate.sh <ハーネスのバイナリ> <候補のコミット> <基準のコミット> <測定名> <シード> <時間制御> <判定の指定 ...>
# 例: run-gate.sh <ハーネス> <候補> <基準> <名前> <シード> time=10000+100 gsprt --max-pairs 3000
set -euo pipefail
runner=$1; candidate=$2; baseline=$3; name=$4; seed=$5; tc=$6; shift 6
R=/home/stepney141/board-games/minase
source "$R/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
N=/mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx
cd "$R"
date -Is
"$runner" match run --run-dir "data/matches/$name" --seed "$seed" \
    --candidate "commit:$candidate" --baseline "commit:$baseline" \
    --candidate-args "--search mcts --nn-model $N" --baseline-args "--search mcts --nn-model $N" \
    --candidate-option NnBatchSize=16 --baseline-option NnBatchSize=16 \
    --each "$tc" --concurrency 2 "$@"
date -Is
