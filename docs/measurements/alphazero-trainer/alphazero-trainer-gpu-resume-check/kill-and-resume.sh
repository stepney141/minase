#!/usr/bin/env bash
# Starts run B, kills uv and its Python child with SIGKILL right after the step-200 checkpoint
# line appears, and resumes from the last checkpoint (run with bash).
set -euo pipefail
D=/home/stepney141/board-games/minase/data/alphazero-trainer
(setsid bash "$D/run-train.sh" "$D/train-b" > "$D/train-b-run1.log" 2>&1 &)
until grep -q "steps=200/" "$D/train-b-run1.log" 2>/dev/null; do sleep 0.2; done
kill -KILL $(pgrep -f "[m]inase_zero train .*train-b")
sleep 1
cd /home/stepney141/board-games/minase/data/worktrees/alphazero-trainer/tools/train/zero
uv run --frozen python -m minase_zero train --resume "$D/train-b" --device cuda > "$D/train-b-run2.log" 2>&1
