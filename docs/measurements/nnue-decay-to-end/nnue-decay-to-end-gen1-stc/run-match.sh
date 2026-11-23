#!/usr/bin/env bash
# 候補の条件のネットと現行の条件のネットを、GSPRTで比べる。
# 使い方: run-match.sh <測定名> <シード> <stc|ltc> <候補のネット> <対照のネット> <同時対局数>
# 両側に世代反復の反復のコミットのエンジンを使い、ネットだけをEvalFileで替える。
# 測定名とシードは、設計書 docs/plans/nnue-decay-to-end.md の「測定と判定」の表の値を渡す。
# 途中で止まった測定は、同じ実行ディレクトリを minase match run --resume で再開する。
set -euo pipefail
name=$1
seed=$2
control=$3
candidate=$4
baseline=$5
concurrency=$6
R=/home/stepney141/board-games/minase
D=$R/data/nnue-generation-iteration
commit=$(cat "$D/iteration-commit")
case $control in
  stc) limits=(--each time=10000+100 --concurrency "$concurrency" gsprt --max-pairs 3000) ;;
  ltc) limits=(--each time=60000+200 --concurrency "$concurrency" gsprt) ;;
  *) echo "stc か ltc を指定する" >&2; exit 2 ;;
esac
mkdir -p "$R/data/nnue-decay-to-end"
cd "$R"
exec systemd-run --user --unit="$name" --same-dir --collect -p Nice=19 -p MemoryMax=12G -p MemorySwapMax=0 \
  --setenv=PATH="$PATH" \
  -p StandardOutput=append:"$R/data/nnue-decay-to-end/$name.log" \
  -p StandardError=append:"$R/data/nnue-decay-to-end/$name.log" \
  "$D/bin/minase" match run \
  --run-dir "data/matches/$name" --seed "$seed" \
  --candidate "commit:$commit" --baseline "commit:$commit" \
  --candidate-option "EvalFile=$candidate" --baseline-option "EvalFile=$baseline" \
  --rules lishogi "${limits[@]}"
