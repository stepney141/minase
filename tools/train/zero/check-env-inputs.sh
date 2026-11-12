#!/usr/bin/env bash
# Source the environment's env.sh before running this check or uv sync.
# Compare this worktree's five INPUTS byte-for-byte with copies in the environment
# installed by setup.sh: the parent of UV_PROJECT_ENVIRONMENT (/opt/minase-zero in Docker).
# Run this before uv sync, which would otherwise silently replace packages pinned
# in the image's virtual environment with versions from this worktree's uv.lock.
set -euo pipefail

ENV_DIR="$(dirname -- "${UV_PROJECT_ENVIRONMENT:?}")"
SOURCE_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
INPUTS=(setup.sh env.sh pyproject.toml uv.lock .python-version)

result=0
for input in "${INPUTS[@]}"; do
    if ! cmp -s -- "$SOURCE_DIR/$input" "$ENV_DIR/$input"; then
        printf '環境の入力が環境のディレクトリの写しと違います: %s（写し: %s）\n' \
            "$SOURCE_DIR/$input" "$ENV_DIR/$input" >&2
        result=1
    fi
done
if [[ "$result" -ne 0 ]]; then
    printf '%s\n' 'この環境は、作業ツリーと同じ環境の入力から作り直す必要があります' >&2
fi
exit "$result"
