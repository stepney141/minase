#!/usr/bin/env bash
# 世代反復の第N世代のネットを、暫定最良世代のネットとSTCのGSPRTで比べる。
# 使い方: run-stc.sh <世代N> <候補（第N世代）のネット> <対照（暫定最良世代）のネット> <同時対局数>
# 候補と対照の両方に反復のコミットのエンジンを使い、ネットだけをEvalFileで替える。
# 測定名は nnue-generation-iteration-gen<N>-stc、シードは 20,261,300,000,000 + 1,000,000×N である。
set -euo pipefail
generation=$1
candidate=$2
baseline=$3
concurrency=$4
R=/home/stepney141/board-games/minase
D=$R/data/nnue-generation-iteration
commit=$(cat "$D/iteration-commit")
name=nnue-generation-iteration-gen$generation-stc
cd "$R"
exec systemd-run --user --unit="$name" --same-dir --collect -p Nice=19 -p MemoryMax=12G -p MemorySwapMax=0 \
  --setenv=PATH="$PATH" \
  -p StandardOutput=append:"$D/gen$generation-stc.log" -p StandardError=append:"$D/gen$generation-stc.log" \
  "$D/bin/minase" match run \
  --run-dir "data/matches/$name" --seed $((20261300000000 + 1000000 * generation)) \
  --candidate "commit:$commit" --baseline "commit:$commit" \
  --candidate-option "EvalFile=$candidate" --baseline-option "EvalFile=$baseline" \
  --rules lishogi --each time=10000+100 --concurrency "$concurrency" gsprt --max-pairs 3000
