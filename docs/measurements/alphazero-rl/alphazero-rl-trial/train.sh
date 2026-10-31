#!/usr/bin/env bash
# /content/az を作業ディレクトリとして学習器を起動する。引数はそのまま minase_zero へ渡す。
# 使い方: train.sh <minase_zero のサブコマンドと引数...>
set -euo pipefail
source /content/src/tools/train/zero/env.sh
cd /content/az
PYTHONPATH=/content/src/tools/train/zero exec uv run --frozen --project /content/src/tools/train/zero \
    python -m minase_zero "$@"
