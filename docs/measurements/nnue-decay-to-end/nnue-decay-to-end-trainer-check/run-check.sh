#!/usr/bin/env bash
# 学習器の修正の確認に使う学習を、作業機で1本ずつ実行する。
# 使い方: run-check.sh <学習器の実行ファイル> <出力先のディレクトリ> <equivalence|tail>
#   equivalence: 第1世代の教師43本を、診断の測定のA-baseと同じ引数で3 superbatch学ぶ。
#   tail:        教師の番号0〜3の4本を、--batchを16432、16281、16001に変えて学ぶ（端数が172、5、8,036局面になる値）。
# 教師は世代反復の実行ディレクトリ data/nnue-generation-iteration/gen-1/teacher/ にある。
set -euo pipefail
trainer=$1
output=$2
mode=$3
teacher=/home/stepney141/board-games/minase/data/nnue-generation-iteration/gen-1/teacher
mkdir -p "$output"

train() {
  local name=$1 last=$2 batch=$3 interval=$4 superbatches=$5 schedule=$6
  local books=()
  for i in $(seq 0 "$last"); do books+=("$teacher/book-$i.mnsd"); done
  "$trainer" nnue train layerstack --data "${books[@]}" --rules lishogi \
    --teacher-cp-per-logit 733.1159421542734 --lambda 1.0 --seed 1 --threads 16 \
    --output-dir "$output/$name" --batch "$batch" --superbatches "$superbatches" \
    --validation-superbatches 1 --device cuda --lr-schedule "$schedule" \
    --batches-per-superbatch "$interval" > "$output/$name.log" 2>&1
}

case $mode in
  equivalence)
    train equivalence 42 16384 963 3 cosine:405.66562824506747:0.027 ;;
  tail)
    train tail-16432 3 16432 4 280 constant:1
    train tail-16281 3 16281 4 284 constant:1
    train tail-16001 3 16001 4 290 constant:1 ;;
  *)
    echo "equivalence か tail を指定する" >&2
    exit 2 ;;
esac
