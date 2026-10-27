#!/usr/bin/env bash
# 1回目：最初の塊が確定したら SIGKILL で止める。
cd /home/stepney141/board-games/minase/data/alphazero-selfplay
./run.sh > run1.out 2> run1.err &
pid=$!
start=$(date +%s)
until grep -q '^chunks: 1/3' run1.err 2>/dev/null; do
  if ! kill -0 $pid 2>/dev/null; then echo "exited before first chunk"; cat run1.err | tail -5; exit 1; fi
  if [ $(( $(date +%s) - start )) -gt 3000 ]; then echo timeout; kill -9 $pid; exit 1; fi
  sleep 0.2
done
kill -9 $pid
wait $pid 2>/dev/null
echo "killed after $(( $(date +%s) - start )) s"
