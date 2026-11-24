#!/usr/bin/env bash
# 世代反復の第N世代のネットを、masterのエンジンと標準のネットを対照としてSTCのGSPRTで比べる。
# 使い方: run-stc.sh <世代N> <候補（第N世代）のネット> <対照のmasterのコミット> <対照のネット> <シード> <同時対局数>
# 候補は反復のコミットのエンジンに第N世代のネットを渡したもの、対照は指定したmasterのコミットのエンジンに
# 標準のネット（docs/guides/evaluation-nnue.md の「探索の測定に使う標準のネット」）を渡したものである。
# 測定名は nnue-generation-iteration-gen<N>-master-stc である。
set -euo pipefail
generation=$1
candidate=$2
baseline_commit=$3
baseline=$4
seed=$5
concurrency=$6
R=/home/stepney141/board-games/minase
D=$R/data/nnue-generation-iteration
commit=$(cat "$D/iteration-commit")
name=nnue-generation-iteration-gen$generation-master-stc
cd "$R"
exec systemd-run --user --unit="$name" --same-dir --collect -p Nice=19 -p MemoryMax=12G -p MemorySwapMax=0 \
  --setenv=PATH="$PATH" \
  -p StandardOutput=append:"$D/gen$generation-master-stc.log" -p StandardError=append:"$D/gen$generation-master-stc.log" \
  "$D/bin/minase" match run \
  --run-dir "data/matches/$name" --seed "$seed" \
  --candidate "commit:$commit" --baseline "commit:$baseline_commit" \
  --candidate-option "EvalFile=$candidate" --baseline-option "EvalFile=$baseline" \
  --rules lishogi --each time=10000+100 --concurrency "$concurrency" gsprt --max-pairs 3000
