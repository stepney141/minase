#!/usr/bin/env bash
set -u
source /home/stepney141/board-games/minase/tools/train/zero/env.sh
export XDG_CACHE_HOME=$HOME/.cache
nice -n 19 /tmp/claude-1000/-home-stepney141-board-games-minase/3c637203-c8df-48af-bde7-1683ae80e271/scratchpad/diag-maxply/target/release/minase data zero-selfplay --output-dir /tmp/claude-1000/-home-stepney141-board-games-minase/3c637203-c8df-48af-bde7-1683ae80e271/scratchpad/diag-out --nn-model /mnt/synology_nas/minase/alphazero/cycles/cycle-0008/train/model.onnx --nn-backend tensorrt --generation-commit ac2b3f3e0b6d3eeff6cf6cd21d7ddfe9be9b1dd2 --seed 900000000000 --games 400 --chunk-games 50 --target-positions 1000000000 --simulations 32 --playthrough-percent 100 --threads 4 --games-per-thread 64
echo "exit $?"
