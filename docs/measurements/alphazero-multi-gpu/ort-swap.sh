#!/usr/bin/env bash
# ONNX Runtime 1.28.3と1.31.1で、新しいキャッシュでTensorRTのエンジンを作るnn-evalと生成器を交互に実行し、終了コードを数える。
# 2つの版を交互に実行して、時間とともに変わる条件が両方に同じように効くようにする。
# 使い方: bash ort-swap.sh <env.sh> <minase> <ネット> <1.31.1のlibonnxruntime.so> <出力先> <nn-evalの回数> <生成器の回数> <キャッシュの親> <コミット40桁>
set -u
env_sh=$1 minase=$2 net=$3 ort131=$4 out=$5 reps=$6 genreps=$7 cache=$8 commit=$9
source "$env_sh"
ort128=$ORT_DYLIB_PATH
mkdir -p "$out" "$cache"
n=0
record() {  # <name> <code> <started>
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$(( ($(date +%s%N) - $3) / 1000000 ))" "$(tail -n 1 "$out/$1.err" | tr '\t' ' ')" >> "$out/exit.tsv"
}
eval_run() {  # <name> <libonnxruntime.so>
    n=$((n + 1))
    local started; started=$(date +%s%N)
    ORT_DYLIB_PATH=$2 XDG_CACHE_HOME="$cache/x$n" "$minase" dev nn-eval --nn-model "$net" --backend tensorrt --batch 1 \
        --out "$out/$1.json" --cache-dir "$cache/e$n" > "$out/$1.out" 2> "$out/$1.err"
    record "$1" $? "$started"
}
gen_run() {  # <name> <libonnxruntime.so>
    n=$((n + 1))
    local started; started=$(date +%s%N)
    ORT_DYLIB_PATH=$2 XDG_CACHE_HOME="$cache/x$n" "$minase" data zero-selfplay --output-dir "$out/$1" \
        --nn-model "$net" --nn-backend tensorrt --generation-commit "$commit" --seed $((9200000000 + n)) \
        --start-position shuffled-setup --games 8 --target-positions 1000000000 --chunk-games 8 \
        --simulations 32 --playthrough-percent 100 --threads 1 --devices 1 --games-per-thread 8 \
        > "$out/$1.out" 2> "$out/$1.err"
    record "$1" $? "$started"
}
for i in $(seq 1 "$reps"); do
    eval_run "eval-128-$i" "$ort128"
    eval_run "eval-131-$i" "$ort131"
done
for i in $(seq 1 "$genreps"); do
    gen_run "gen-128-$i" "$ort128"
    gen_run "gen-131-$i" "$ort131"
done
echo ortswap-done
