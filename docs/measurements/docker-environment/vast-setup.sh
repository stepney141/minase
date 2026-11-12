#!/usr/bin/env bash
# ソースを展開し、導入済みの環境でmnsd_readerとminaseをビルドして機械の構成を表示する。
# 使い方: bash vast-setup.sh /workspace/minase-<40桁のコミット>.tar
# Dockerfileから作ったイメージで借りたvast.aiの機械で実行する。
set -euo pipefail
date -Is
if [[ $# -ne 1 || ! -f "$1" ]]; then
    printf 'ソースの書庫を1つ指定してください\n' >&2
    exit 1
fi

mkdir -p /workspace/src /workspace/az /workspace/cache
tar -xf "$1" -C /workspace/src
date -Is
source /opt/minase-zero/env.sh
# 実行器の工程と同じキャッシュを使う。
export XDG_CACHE_HOME=/workspace/cache
bash /workspace/src/tools/train/zero/sync-env.sh
date -Is
(cd /workspace/src && cargo build --release --bin minase)
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
df -h /dev/shm
date -Is
echo setup-done
