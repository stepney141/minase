#!/usr/bin/env bash
# 第1世代の教師43本を、現行の条件または候補の条件で、作業機で学ぶ。
# 使い方: train.sh <学習器の実行ファイル> <出力先のディレクトリ> <current|candidate>
# 1 superbatchの更新回数は、訓練局面94,685,720から求めたB＝ceil(floor(94,685,720÷16,384)÷2)＝2,890である。
# 現行の条件の減衰の期間は390,656÷2,890 superbatch、上限はその切り上げの136 superbatchである。
set -euo pipefail
trainer=$1
output=$2
condition=$3
teacher=/home/stepney141/board-games/minase/data/nnue-generation-iteration/gen-1/teacher
books=()
for i in $(seq 0 42); do books+=("$teacher/book-$i.mnsd"); done
case $condition in
  current)
    recipe=(--superbatches 136 --lr-schedule cosine:135.1750865051903:0.027 --validation-patience 2) ;;
  candidate)
    recipe=(--superbatches 8 --lr-schedule cosine:8:0.027) ;;
  *)
    echo "current か candidate を指定する" >&2
    exit 2 ;;
esac
mkdir -p "$output"
/usr/bin/time -v "$trainer" nnue train layerstack --data "${books[@]}" --rules lishogi \
  --teacher-cp-per-logit 733.1159421542734 --lambda 1.0 --seed 1 --threads 16 \
  --output-dir "$output/gen1-$condition" --batch 16384 --batches-per-superbatch 2890 \
  --validation-superbatches 1 --device cuda "${recipe[@]}" > "$output/gen1-$condition.log" 2>&1
