#!/usr/bin/env bash
# Usage: CARGO_BUILD_JOBS=1 nice -n 19 bash build_engine.sh candidate.mnpt output/minase
set -euo pipefail
if [[ $# != 2 ]]; then
    echo "usage: $0 MNPT OUTPUT" >&2
    exit 2
fi
weights=$(realpath -- "$1")
output=$(realpath -m -- "$2")
repo=$(git -C "$(dirname -- "$0")" rev-parse --show-toplevel)
[[ -f "$weights" ]] || { echo "MNPT does not exist: $weights" >&2; exit 2; }
[[ ! -e "$output" ]] || { echo "output already exists: $output" >&2; exit 2; }
temporary=$(mktemp -d)
cleanup() {
    if [[ -e "$temporary/worktree/.git" ]]; then
        git -C "$repo" worktree remove --force "$temporary/worktree"
    fi
    rm -rf -- "$temporary"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
git -C "$repo" worktree add --detach "$temporary/worktree" HEAD
cp -- "$weights" "$temporary/worktree/nets/pst.bin"
(
    cd "$temporary/worktree"
    # Isolate artifacts even when the caller has set CARGO_TARGET_DIR.
    # CARGO_BUILD_JOBS is inherited without alteration.
    CARGO_TARGET_DIR="$temporary/target" cargo build --locked --release --bin minase
)
mkdir -p -- "$(dirname -- "$output")"
cp -- "$temporary/target/release/minase" "$output"
sha256sum -- "$output" "$weights"
