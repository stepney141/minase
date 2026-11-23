#!/usr/bin/env bash
# 第N世代を1つの条件で、作業機にある教師ファイルから学習機で学ぶ連続生成スクリプトを起動する。
# 使い方: train-trial.sh <世代N> <current|candidate> <1世代の本数> <B> <S> <lr-scheduleの値> <学習のスレッド数> \
#           <学習待ちの上限（時間）> <時間単価の上限> <CPUスレッド数の下限> [--offer <学習機の出品ID>]
# B、S、lr-scheduleの値は、同じディレクトリの recipe_values.py で求める。
# 実行ディレクトリは prepare-trial.sh が作った data/nnue-decay-to-end/gen<N>-<条件> である。
# 教師の生成器とヘッダの照合には世代反復の反復のコミットを、学習機でのビルドと学習には
# 参照 refs/nnue-decay-to-end/trainer が指す学習器のコミットを使う。
# 現行の条件（current）だけ --validation-patience 2 を渡す。
# --offerを外して同じ引数で起動し直すと、借りた学習機での続きから再開する。
set -euo pipefail
generation=$1
condition=$2
books=$3
interval=$4
superbatches=$5
schedule=$6
threads=$7
wait_hours=$8
max_price=$9
min_cores=${10}
shift 10
R=/home/stepney141/board-games/minase
S=$R/data/nnue-generation-iteration
W=$R/data/worktrees/nnue-generation-iteration
T=$R/data/nnue-decay-to-end/gen$generation-$condition
commit=$(cat "$S/iteration-commit")
trainer_ref=refs/nnue-decay-to-end/trainer
trainer_commit=$(git -C "$W" rev-parse --verify "$trainer_ref^{commit}")
case $condition in
  current) patience=(--validation-patience 2) ;;
  candidate) patience=() ;;
  *) echo "current か candidate を指定する" >&2; exit 2 ;;
esac
exec systemd-run --user --unit="nnue-decay-to-end-gen$generation-$condition-train" --collect -p Nice=19 \
  --setenv=PATH="$PATH" -p WorkingDirectory="$W" \
  -p StandardOutput=append:"$T.log" -p StandardError=append:"$T.log" \
  python3 scripts/vast/nnue/generation_chain.py run --run-dir "$T" --repo "$W" \
  --gen0-network /mnt/synology_nas/minase/nnue/nets/b2a0a450106bb3311f8ffe94f3bee17501347dabe93ecbc499827c66e07f0445.bin \
  --network-store /mnt/synology_nas/minase/nnue/nets \
  --minase-bin "$S/bin/minase" --trainer-bin "$R/data/nnue-decay-to-end/minase-trainer-afdc36fb" \
  --ref refs/nnue-generation-iteration/iteration --commit "$commit" \
  --trainer-ref "$trainer_ref" --trainer-commit "$trainer_commit" \
  --first-generation "$generation" --last-generation "$generation" --books-per-generation "$books" \
  --games 7500 --nodes 20000 --random-moves 0 --max-ply 4000 --hash-mb 16 --chunk-games 250 \
  --seed-base 9000000000 --seed-generation-step 100000000 --seed-book-step 100000 \
  --structure layerstack --replay-buffer-generations 1 --calibration-books 4 --lambda 1.0 \
  --batch 16384 --batches-per-superbatch "$interval" --superbatches "$superbatches" \
  --lr-schedule "$schedule" --validation-superbatches 1 "${patience[@]}" \
  --train-seed 1 --device cuda --train-threads "$threads" --train-place generator \
  --removal-sample 10000 --removal-seed 1 --bench-depth 5 \
  --image rust:1.98.0-bookworm --disk 60 --max-price "$max_price" --min-cpu-cores "$min_cores" --cpu-pattern . \
  --min-duration-days 1 --min-reliability 0.97 --min-inet-up 100 --min-disk 60 \
  --book-minutes 30 --deadline-margin 1.3 --training-wait-hours "$wait_hours" --idle-limit-hours 0.5 \
  --poll-seconds 60 --book-stall-minutes 120 --ssh-failure-minutes 15 "$@"
