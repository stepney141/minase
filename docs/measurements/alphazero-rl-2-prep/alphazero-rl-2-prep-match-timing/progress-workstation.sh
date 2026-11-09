#!/usr/bin/env bash
# 指定した2つのネットをPUCTで対局させ、新のネットを探索なしでも測る。
set -euo pipefail
if [[ $# -ne 8 ]]; then
    echo "usage: $0 <new.onnx> <old.onnx> <NnBatchSize> <pairs> <seed> <random-pairs> <random-seed> <output-dir>" >&2
    exit 2
fi
repo=$(git -C "$(dirname -- "${BASH_SOURCE[0]}")" rev-parse --show-toplevel)
new=$1
old=$2
batch=$3
pairs=$4
seed=$5
random_pairs=$6
random_seed=$7
out=$8
# shellcheck source=/dev/null
source "$repo/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
minase=$repo/target/release/minase
if [[ "$minase" == *[[:space:]]* || "$new" == *[[:space:]]* || "$old" == *[[:space:]]* ]]; then
    echo "engine command paths must contain no whitespace" >&2
    exit 1
fi
new_name=$(basename -- "$new" .onnx)
old_name=$(basename -- "$old" .onnx)
engine() {
    printf '%s --protocol usi --rules engine-default --search mcts --nn-model %s' "$minase" "$1"
}
for network in "$new" "$old"; do
    printf 'usi\nsetoption name NnBatchSize value %s\nisready\nquit\n' "$batch" |
        "$minase" --protocol usi --rules engine-default --search mcts --nn-model "$network" > /dev/null
done
mkdir -p -- "$out"
match_dir=$out/$new_name-vs-$old_name
date -Is
"$minase" match run --run-dir "$match_dir" --seed "$seed" \
    --candidate "$(engine "$new")" --baseline "$(engine "$old")" \
    --candidate-option "NnBatchSize=$batch" --baseline-option "NnBatchSize=$batch" \
    --each nodes=800 --concurrency 2 elo --pairs "$pairs" > "$match_dir.out" 2>&1
date -Is
if [[ "$random_pairs" != 0 ]]; then
    match_dir=$out/$new_name-nodes1-vs-random
    date -Is
    "$minase" match run --run-dir "$match_dir" --seed "$random_seed" \
        --candidate "$(engine "$new")" --baseline random --candidate-option "NnBatchSize=$batch" \
        --each nodes=1 --concurrency 2 elo --pairs "$random_pairs" > "$match_dir.out" 2>&1
    date -Is
fi
echo progress-done
