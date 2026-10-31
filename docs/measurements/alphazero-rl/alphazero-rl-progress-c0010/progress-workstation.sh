#!/usr/bin/env bash
# 作業機での進捗測定：サイクル<新>とサイクル<旧>のネットをPUCTの1手800シミュレーションで固定200ペア、
# 続けて探索なし（nodes=1）のMCTSを乱択エンジンrandomと固定シードで100ペア（<新>とサイクル0のネット）対局させる。
# 使い方: progress-workstation.sh <新（4桁）> <旧（4桁）> <NnBatchSize> <対局のシード> <nodes=1の対局のシード> <出力先>
set -euo pipefail
new=$1; old=$2; batch=$3; seed=$4; random_seed=$5; out=$6
W=/home/stepney141/board-games/minase/data/worktrees/alphazero-rl
source "$W/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
M=$W/target/release/minase
N=/mnt/synology_nas/minase/alphazero/nets
engine() { printf '%s' "$M --protocol usi --rules engine-default --search mcts --nn-model $N/cycle-$1.onnx"; }
for cycle in "$new" "$old" 0000; do
    printf 'usi\nsetoption name NnBatchSize value %s\nisready\nquit\n' "$batch" | $(engine "$cycle") > /dev/null
done
mkdir -p "$out"
date -Is
"$M" match run --run-dir "$out/c$new-vs-c$old" --seed "$seed" \
    --candidate "$(engine "$new")" --baseline "$(engine "$old")" \
    --candidate-option "NnBatchSize=$batch" --baseline-option "NnBatchSize=$batch" \
    --each nodes=800 --concurrency 2 elo --pairs 200 > "$out/c$new-vs-c$old.out" 2>&1
date -Is
for cycle in "$new" 0000; do
    "$M" match run --run-dir "$out/c$cycle-nodes1-vs-random" --seed "$random_seed" \
        --candidate "$(engine "$cycle")" --baseline random --candidate-option "NnBatchSize=$batch" \
        --each nodes=1 --concurrency 2 elo --pairs 100 > "$out/c$cycle-nodes1-vs-random.out" 2>&1
    date -Is
done
echo progress-done
