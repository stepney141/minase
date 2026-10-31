#!/usr/bin/env bash
# 作業機で新しい構成の生成を始め、塊が2つ確定したらSIGKILLで止め、同じ引数で再開する。
set -uo pipefail
W=/home/stepney141/board-games/minase/data/worktrees/alphazero-restart-techniques
O=$W/data/alphazero-restart-techniques/resume-check
N=$W/data/alphazero-restart-techniques/net-check/cycle-0000/model.onnx
C=$(git -C "$W" rev-parse HEAD)
source "$W/tools/train/zero/env.sh"
export XDG_CACHE_HOME=$W/data/alphazero-restart-techniques/net-check/xdg-cache
mkdir -p "$O"
args=(data zero-selfplay --output-dir "$O/gen" --nn-model "$N" --nn-backend tensorrt --generation-commit "$C"
      --seed 300000000 --games 1024 --chunk-games 128 --simulations 32 --playthrough-percent 100
      --threads 2 --games-per-thread 128)
echo "commit $C"; date -Is
setsid "$W/target/release/minase" "${args[@]}" > "$O/gen-run1.out" 2> "$O/gen-run1.err" &
pid=$!
until [ "$(find "$O/gen" -maxdepth 1 -name 'chunk-*.json' 2>/dev/null | wc -l)" -ge 2 ] || ! kill -0 $pid 2>/dev/null; do sleep 1; done
kill -KILL -- -$pid 2>/dev/null; wait $pid; echo "run1 exit $? at $(date -Is)"
( cd "$O/gen" && find . -maxdepth 1 -type f | sort | xargs sha256sum ) > "$O/gen-after-run1.txt"
cat "$O/gen-after-run1.txt"
"$W/target/release/minase" "${args[@]}" > "$O/gen-run2.out" 2> "$O/gen-run2.err"; echo "run2 exit $? at $(date -Is)"
( cd "$O/gen" && find . -maxdepth 1 -type f | sort | xargs sha256sum ) > "$O/gen-after-run2.txt"
cat "$O/gen-run2.out"
