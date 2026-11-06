#!/usr/bin/env bash
# 標準レシピv1の連続生成スクリプトの試走。使い捨てのcloneと試走専用の参照の接頭辞を使う。
# 1回目: run-trial.sh 1 --offer <ID>（第1世代を生成機で生成して学び、生成機を削除する）
# 2回目: run-trial.sh 2 --offer <ID>（新しく借りた空の生成機で第2世代を生成し、第1世代の本も送って窓2で学ぶ）
set -euo pipefail
last=$1
shift
T=/home/stepney141/board-games/minase/data/nnue-standard-recipe-trial
W=/home/stepney141/board-games/minase/data/worktrees/nnue-standard-recipe
B=/home/stepney141/board-games/minase/data/nnue-generation-reset/bin
cd "$T/clone"
exec systemd-run --user --unit=nnue-standard-recipe-trial --same-dir --collect -p MemoryMax=8G -p Nice=19 \
  -p StandardOutput=append:$T/chain.log -p StandardError=append:$T/chain.log \
  --setenv=PATH="$PATH" --setenv=CARGO_BUILD_JOBS=4 \
  python3 "$W/scripts/vast/generation_chain.py" run --run-dir "$T/run" --worktree "$T/clone" \
  --minase-bin "$B/minase" --trainer-bin "$B/minase-trainer" \
  --ref-prefix refs/nnue-standard-recipe-trial \
  --gen0-commit 6f874692fb91544c94394ca0e8df25299ef2e02d --first-generation 1 --last-generation "$last" \
  --books-per-generation 2 \
  --games 40 --nodes 500 --random-moves 0 --max-ply 4000 --hash-mb 16 \
  --seed-base 9500000000 --seed-generation-step 10000000 --seed-book-step 100000 \
  --calibration-books 1 --lambda 1.0 --feature-set P --width 256 --head pairwise-l16-l32 --batch 1024 \
  --epochs 2 --patience 5 --train-seed 1 --device cuda --validation-interval 0 --lr-schedule cosine:3:0.1 \
  --lr-linear 0.67903 --train-threads 8 --train-place generator --export-epoch best --window 2 --fit no \
  --removal-sample 100 --removal-seed 1 --bench-depth 5 \
  --image rust:1.98.0-bookworm --disk 30 --max-price 0.15 --min-cpu-cores 24 --cpu-pattern . --min-duration-days 1 \
  --min-reliability 0.95 --min-inet-up 100 --min-disk 30 \
  --book-minutes 10 --deadline-margin 1.3 --training-wait-hours 0.5 --idle-limit-hours 0.5 --poll-seconds 15 \
  --book-stall-minutes 30 --ssh-failure-minutes 5 "$@"
