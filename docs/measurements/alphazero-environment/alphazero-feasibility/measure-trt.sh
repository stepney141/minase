#!/usr/bin/env bash
# alphazero-feasibility のTensorRTの推論速度を測る。measure.sh が呼び、TensorRTの段階だけを測り直すときは単独でも使う。
# 使い方: measure-trt.sh <展開したソースのルート> <measure.sh の出力ディレクトリ（nets/ を含む）>
set -euo pipefail

SRC=$1
OUT=$2
source "$SRC/tools/train/zero/env.sh"
BENCH=$SRC/tools/train/zero/feasibility/target/release/trt_bench

trt() {
    local net=$1 io=$2 fp16=$3
    "$BENCH" --model "$OUT/nets/zero-$net.onnx" --provider tensorrt --io "$io" --fp16 "$fp16" \
        --batches 64,128,256,512 --seconds 10 --warmup 20 \
        --cache-dir "$OUT/trtcache-$net-$io-$fp16" --detailed-build-log off \
        >> "$OUT/trt.jsonl" 2>> "$OUT/trt-$net-$io-$fp16.log"
}
for net in b6c96 b10c128 b15c192 b20c256; do
    trt "$net" copy on
    trt "$net" binding on
done
trt b10c128 copy off
trt b10c128 binding off

# FP16で動いていることを、エンジンの層の精度の記録で確かめる。
"$BENCH" --model "$OUT/nets/zero-b10c128.onnx" --provider tensorrt --io copy --fp16 on \
    --batches 256 --seconds 2 --warmup 1 --cache-dir "$OUT/trtcache-verbose" --detailed-build-log on \
    > "$OUT/trt-verbose.jsonl" 2> "$OUT/trt-verbose.log"
awk '/Engine Layer Information/{f=1;next} f && /^Layer\(/' "$OUT/trt-verbose.log" > "$OUT/trt-verbose-layers.txt"
