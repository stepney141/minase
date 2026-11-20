#!/usr/bin/env bash
# 学習機だけを借りて学習する連続生成スクリプトの試走。本番とは別の実行ディレクトリを使う。
# 使い方: run-trial.sh <実行ディレクトリの名前> <自己停止までの待ち時間（時間）> <反復のブランチの先頭のコミット（40桁）> --offer <出品ID>
# 1回目は run 0.05、2回目は run2 0.02 で実行した。
# 教師4本（各200局、1手2,000ノード）は、同じコミットのworktreeで先に生成し、
# $T/<実行ディレクトリの名前>/gen-1/teacher/ と gen-1/logs/ へハードリンクで置いておく。
set -euo pipefail
run=$1
idle=$2
commit=$3
shift 3
T=/home/stepney141/board-games/minase/data/nnue-generation-iteration-trial
W=/home/stepney141/board-games/minase/data/worktrees/nnue-generation-iteration
cd "$W"
exec systemd-run --user --unit=nnue-generation-iteration-trial --same-dir --collect -p Nice=19 \
  -p StandardOutput=append:$T/$run.log -p StandardError=append:$T/$run.log \
  --setenv=PATH="$PATH" \
  python3 "$W/scripts/vast/generation_chain.py" run --run-dir "$T/$run" --repo "$W" \
  --gen0-network "$T/v0.bin" --network-store /mnt/synology_nas/minase/nnue/nets \
  --minase-bin "$W/target/release/minase" --trainer-bin "$W/target/release/minase-trainer" \
  --ref nnue-generation-iteration --commit "$commit" \
  --first-generation 1 --last-generation 1 --books-per-generation 4 \
  --games 200 --nodes 2000 --random-moves 0 --max-ply 4000 --hash-mb 16 --chunk-games 50 \
  --seed-base 19000000000 --seed-generation-step 100000000 --seed-book-step 100000 \
  --structure layerstack --replay-buffer-generations 1 --calibration-books 4 --lambda 1.0 \
  --batch 16384 --batches-per-superbatch 6 --superbatches 20 --lr-schedule cosine:20:0.027 \
  --validation-superbatches 1 --validation-patience 2 \
  --train-seed 1 --device cuda --train-threads 8 --train-place generator \
  --removal-sample 100 --removal-seed 1 --bench-depth 5 \
  --image rust:1.98.0-bookworm --disk 30 --max-price 0.15 --min-cpu-cores 8 --cpu-pattern . \
  --min-duration-days 1 --min-reliability 0.95 --min-inet-up 100 --min-disk 30 \
  --book-minutes 10 --deadline-margin 1.3 --training-wait-hours 0.75 --idle-limit-hours "$idle" \
  --poll-seconds 15 --book-stall-minutes 30 --ssh-failure-minutes 5 "$@"
