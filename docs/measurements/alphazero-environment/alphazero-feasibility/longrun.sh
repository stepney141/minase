#!/usr/bin/env bash
# Colab機の連続実行の確認で、生成に似た負荷（GPUの推論とCPUの探索）を止まるまで走らせ、10分ごとに時刻を記録する。
# 使い方: longrun.sh <measure.sh を実行したソースのルート> <measure.sh の出力ディレクトリ> <教師データのMNSD> <CPUの探索の並列数> <記録先>
set -euo pipefail

SRC=$1
MEASURED=$2
TEACHER=$3
WORKERS=$4
OUT=$5
mkdir -p "$OUT"
source "$SRC/tools/train/zero/env.sh"
BIN=$SRC/tools/train/zero/feasibility/target/release

(
    while true; do
        "$BIN/trt_bench" --model "$MEASURED/nets/zero-b10c128.onnx" --provider tensorrt --io copy --fp16 on \
            --batches 256 --seconds 600 --warmup 5 --cache-dir "$MEASURED/trtcache-b10c128-copy-on" \
            --detailed-build-log off >> "$OUT/trt.jsonl" 2>> "$OUT/trt.log"
    done
) &
for i in $(seq 1 "$WORKERS"); do
    (
        while true; do
            "$BIN/sim_cost" --data "$TEACHER" --positions 2000 --stride 1709 --simulations 32 --considered 16 \
                --seed "$i" --breakdown off >> "$OUT/sim-$i.jsonl"
        done
    ) &
done
while true; do
    date -Is >> "$OUT/heartbeat.log"
    sleep 600
done
