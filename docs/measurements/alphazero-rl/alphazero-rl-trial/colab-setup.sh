#!/usr/bin/env bash
# Colab機でソースの書庫を展開し、環境を導入してminaseをビルドし、機械の構成を表示する。
# 使い方: colab-setup.sh <書庫>
set -euo pipefail
date -Is
mkdir -p /content/src /content/az /content/cache
tar -xf "$1" -C /content/src
bash /content/src/tools/train/zero/setup.sh
source /content/src/tools/train/zero/env.sh
date -Is
(cd /content/src && cargo +"$RUST_TOOLCHAIN" build --release --bin minase)
date -Is
nvidia-smi --query-gpu=name,memory.total,driver_version,compute_cap --format=csv
nproc
cat /sys/fs/cgroup/cpu.max
lscpu | grep -E 'Model name|Thread|Core|Socket'
free -g | head -2
command -v uv
echo setup-done
