#!/usr/bin/env bash
# vast.aiのGPU 4枚の機械で、複数GPU化の設計書のフェーズ4の測定を1段ずつ行う。
# 使い方: bash vast-measure.sh <段> <学習器のコミット40桁> [段の引数]
# 前提: vast-setup.shで/workspace/srcを導入し、/workspace/mに次を置いてある。
#   init/checkpoint.pt と init/model.onnx（cycles-rl2/cycle-0000の初期ネット）
#   c32.json（1回目のサイクル32の生成ファイルの絶対パスのJSON配列）と、そのファイル
#   buf.json と latest.json（1回目のサイクル3〜31の生成ファイルと、サイクル31だけのJSON配列）と、そのファイル
#   train-nocompile.py（コンパイルを入れる前のコミットのminase_zero/train.py）
# 段:
#   sanity                 GPU 0でのコンパイルの勾配の確認と、塊0の2ファイルでのN=4の1サイクル
#   gen-warm               TensorRTのエンジンを作るための16局の生成
#   gen <GPUあたりのスレッド数>  4,096局の生成の時間
#   nocompile              コンパイルしない学習器でN=4の1サイクル
#   resume                 コンパイルした学習器でN=4の途中で止めない学習、強制終了、再開の比較
#   n1pair                 GPU 0で全体のバッチ1,024、GPU 1で256のN=1の1サイクルを同時に学ぶ
#   compare                N=1とN=4の出力の差と、全体のバッチ256と1,024の検証の損失の比較
#   load                   約3,000万局面の読み込みの時間と最大常駐メモリ
#   ort-exit <ネット>       minase dev nn-evalを各実行プロバイダで繰り返して終了コードを数え、異常終了したときのスタックをgdbで取る
#   check-timing <学習の出力>  実行器の照合と同じ12個のコマンドの時間と終了コードを、異常終了で止めずに測る
#   seed2                  抽出のシードだけを変えたN=4の1サイクルと、その出力の比較
#   ort-exit2 <ネット>      新しいキャッシュでTensorRTのエンジンを作るnn-evalを、libcudaやONNX Runtimeを先に読み込ませる場合とデバッグ用のmallocの場合で繰り返す
set -euo pipefail
stage=$1; C=$2; shift 2
Z=/workspace/src/tools/train/zero
D=/workspace/src/docs/measurements/alphazero-multi-gpu
M=/workspace/m
source "$Z/env.sh"
export XDG_CACHE_HOME=/workspace/cache
# 使えるCPUの数（cpusetとcgroupの割当ての小さいほう）を4で割り、各GPUのプロセスの再生のスレッド数にする。
cpus=$(python3 -c "
import os, pathlib
n = len(os.sched_getaffinity(0)); r = pathlib.Path('/sys/fs/cgroup')
if (r / 'cpu.max').is_file():
    q, p = (r / 'cpu.max').read_text().split(); q = None if q == 'max' else int(q); p = int(p)
else:
    q = int((r / 'cpu/cpu.cfs_quota_us').read_text()); q = None if q == -1 else q
    p = int((r / 'cpu/cpu.cfs_period_us').read_text())
print(n if q is None else min(n, q // p))")
R=$((cpus / 4))
c32=$(python3 -c "import json; print(' '.join(json.load(open('$M/c32.json'))))")
date -Is
echo "stage=$stage cpus=$cpus replay_threads=$R"

new_cycle() {  # <出力先> <全体のバッチ> <学習率> <保存の間隔> <コミット>
    printf '%s\n' --init "$M/init/checkpoint.pt" --data-list "$M/c32.json" --latest-data $c32 \
        --out-dir "$1" --cycle 1 --seed 20261101 --batch-size "$2" --micro-batch-size 256 \
        --lr "$3" --momentum 0.9 --weight-decay 0.0001 --clip-grad-norm 10 --value-lambda 0.333 \
        --precision bf16 --checkpoint-steps "$4" --device cuda --replay-threads "$R" --trainer-commit "$5"
}

case "$stage" in
sanity)
    cd "$Z"
    read -r first second < <(python3 -c "import json; print(*json.load(open('$M/c32.json'))[:2])")
    uv run --frozen python "$D/compile-gradients.py" --checkpoint "$M/init/checkpoint.pt" \
        --data "$first" "$second" --micro-batch-size 256 --seed 20261101 --device cuda \
        --out "$M/sanity-gradients.json"
    printf '["%s","%s"]\n' "$first" "$second" > "$M/sanity.json"
    mkdir -p "$M/sanity-cache"
    TORCHINDUCTOR_CACHE_DIR="$M/sanity-cache/inductor" TRITON_CACHE_DIR="$M/sanity-cache/triton" \
        uv run --frozen torchrun --standalone --nproc-per-node 4 -m minase_zero train \
        --init "$M/init/checkpoint.pt" --data-list "$M/sanity.json" --latest-data "$first" "$second" \
        --out-dir "$M/sanity-train" --cycle 1 --seed 20261101 --batch-size 1024 --micro-batch-size 256 \
        --lr 0.04 --momentum 0.9 --weight-decay 0.0001 --clip-grad-norm 10 --value-lambda 0.333 \
        --precision bf16 --checkpoint-steps 50 --device cuda --replay-threads "$R" --trainer-commit "$C"
    ;;
gen-warm)
    /workspace/src/target/release/minase data zero-selfplay --output-dir "$M/gen-warm" \
        --nn-model "$M/init/model.onnx" --nn-backend tensorrt --generation-commit "$C" --seed 9000000000 \
        --start-position shuffled-setup --games 16 --target-positions 1000000000 --chunk-games 16 \
        --simulations 32 --playthrough-percent 100 --threads 4 --devices 4 --games-per-thread 4
    ;;
gen)
    t=$1
    /usr/bin/time -v /workspace/src/target/release/minase data zero-selfplay --output-dir "$M/gen-t$t" \
        --nn-model "$M/init/model.onnx" --nn-backend tensorrt --generation-commit "$C" --seed 9100000000 \
        --start-position shuffled-setup --games 4096 --target-positions 1000000000 --chunk-games 256 \
        --simulations 32 --playthrough-percent 100 --threads $((4 * t)) --devices 4 --games-per-thread 128
    ;;
nocompile)
    rm -rf "$M/nocompile"
    mkdir -p "$M/nocompile/cache"
    cp -r "$Z/minase_zero" "$M/nocompile/"
    cp "$M/train-nocompile.py" "$M/nocompile/minase_zero/train.py"
    cd "$M/nocompile"
    commit=$1
    mapfile -t arguments < <(new_cycle "$M/nocompile/run" 1024 0.04 50 "$commit")
    TORCHINDUCTOR_CACHE_DIR="$M/nocompile/cache/inductor" TRITON_CACHE_DIR="$M/nocompile/cache/triton" \
        /usr/bin/time -v "$Z/.venv/bin/torchrun" --standalone --nproc-per-node 4 -m minase_zero train "${arguments[@]}"
    ;;
resume)
    cd "$Z"
    uv run --frozen python "$D/resume-check.py" --out "$M/resume-n4" --commit "$C" --init "$M/init/checkpoint.pt" \
        --data-list "$M/c32.json" --nproc 4 --replay-threads "$R" --checkpoint-steps 50 --kill-after 1950
    ;;
n1pair)
    cd "$Z"
    pids=()
    for job in "0 b1024 1024 0.04 50" "1 b256 256 0.01 200"; do
        read -r gpu name batch lr steps <<< "$job"
        mkdir -p "$M/n1-$name-cache"
        mapfile -t arguments < <(new_cycle "$M/n1-$name" "$batch" "$lr" "$steps" "$C")
        CUDA_VISIBLE_DEVICES=$gpu TORCHINDUCTOR_CACHE_DIR="$M/n1-$name-cache/inductor" \
            TRITON_CACHE_DIR="$M/n1-$name-cache/triton" \
            /usr/bin/time -v uv run --frozen torchrun --standalone --nproc-per-node 1 -m minase_zero train \
            "${arguments[@]}" > "$M/n1-$name.log" 2>&1 &
        pids+=("$!")
    done
    failed=0
    for pid in "${pids[@]}"; do
        wait "$pid" || failed=1
    done
    [ "$failed" -eq 0 ] || { echo "n1pair: a training failed" >&2; exit 1; }
    ;;
compare)
    cd "$Z"
    fixture=/workspace/src/crates/minase/tests/fixtures/nn-positions.jsonl
    for pair in "n1 $M/n1-b1024" "n4 $M/resume-n4/uninterrupted"; do
        read -r name run <<< "$pair"
        uv run --frozen python -m minase_zero fixture-eval --checkpoint "$run/checkpoint.pt" \
            --onnx "$run/model.onnx" --fixture "$fixture" --out "$M/fixture-$name.json"
    done
    uv run --frozen python "$D/compare-outputs.py" --reference "$M/fixture-n1.json" \
        --candidate "$M/fixture-n4.json" --out "$M/n1-vs-n4.json"
    uv run --frozen python "$D/compare-validation.py" --reference "$M/n1-b256/checkpoint.pt" \
        --candidate "$M/n1-b1024/checkpoint.pt" --data $c32 --value-lambda 0.333 --batch 256 \
        --resamples 10000 --seed 20261110 --device cuda --out "$M/b256-vs-b1024.json"
    ;;
load)
    cd "$Z"
    uv run --frozen python "$D/load-check.py" --out "$M/load" --commit "$C" --init "$M/init/checkpoint.pt" \
        --data-list "$M/buf.json" --latest-list "$M/latest.json" --nproc 4 --replay-threads "$R"
    ;;
ort-exit)
    net=$1
    out=$M/ort-exit
    mkdir -p "$out"
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends gdb > /dev/null
    minase=/workspace/src/target/release/minase
    run() {  # <名前> <コマンド...>
        local name=$1; shift
        local started=$(date +%s%N)
        set +e
        "$@" > "$out/$name.out" 2> "$out/$name.err"
        local code=$?
        set -e
        printf '%s\t%s\t%s\t%s\n' "$name" "$code" "$(( ($(date +%s%N) - started) / 1000000 ))" "$(tail -n 1 "$out/$name.err" | tr '\t' ' ')" >> "$out/exit.tsv"
    }
    for i in 1 2 3 4 5; do run "trt-b1-$i" "$minase" dev nn-eval --nn-model "$net" --backend tensorrt --batch 1 --out "$out/trt-b1-$i.json" --cache-dir /workspace/cache/ort-exit; done
    for i in 1 2 3; do run "trt-b7-$i" "$minase" dev nn-eval --nn-model "$net" --backend tensorrt --batch 7 --out "$out/trt-b7-$i.json" --cache-dir /workspace/cache/ort-exit; done
    for i in 1 2 3; do run "trt-b256-$i" "$minase" dev nn-eval --nn-model "$net" --backend tensorrt --batch 256 --out "$out/trt-b256-$i.json" --cache-dir /workspace/cache/ort-exit; done
    for i in 1 2 3; do run "cuda-b7-$i" "$minase" dev nn-eval --nn-model "$net" --backend cuda --batch 7 --out "$out/cuda-b7-$i.json"; done
    for i in 1 2; do run "cpu-b7-$i" "$minase" dev nn-eval --nn-model "$net" --backend cpu --batch 7 --out "$out/cpu-b7-$i.json"; done
    debug=$(ls /usr/lib/x86_64-linux-gnu/libc_malloc_debug.so.0 2>/dev/null || true)
    if [ -n "$debug" ]; then
        for i in 1 2 3; do run "trt-b1-mallocdebug-$i" env LD_PRELOAD="$debug" MALLOC_CHECK_=3 "$minase" dev nn-eval --nn-model "$net" --backend tensorrt --batch 1 --out "$out/trt-b1-mallocdebug-$i.json" --cache-dir /workspace/cache/ort-exit; done
    fi
    for i in 1 2 3; do
        set +e
        gdb -batch -ex run -ex bt -ex "info sharedlibrary" --args "$minase" dev nn-eval --nn-model "$net" --backend tensorrt --batch 1 \
            --out "$out/gdb-$i.json" --cache-dir /workspace/cache/ort-exit > "$out/gdb-$i.txt" 2>&1
        set -e
        if grep -q "SIGABRT" "$out/gdb-$i.txt"; then break; fi
    done
    cat "$out/exit.tsv"
    ;;
check-timing)
    train=$1
    out=$M/check-timing
    mkdir -p "$out"
    cd "$Z"
    minase=/workspace/src/target/release/minase
    fixture=/workspace/src/crates/minase/tests/fixtures/nn-positions.jsonl
    step() {  # <名前> <コマンド...>
        local name=$1; shift
        local started=$(date +%s%N)
        set +e
        "$@" > "$out/$name.out" 2> "$out/$name.err"
        local code=$?
        set -e
        printf '%s\t%s\t%s\n' "$name" "$code" "$(( ($(date +%s%N) - started) / 1000000 ))" >> "$out/steps.tsv"
    }
    step fixture-eval uv run --frozen python -m minase_zero fixture-eval --checkpoint "$train/checkpoint.pt" \
        --onnx "$train/model.onnx" --fixture "$fixture" --out "$out/pytorch.json"
    for batch in 1 7 256; do
        step "cpu-b$batch" "$minase" dev nn-eval --nn-model "$train/model.onnx" --backend cpu --batch "$batch" --out "$out/cpu-b$batch.json"
        step "cpu-b$batch-compare" uv run --frozen python nn_compare.py --reference "$out/pytorch.json" --candidate "$out/cpu-b$batch.json" \
            --policy logits --policy-tolerance 1e-4 --value-tolerance 1e-4
        step "trt-b$batch" "$minase" dev nn-eval --nn-model "$train/model.onnx" --backend tensorrt --batch "$batch" --out "$out/trt-b$batch.json" \
            --cache-dir /workspace/cache/check-timing
        step "trt-b$batch-compare" uv run --frozen python nn_compare.py --reference "$out/pytorch.json" --candidate "$out/trt-b$batch.json" \
            --policy probabilities --policy-tolerance 1e-2 --value-tolerance 5e-2
    done
    cat "$out/steps.tsv"
    ;;
seed2)
    cd "$Z"
    mkdir -p "$M/seed2-cache"
    mapfile -t arguments < <(new_cycle "$M/seed2" 1024 0.04 50 "$C")
    arguments=("${arguments[@]/#20261101/20261102}")
    TORCHINDUCTOR_CACHE_DIR="$M/seed2-cache/inductor" TRITON_CACHE_DIR="$M/seed2-cache/triton" \
        uv run --frozen torchrun --standalone --nproc-per-node 4 -m minase_zero train "${arguments[@]}"
    fixture=/workspace/src/crates/minase/tests/fixtures/nn-positions.jsonl
    for pair in "seed1 $M/resume-n4/uninterrupted" "seed2 $M/seed2"; do
        read -r name run <<< "$pair"
        uv run --frozen python -m minase_zero fixture-eval --checkpoint "$run/checkpoint.pt" \
            --onnx "$run/model.onnx" --fixture "$fixture" --out "$M/fixture-n4-$name.json"
    done
    uv run --frozen python "$D/compare-outputs.py" --reference "$M/fixture-n4-seed1.json" \
        --candidate "$M/fixture-n4-seed2.json" --out "$M/seed1-vs-seed2.json"
    ;;
ort-exit2)
    net=$1
    out=$M/ort-exit2
    mkdir -p "$out"
    command -v gdb > /dev/null || DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends gdb > /dev/null
    minase=/workspace/src/target/release/minase
    cuda=/usr/lib/x86_64-linux-gnu/libcuda.so.1
    debug=/usr/lib/x86_64-linux-gnu/libc_malloc_debug.so.0
    n=0
    build() {  # <名前> <回数> [環境変数...]
        local name=$1 count=$2; shift 2
        for i in $(seq 1 "$count"); do
            n=$((n + 1))
            local started=$(date +%s%N)
            set +e
            env "$@" "$minase" dev nn-eval --nn-model "$net" --backend tensorrt --batch 1 \
                --out "$out/$name-$i.json" --cache-dir "/workspace/cache/ort-exit2-$n" > "$out/$name-$i.out" 2> "$out/$name-$i.err"
            local code=$?
            set -e
            printf '%s\t%s\t%s\t%s\n' "$name-$i" "$code" "$(( ($(date +%s%N) - started) / 1000000 ))" "$(tail -n 1 "$out/$name-$i.err" | tr '\t' ' ')" >> "$out/exit.tsv"
        done
    }
    build base 6
    build preload-libcuda 6 LD_PRELOAD="$cuda"
    build preload-ort 6 LD_PRELOAD="$ORT_DYLIB_PATH"
    for i in 1 2 3 4; do
        n=$((n + 1))
        set +e
        gdb -batch -ex "set environment LD_PRELOAD $debug" -ex "set environment MALLOC_CHECK_ 3" -ex run -ex bt -ex "info sharedlibrary" \
            --args "$minase" dev nn-eval --nn-model "$net" --backend tensorrt --batch 1 \
            --out "$out/mallocdebug-gdb-$i.json" --cache-dir "/workspace/cache/ort-exit2-$n" > "$out/mallocdebug-gdb-$i.txt" 2>&1
        set -e
        printf '%s\t%s\n' "mallocdebug-gdb-$i" "$(grep -c SIGABRT "$out/mallocdebug-gdb-$i.txt")" >> "$out/exit.tsv"
    done
    cat "$out/exit.tsv"
    ;;
*)
    echo "unknown stage: $stage" >&2
    exit 1
    ;;
esac
date -Is
echo "stage-done $stage"
