#!/usr/bin/env bash
# 作業機でAlphaZero用のイメージを起動する。引数はコンテナ内で実行するコマンドであり、
# 省略するとbashを起動する。
#
#   tools/train/zero/run.sh uv run --project tools/train/zero pytest tools/train/zero/tests
#
# リポジトリ（worktreeならそのworktree）を/workへ、本体リポジトリのdata/を/work/data/へ
# マウントし、ホストの利用者のuidで動かす。TensorRTのエンジンのキャッシュはdata/cache/に置いて
# 起動をまたいで再利用し、Cargoの成果物はホストのビルドと混ざらないようtarget/zero-image/へ分ける。
set -euo pipefail

IMAGE="${MINASE_ZERO_IMAGE:-minase-zero}"
REPO="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
DATA="$(cd "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)/.." && pwd)/data"

if [ "$#" -eq 0 ]; then
    set -- bash
fi
TTY=()
if [ -t 0 ]; then
    TTY=(-t)
fi

exec docker run --rm -i "${TTY[@]}" --gpus all \
    --user "$(id -u):$(id -g)" \
    -v "$REPO:/work" \
    -v "$DATA:/work/data" \
    -e HOME=/tmp \
    -e XDG_CACHE_HOME=/work/data/cache \
    -e CARGO_TARGET_DIR=/work/target/zero-image \
    -e ORT_DYLIB_PATH=/opt/zero-venv/lib/python3.12/site-packages/onnxruntime/capi/libonnxruntime.so.1.28.0 \
    "$IMAGE" "$@"
