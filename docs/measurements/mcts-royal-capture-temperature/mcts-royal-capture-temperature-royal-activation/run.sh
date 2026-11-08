#!/usr/bin/env bash
# 王駒の捕獲で負けた局の最後のMCTSの手番を、候補のバイナリで探索し直す（bashで実行する）。
# 使い方: run.sh <minaseのバイナリ> <出力ファイル>
set -euo pipefail
binary=$1; out=$2
R=/home/stepney141/board-games/minase
source "$R/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
S=$R/docs/measurements/adhoc/alphazero-rl-c0032-value-bias
cd "$S"
date -Is > "$out"
python3 proofs_and_hanging.py "$binary" /mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx "$R/data/matches/alphazero-rl-restart-stc-c0032" >> "$out" 2>&1
date -Is >> "$out"
