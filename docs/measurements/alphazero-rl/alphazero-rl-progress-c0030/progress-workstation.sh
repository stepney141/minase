#!/usr/bin/env bash
# 作業機での進捗測定：サイクル<新>とサイクル<旧>のネットをPUCTの1手800シミュレーションで固定200ペア、
# 乱択との対局は最初の進捗測定でだけ行うので含めない。
# 使い方: progress-workstation.sh <新（4桁）> <旧（4桁）> <NnBatchSize> <対局のシード> <出力先>
set -euo pipefail
new=$1; old=$2; batch=$3; seed=$4; out=$5
W=/home/stepney141/board-games/minase/data/worktrees/alphazero-rl
source "$W/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
M=$W/target/release/minase
N=/mnt/synology_nas/minase/alphazero/nets
engine() { printf '%s' "$M --protocol usi --rules engine-default --search mcts --nn-model $N/cycle-$1.onnx"; }
for cycle in "$new" "$old"; do
    printf 'usi\nsetoption name NnBatchSize value %s\nisready\nquit\n' "$batch" | $(engine "$cycle") > /dev/null
done
mkdir -p "$out"
date -Is
"$M" match run --run-dir "$out/c$new-vs-c$old" --seed "$seed" \
    --candidate "$(engine "$new")" --baseline "$(engine "$old")" \
    --candidate-option "NnBatchSize=$batch" --baseline-option "NnBatchSize=$batch" \
    --each nodes=800 --concurrency 2 elo --pairs 200 > "$out/c$new-vs-c$old.out" 2>&1
date -Is
echo progress-done
