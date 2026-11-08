#!/usr/bin/env bash
# 基準と候補のバイナリで、STCの対局の同じ局面の毎秒の探索回数を交互に測る（bashで実行する）。
# 使い方: run.sh <基準のバイナリ> <候補のバイナリ> <回数> <出力ファイル>
set -euo pipefail
base=$1; candidate=$2; rounds=$3; out=$4
R=/home/stepney141/board-games/minase
source "$R/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$HOME/.cache
S=$R/docs/measurements/adhoc/alphazero-rl-c0032-value-bias
N=/mnt/synology_nas/minase/alphazero/nets/cycle-0032.onnx
cd "$S"
: > "$out"
for round in $(seq 1 "$rounds"); do
    for side in base candidate; do
        binary=$base; [ "$side" = candidate ] && binary=$candidate
        echo "round $round $side" >> "$out"
        python3 simulation_rate.py "$binary" "$N" "$R/data/matches/alphazero-rl-restart-stc-c0032" 16 >> "$out" 2>&1
    done
done
