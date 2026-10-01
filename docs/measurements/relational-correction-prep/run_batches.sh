#!/bin/sh
# 第2回以降を停止判定まで順に実行する。生成は固定worktreeから起動する。
set -e
R=/home/stepney141/board-games/minase/data/relational-correction
S=/home/stepney141/board-games/minase/data/worktrees/relational-correction/docs/measurements/relational-correction-prep/phase0.py
for B in 2 3 4; do
  if python3 -c "import json,sys; sys.exit(0 if json.load(open('$R/phase0/status.json'))['stop'] else 1)"; then
    echo "stop before batch $B"; exit 0
  fi
  P=$R/phase0/batch$B
  mkdir -p $P
  SEED=$((99000000 + 100000 * (B - 1)))
  (cd $R/pinned && $R/bin/selfplay_gen generate --output $P/generated.bin --kinds $P/generated.kinds \
    --history $P/generated.history.jsonl --games 1024 --seed $SEED --nodes 100000 --random-moves 0 \
    --concurrency 16 --max-ply 4000 --hash-mb 16 > $P/generate.log 2>&1)
  $R/bin/selfplay_gen inspect $P/generated.bin --kinds $P/generated.kinds --history $P/generated.history.jsonl > $P/inspect.log 2>&1
  python3 $S sample --batch $B --jobs 16 > $P/run.log 2>&1
  python3 $S teacher --batch $B --jobs 16 >> $P/run.log 2>&1
  python3 $S trace --batch $B --jobs 16 --root-cap 38.94 --batch-cap 14400 >> $P/run.log 2>&1
  python3 $S status >> $P/run.log 2>&1
  echo "batch $B done"; date
done
