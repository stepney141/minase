#!/usr/bin/env bash
# 世代反復の教師ファイルを、中断可能インスタンスで生成する制御スクリプトを起動する。
# 使い方: generate-teachers.sh <世代N> <実行ディレクトリの名前> <最初の番号> <最後の番号> \
#           <教師の生成に使うネット> <UTCの絶対期限> <累計費用の上限> <借りる回数の上限> <時間単価の上限>
# 実行ディレクトリは data/nnue-generation-iteration/<名前> であり、同じ引数で起動し直すと続きを生成する。
# 番号iの教師ファイルのシードは 9,000,000,000 + 100,000,000×N + 100,000×i である。
set -euo pipefail
generation=$1
name=$2
first=$3
last=$4
network=$5
deadline=$6
max_cost=$7
max_rentals=$8
max_price=$9
D=/home/stepney141/board-games/minase/data/nnue-generation-iteration
W=/home/stepney141/board-games/minase/data/worktrees/nnue-generation-iteration
commit=$(cat "$D/iteration-commit")
exec systemd-run --user --unit="nnue-generation-iteration-$name" --collect \
  --setenv=PATH="$PATH" -p WorkingDirectory="$W" \
  -p StandardOutput=append:"$D/$name.log" -p StandardError=append:"$D/$name.log" \
  python3 scripts/vast/nnue/teacher_generation.py run --run-dir "$D/$name" \
  --repo "$W" --ref refs/nnue-generation-iteration/iteration --commit "$commit" \
  --minase-bin "$D/bin/minase" --eval-file "$network" \
  --generation "$generation" --first-book "$first" --last-book "$last" \
  --seed-base $((9000000000 + 100000000 * generation)) --seed-book-step 100000 \
  --games 7500 --nodes 20000 --random-moves 0 --max-ply 4000 --hash-mb 16 --chunk-games 250 \
  --rental interruptible --max-price "$max_price" --deadline "$deadline" \
  --max-cost "$max_cost" --max-rentals "$max_rentals"
