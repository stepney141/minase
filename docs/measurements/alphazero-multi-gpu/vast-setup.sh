#!/usr/bin/env bash
# vast.aiの機械に学習環境を導入し、minaseをビルドして機械の構成を表示する。
# 使い方: bash vast-setup.sh /workspace/minase-<40桁のコミット>.tar
set -euo pipefail
date -Is
if [[ $# -ne 1 || ! -f "$1" ]]; then
    printf 'ソースの書庫を1つ指定してください\n' >&2
    exit 1
fi

apt-get update
date -Is
apt-get install -y --no-install-recommends \
    ca-certificates curl rsync python3 build-essential pkg-config git xz-utils procps time
date -Is

mkdir -p /workspace/src /workspace/az /workspace/cache
tar -xf "$1" -C /workspace/src
date -Is
bash /workspace/src/tools/train/zero/setup.sh
source /workspace/src/tools/train/zero/env.sh
date -Is
(cd /workspace/src && cargo +"$RUST_TOOLCHAIN" build --release --bin minase)
date -Is

nvidia-smi --query-gpu=name,memory.total,driver_version,compute_cap --format=csv
nproc
if [[ -f /sys/fs/cgroup/cpu.max ]]; then
    cat /sys/fs/cgroup/cpu.max
else
    cat /sys/fs/cgroup/cpu/cpu.cfs_quota_us
    cat /sys/fs/cgroup/cpu/cpu.cfs_period_us
fi
LC_ALL=C lscpu | grep -E 'Model name|Thread|Core|Socket'
free -g
df -h /workspace
date -Is
echo setup-done
