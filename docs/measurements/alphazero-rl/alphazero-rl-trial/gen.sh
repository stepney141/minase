#!/usr/bin/env bash
# /content/az を作業ディレクトリとして自己対局を生成し、標準エラーの各行に時刻を付け、
# 30秒ごとにGPUとCPUの使用率を記録する。
# 使い方: gen.sh <出力先> <ネット> <生成コミット> <基本シード> <対局数> <塊の対局数> <スレッド数> <スレッドごとの同時対局数> <最後まで指させる割合>
set -euo pipefail
source /content/src/tools/train/zero/env.sh
export XDG_CACHE_HOME=/content/cache
cd /content/az
mkdir -p "$(dirname "$1")"
(
    while true; do
        printf '%s gpu %s cpu %s\n' "$(date +%s)" \
            "$(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader)" \
            "$(top -bn1 | sed -n 3p)"
        sleep 30
    done
) > "$1.monitor" 2>&1 &
monitor=$!
trap 'kill $monitor' EXIT
date +%s.%N > "$1.start"
/content/src/target/release/minase data zero-selfplay --output-dir "$1" \
    --nn-model "$2" --nn-backend tensorrt --generation-commit "$3" \
    --seed "$4" --games "$5" --chunk-games "$6" --simulations 32 \
    --playthrough-percent "$9" --threads "$7" --games-per-thread "$8" \
    > "$1.out" 2> >(while IFS= read -r line; do printf '%s %s\n' "$(date +%s.%N)" "$line"; done > "$1.err")
date +%s.%N > "$1.end"
echo gen-done
