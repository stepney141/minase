#!/usr/bin/env bash
# alphazero-feasibility の部品の実測を1台の機械で行う。
# 使い方: measure.sh <展開したソースのルート> <出力ディレクトリ> <教師データのMNSD> <sim_costの並列数の列（例: 1,6,12）>
# ソースは統合ブランチの固定したコミットから git archive で作った書庫を展開したものとする。
set -euo pipefail

SRC=$1
OUT=$2
TEACHER=$3
PARALLEL=$4
ZERO=$SRC/tools/train/zero
mkdir -p "$OUT"
step() { printf '%s\t%s\n' "$1" "$(date +%s.%N)" >> "$OUT/timings.tsv"; }

step machine-start
{
    nvidia-smi --query-gpu=name,memory.total,driver_version,compute_cap --format=csv
    nproc
    lscpu
    for f in /sys/fs/cgroup/cpu.max /sys/fs/cgroup/cpuset.cpus.effective; do
        if [[ -e $f ]]; then echo "$f: $(cat "$f")"; else echo "$f: absent"; fi
    done
    free -m
    df -h "$OUT"
    cat /etc/os-release
    ldd --version | sed -n 1p
} > "$OUT/machine.txt" 2>&1

step setup-start
bash "$ZERO/setup.sh" > "$OUT/setup.log" 2>&1
step setup-end
source "$ZERO/env.sh"

step build-feasibility-start
(cd "$ZERO/feasibility" && cargo +"$RUST_TOOLCHAIN" build --release) > "$OUT/build-feasibility.log" 2>&1
step build-feasibility-end
step build-minase-start
(cd "$SRC" && cargo +"$RUST_TOOLCHAIN" build --release --bin minase) > "$OUT/build-minase.log" 2>&1
step build-minase-end

step nets-start
(cd "$ZERO" && uv run --frozen python feasibility/make_nets.py --out-dir "$OUT/nets") > "$OUT/nets.log" 2>&1
(cd "$OUT/nets" && sha256sum ./*.onnx) > "$OUT/nets/SHA256SUMS"
step nets-end

step trt-start
bash "$(dirname -- "${BASH_SOURCE[0]}")/measure-trt.sh" "$SRC" "$OUT"
step trt-end

SIM=$ZERO/feasibility/target/release/sim_cost
sim() {
    "$SIM" --data "$TEACHER" --positions 2000 --stride 1709 --simulations 32 --considered 16 --seed "$1" --breakdown "$2"
}
step sim-start
sim 1 on > "$OUT/sim-breakdown.json"
for p in ${PARALLEL//,/ }; do
    for i in $(seq 1 "$p"); do
        sim "$i" off > "$OUT/sim-p$p-$i.json" &
    done
    wait
done
step sim-end

# GPUのメモリに収まらない構成は、失敗を train-failures.tsv に記録して次へ進む。
train() {
    (cd "$ZERO" && uv run --frozen python feasibility/train_bench.py --blocks "$1" --channels "$2" \
        --batches "$4" --precision "$3" --steps 50 --warmup 10 --device cuda) \
        >> "$OUT/train.jsonl" 2>> "$OUT/train.log" \
        || printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >> "$OUT/train-failures.tsv"
}
step train-start
for size in "6 96" "10 128" "15 192" "20 256"; do
    set -- $size
    for precision in bf16 fp32; do
        for batch in 256 1024; do
            train "$1" "$2" "$precision" "$batch"
        done
    done
done
step train-end
step machine-end
