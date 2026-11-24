#!/usr/bin/env bash
# 世代反復の第3世代以降の1世代を、作業機にある教師ファイルから、学習機で標準レシピv4で学ぶ連続生成スクリプトを起動する。
# 使い方: train-generation.sh <世代N> <1世代の本数kN> <B> <学習のスレッド数> \
#           <学習待ちの上限（時間）> <時間単価の上限> <CPUスレッド数の下限> [--offer <学習機の出品ID>]
# Bは、同じディレクトリの recipe_values.py で求める。
# 実行ディレクトリは data/nnue-generation-iteration であり、教師ファイルと生成ログを
# gen-<N>/teacher/ と gen-<N>/logs/ へハードリンクで置いてから起動する。
# 教師の生成器とヘッダの照合には反復のコミットを、学習機でのビルドと学習には
# 参照 refs/nnue-decay-to-end/trainer が指す学習器のコミットを使う。
# 学習の長さと学習率の予定は --superbatches 8 と cosine:8:0.027 であり、--validation-patience は渡さない。
# --offerを外して同じ引数で起動し直すと、借りた学習機での続きから再開する。
set -euo pipefail
generation=$1
books=$2
interval=$3
threads=$4
wait_hours=$5
max_price=$6
min_cores=$7
shift 7
R=/home/stepney141/board-games/minase
D=$R/data/nnue-generation-iteration
W=$R/data/worktrees/nnue-generation-iteration
commit=$(cat "$D/iteration-commit")
trainer_ref=refs/nnue-decay-to-end/trainer
trainer_commit=$(git -C "$W" rev-parse --verify "$trainer_ref^{commit}")
exec systemd-run --user --unit="nnue-generation-iteration-gen$generation-train" --collect -p Nice=19 \
  --setenv=PATH="$PATH" -p WorkingDirectory="$W" \
  -p StandardOutput=append:"$D/gen$generation-train.log" -p StandardError=append:"$D/gen$generation-train.log" \
  python3 scripts/vast/nnue/generation_chain.py run --run-dir "$D" --repo "$W" \
  --gen0-network /mnt/synology_nas/minase/nnue/nets/b2a0a450106bb3311f8ffe94f3bee17501347dabe93ecbc499827c66e07f0445.bin \
  --network-store /mnt/synology_nas/minase/nnue/nets \
  --minase-bin "$D/bin/minase" --trainer-bin "$R/data/nnue-decay-to-end/minase-trainer-afdc36fb" \
  --ref refs/nnue-generation-iteration/iteration --commit "$commit" \
  --trainer-ref "$trainer_ref" --trainer-commit "$trainer_commit" \
  --first-generation "$generation" --last-generation "$generation" --books-per-generation "$books" \
  --games 7500 --nodes 20000 --random-moves 0 --max-ply 4000 --hash-mb 16 --chunk-games 250 \
  --seed-base 9000000000 --seed-generation-step 100000000 --seed-book-step 100000 \
  --structure layerstack --replay-buffer-generations 1 --calibration-books 4 --lambda 1.0 \
  --batch 16384 --batches-per-superbatch "$interval" --superbatches 8 \
  --lr-schedule cosine:8:0.027 --validation-superbatches 1 \
  --train-seed 1 --device cuda --train-threads "$threads" --train-place generator \
  --removal-sample 10000 --removal-seed 1 --bench-depth 5 \
  --image rust:1.98.0-bookworm --disk 60 --max-price "$max_price" --min-cpu-cores "$min_cores" --cpu-pattern . \
  --min-duration-days 1 --min-reliability 0.97 --min-inet-up 100 --min-disk 60 \
  --book-minutes 30 --deadline-margin 1.3 --training-wait-hours "$wait_hours" --idle-limit-hours 0.5 \
  --poll-seconds 60 --book-stall-minutes 120 --ssh-failure-minutes 15 "$@"
