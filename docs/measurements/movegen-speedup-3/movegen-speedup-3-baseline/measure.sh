#!/usr/bin/env bash
# 測定単位1つ分のサイクル数の対比較を、Botを止めた測定機で行う。
#
#   measure.sh <親のバイナリ> <候補のバイナリ> <benchの組数> <教師生成の組数> <結果.jsonl>
#
# 1. 対局の測定、SPSA、教師生成、およびcargoのビルドが動いていないことを確かめる（動いていれば中止する）。
# 2. lishogiのBotのエンジン（/opt/minase/minase）が1つも動いていない時点を待ち、Botのコンテナを止める。
# 3. paired.py runでbench、教師生成の順に対比較を行う。
# 4. 終了時（失敗時を含む）にBotのコンテナを起動し直す。
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
MS3=/home/stepney141/board-games/minase/data/ms3
BOT=minase-lishogi-bot-bot-1
PARENT=$1 CANDIDATE=$2 BENCH_PAIRS=$3 TEACHER_PAIRS=$4 OUT=$5

if pgrep -af '[m]inase (match run|spsa)|[m]atch_runner|[s]psa_runner|[s]elfplay generate|[c]argo (build|test|clippy)|[r]ustc '; then
  echo 'other CPU load is running; aborting' >&2
  exit 1
fi
while pgrep -f '^/opt/minase/minase --protocol usi' >/dev/null; do sleep 30; done
docker stop "$BOT" >/dev/null
echo "bot stopped at $(date -Is)"
# EXITのtrapに頼らず、測定の成否によらずBotを起動し直してから終了状態を返す。
status=0
python3 "$HERE/paired.py" run --workload bench --pairs "$BENCH_PAIRS" --parent "$PARENT" --candidate "$CANDIDATE" \
  --net "$MS3/net.bin" --cwd "$MS3/cwd" --out "$OUT" || status=$?
if [ "$status" -eq 0 ]; then
  python3 "$HERE/paired.py" run --workload teacher --pairs "$TEACHER_PAIRS" --parent "$PARENT" --candidate "$CANDIDATE" \
    --net "$MS3/net.bin" --cwd "$MS3/cwd" --out "$OUT" || status=$?
fi
docker start "$BOT" >/dev/null
echo "bot started at $(date -Is); finished with status $status"
exit "$status"
