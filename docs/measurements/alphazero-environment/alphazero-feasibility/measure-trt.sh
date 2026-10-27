#!/usr/bin/env bash
# alphazero-feasibility のTensorRTの推論速度を測る。measure.sh が呼び、TensorRTの段階だけを測り直すときは単独でも使う。
# 使い方: measure-trt.sh <展開したソースのルート> <measure.sh の出力ディレクトリ（nets/ を含む）>
# 出力の受け取りは、ONNX Runtimeに出力を確保させる方式（--io copy）だけを使う。
set -euo pipefail

SRC=$1
OUT=$2
source "$SRC/tools/train/zero/env.sh"
BENCH=$SRC/tools/train/zero/feasibility/target/release/trt_bench

# Colab機では、trt_bench が全バッチの結果を書いた後のセッションの破棄か終了処理で、
# glibc がヒープの破壊（corrupted double-linked list）を検出して異常終了することがある。
# 結果の行がすべて書かれていれば終了コードを trt-exit.tsv に記録して進み、足りなければ止める。
touch "$OUT/trt.jsonl"
trt() {
    local net=$1 fp16=$2 streams=$3 batches=$4 status=0
    local before after
    before=$(wc -l < "$OUT/trt.jsonl")
    "$BENCH" --model "$OUT/nets/zero-$net.onnx" --provider tensorrt --io copy --fp16 "$fp16" \
        --streams "$streams" --batches "$batches" --seconds 10 --warmup 20 \
        --cache-dir "$OUT/trtcache-$net-$fp16" --detailed-build-log off \
        >> "$OUT/trt.jsonl" 2>> "$OUT/trt-$net-$fp16-s$streams.log" || status=$?
    after=$(wc -l < "$OUT/trt.jsonl")
    if (( after - before != $(tr ',' '\n' <<< "$batches" | wc -l) )); then
        echo "trt_bench $net $fp16 s$streams stopped before writing all rows (exit $status)" >&2
        exit 1
    fi
    if (( status != 0 )); then
        printf '%s\t%s\t%s\t%s\n' "$net" "$fp16" "$streams" "$status" >> "$OUT/trt-exit.tsv"
    fi
}
for net in b6c96 b10c128 b15c192 b20c256; do
    trt "$net" on 1 64,128,256,512
done
trt b10c128 off 1 64,128,256,512
for streams in 2 4; do
    trt b10c128 on "$streams" 128,256,512
done
trt b10c128 off 4 256,512
trt b20c256 on 4 256,512

# FP16で動いていることを、エンジンの層の精度の記録で確かめる。
status=0
"$BENCH" --model "$OUT/nets/zero-b10c128.onnx" --provider tensorrt --io copy --fp16 on --streams 1 \
    --batches 256 --seconds 2 --warmup 1 --cache-dir "$OUT/trtcache-verbose" --detailed-build-log on \
    > "$OUT/trt-verbose.jsonl" 2> "$OUT/trt-verbose.log" || status=$?
if [[ ! -s $OUT/trt-verbose.jsonl ]]; then
    echo "trt_bench verbose run wrote no row (exit ${status:-0})" >&2
    exit 1
fi
if (( ${status:-0} != 0 )); then
    printf 'verbose\t%s\n' "$status" >> "$OUT/trt-exit.tsv"
fi
awk '/Engine Layer Information/{f=1;next} f && /^Layer\(/' "$OUT/trt-verbose.log" > "$OUT/trt-verbose-layers.txt"
