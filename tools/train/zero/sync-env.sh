#!/usr/bin/env bash
# Source the environment's env.sh before running this script.
# After the environment inputs pass validation, run uv sync --frozen to build
# this worktree's mnsd_reader from source and install it into the virtual environment.
set -euo pipefail

if [[ $# -ne 0 ]]; then
    printf '%s\n' 'sync-env.sh は引数を取りません' >&2
    exit 1
fi

SOURCE_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
"$SOURCE_DIR/check-env-inputs.sh"
exec uv sync --frozen --project "$SOURCE_DIR"
