#!/usr/bin/env bash
# 第N世代を1つの条件で試す、連続生成スクリプトの試行用の実行ディレクトリを作る。
# 使い方: prepare-trial.sh <世代N> <current|candidate>
# 世代反復の実行ディレクトリ data/nnue-generation-iteration から、前の世代までの記録と較正の結果を写し、
# 第N世代の本と生成ログをハードリンクで置く。試行用の実行ディレクトリは
# data/nnue-decay-to-end/gen<N>-<条件> であり、すでにあれば止まる。
set -euo pipefail
generation=$1
condition=$2
case $condition in current|candidate) ;; *) echo "current か candidate を指定する" >&2; exit 2 ;; esac
R=/home/stepney141/board-games/minase
S=$R/data/nnue-generation-iteration
T=$R/data/nnue-decay-to-end/gen$generation-$condition
if [ -e "$T" ]; then echo "すでにあります: $T" >&2; exit 1; fi
mkdir -p "$T"
cp "$S/calibration.json" "$T/calibration.json"
for previous in $(seq 0 $((generation - 1))); do
  mkdir -p "$T/gen-$previous"
  cp "$S/gen-$previous/generation.json" "$T/gen-$previous/generation.json"
done
mkdir -p "$T/gen-$generation/teacher" "$T/gen-$generation/logs"
ln "$S/gen-$generation/teacher"/book-*.mnsd "$T/gen-$generation/teacher/"
ln "$S/gen-$generation/logs"/book-*.log "$T/gen-$generation/logs/"
echo "$T: 本 $(ls "$T/gen-$generation/teacher" | wc -l)、生成ログ $(ls "$T/gen-$generation/logs" | wc -l)"
