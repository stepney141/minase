#!/usr/bin/env bash
# vast.aiのGPU 4枚の機械で、推論の転送を減らす変種のビルドと測定を1段ずつ行う。
# 使い方: bash vast-measure.sh <段> [段の引数]
# 前提: docs/measurements/alphazero-multi-gpu/vast-setup.shで、測定の道具を含むコミットを
#   /workspace/srcに導入し、学習環境、ONNX Runtime、Rustのツールチェーンを準備してある。
#   /workspace/in/に次の入力を置いてある。
#   minase-<名前>.tar（S、A、B、C、Dなど、各変種のコミットのgit archiveの書庫）
#   nets/<名前>.onnx と nets/<名前>.pytorch.json（各変種の署名で書き出した2回目の実行の
#     初期ネットと、作業機で作った照合の参照）
#   variants.json（measure-generation.pyの変種の一覧。minaseは/workspace/v/<名前>/target/release/minase、
#     networkは/workspace/in/nets/<名前>.onnxを指す）
# 出力は/workspace/m/の下に置く。
# TensorRTのエンジンのキャッシュはXDG_CACHE_HOME=/workspace/cacheの下に置く。
# 段:
#   build <名前>...                         各変種の書庫を展開してビルドする。
#   bandwidth <GPUの枚数>                   各GPUで順に転送の帯域を測り、次に全GPUで同時に測る。
#   check <CPUの比較の方式> <名前>...        CPUとTensorRTのバッチ7の出力を参照と比べる。
#   warm <GPUの枚数>                        各変種で16局を生成し、TensorRTのエンジンを作る。
#   run <GPUの枚数> <最初の周> <周の数> [変種の一覧のファイル]  4,096局ずつの生成を測る。
#   summarize [変種の一覧のファイル]        生成の測定を集計し、summary.jsonにも保存する。
# CPUの比較の方式はlogitsまたはfp16-logitsを指定する。最初の周は0から数える。
# runとsummarizeで変種の一覧を省略した場合は/workspace/in/variants.jsonを使う。
set -euo pipefail

fail() {
    printf '%s\n' "$1" >&2
    exit 1
}

[[ $# -ge 1 ]] || fail '使い方: bash vast-measure.sh <段> [段の引数]'
stage=$1
shift
case "$stage" in
build)
    [[ $# -ge 1 ]] || fail '使い方: bash vast-measure.sh build <名前>...'
    ;;
bandwidth)
    [[ $# -eq 1 ]] || fail '使い方: bash vast-measure.sh bandwidth <GPUの枚数>'
    [[ "$1" =~ ^[1-9][0-9]*$ ]] || fail 'GPUの枚数は正の整数で指定してください'
    ;;
check)
    [[ $# -ge 2 ]] || fail '使い方: bash vast-measure.sh check <CPUの比較の方式> <名前>...'
    [[ "$1" == logits || "$1" == fp16-logits ]] || fail 'CPUの比較の方式はlogitsかfp16-logitsを指定してください'
    ;;
warm)
    [[ $# -eq 1 ]] || fail '使い方: bash vast-measure.sh warm <GPUの枚数>'
    ;;
run)
    [[ $# -ge 3 && $# -le 4 ]] || fail '使い方: bash vast-measure.sh run <GPUの枚数> <最初の周> <周の数> [変種の一覧のファイル]'
    ;;
summarize)
    [[ $# -le 1 ]] || fail '使い方: bash vast-measure.sh summarize [変種の一覧のファイル]'
    ;;
*)
    fail "不明な段: $stage"
    ;;
esac

Z=/workspace/src/tools/train/zero
D=/workspace/src/docs/measurements/alphazero-inference-transfer
M=/workspace/m
# shellcheck source=tools/train/zero/env.sh
source "$Z/env.sh"
export XDG_CACHE_HOME=/workspace/cache
date -Is
echo "stage=$stage"

case "$stage" in
build)
    mkdir -p /workspace/v
    for name in "$@"; do
        out=/workspace/v/$name
        SECONDS=0
        mkdir "$out"
        tar -xf "/workspace/in/minase-$name.tar" -C "$out"
        (cd "$out" && cargo +"$RUST_TOOLCHAIN" build --release --bin minase)
        printf 'build %s elapsed_seconds=%s\n' "$name" "$SECONDS"
    done
    ;;
bandwidth)
    devices=$1
    out=$M/bandwidth
    mkdir -p "$M"
    mkdir "$out"
    cd "$Z"
    for ((gpu = 0; gpu < devices; gpu++)); do
        uv run --frozen python /workspace/src/docs/measurements/alphazero-multi-gpu/copy-bandwidth.py "$gpu" \
            > "$out/single-$gpu.txt" 2>&1
    done
    pids=()
    for ((gpu = 0; gpu < devices; gpu++)); do
        uv run --frozen python /workspace/src/docs/measurements/alphazero-multi-gpu/copy-bandwidth.py "$gpu" \
            > "$out/concurrent-$gpu.txt" 2>&1 &
        pids+=("$!")
    done
    failed=0
    for pid in "${pids[@]}"; do
        if ! wait "$pid"; then
            failed=1
        fi
    done
    [[ "$failed" -eq 0 ]] || fail 'bandwidth: 同時測定に失敗したGPUがあります'
    ;;
check)
    policy=$1
    shift
    for name in "$@"; do
        variant=/workspace/v/$name
        network=/workspace/in/nets/$name.onnx
        reference=/workspace/in/nets/$name.pytorch.json
        out=$M/check/$name
        mkdir -p "$out"
        "$variant/target/release/minase" dev nn-eval --nn-model "$network" \
            --backend cpu --batch 7 --out "$out/cpu-b7.json"
        python3 "$variant/tools/train/zero/nn_compare.py" --reference "$reference" \
            --candidate "$out/cpu-b7.json" --policy "$policy" \
            --policy-tolerance 1e-4 --value-tolerance 1e-4 > "$out/cpu-b7.compare.json"
        "$variant/target/release/minase" dev nn-eval --nn-model "$network" \
            --backend tensorrt --batch 7 --out "$out/tensorrt-b7.json" --cache-dir /workspace/cache/nn-eval
        python3 "$variant/tools/train/zero/nn_compare.py" --reference "$reference" \
            --candidate "$out/tensorrt-b7.json" --policy probabilities \
            --policy-tolerance 1e-2 --value-tolerance 5e-2 > "$out/tensorrt-b7.compare.json"
    done
    ;;
warm)
    python3 "$D/measure-generation.py" warm --variants /workspace/in/variants.json --out "$M/gen" \
        --devices "$1" --threads-per-gpu 4 --games-per-thread 128 --simulations 32 --seed 9100000000
    ;;
run)
    if [[ $# -eq 4 ]]; then
        variants=$4
    else
        variants=/workspace/in/variants.json
    fi
    python3 "$D/measure-generation.py" run --variants "$variants" --out "$M/gen" \
        --devices "$1" --threads-per-gpu 4 --games-per-thread 128 --simulations 32 --seed 9100000000 \
        --round-start "$2" --rounds "$3" --games 4096 --chunk-games 256
    ;;
summarize)
    if [[ $# -eq 1 ]]; then
        variants=$1
    else
        variants=/workspace/in/variants.json
    fi
    python3 "$D/measure-generation.py" summarize --variants "$variants" --runs "$M/gen/runs.jsonl" \
        | tee "$M/gen/summary.json"
    ;;
esac
date -Is
echo "stage-done $stage"
