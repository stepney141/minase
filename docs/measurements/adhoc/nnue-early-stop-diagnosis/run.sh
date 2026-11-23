#!/usr/bin/env bash
# usage: run.sh <name> <first-book> <last-book> <batch> <B> <S> <schedule> <seed> [extra args...]
set -euo pipefail
root=/home/stepney141/board-games/minase/data/nnue-generation-iteration
gen=${GEN:-1}
name=$1; first=$2; last=$3; batch=$4; B=$5; S=$6; sched=$7; seed=$8; shift 8
books=()
for i in $(seq "$first" "$last"); do books+=("$root/gen-$gen/teacher/book-$i.mnsd"); done
/usr/bin/time -v "$root/bin/minase-trainer" nnue train layerstack --data "${books[@]}" --rules lishogi \
  --teacher-cp-per-logit 733.1159421542734 --lambda 1.0 --seed "$seed" --threads 16 \
  --output-dir "$(dirname "$0")/$name" --batch "$batch" --superbatches "$S" \
  --validation-superbatches 1 --device cuda --lr-schedule "$sched" \
  --batches-per-superbatch "$B" "$@" > "$(dirname "$0")/$name.log" 2>&1
