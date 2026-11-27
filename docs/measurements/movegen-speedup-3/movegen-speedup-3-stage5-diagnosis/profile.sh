#!/usr/bin/env bash
# 採用版のフレームポインタつきのビルドで、bench深さ16と教師生成2局のサイクルと分岐予測ミスの標本を採る。
#
#   profile.sh <フレームポインタつきのバイナリ> <出力ディレクトリ>
#
# measure.shと同じく、他の負荷がないことを確かめ、Botの対局が途切れた時点でBotのコンテナを止め、終了時に起動し直す。
set -euo pipefail
MS3=/home/stepney141/board-games/minase/data/ms3
BOT=minase-lishogi-bot-bot-1
BIN=$(realpath "$1") OUT=$(realpath "$2")
mkdir -p "$OUT"
if pgrep -af '[m]inase (match run|spsa)|[m]atch_runner|[s]psa_runner|[s]elfplay generate|[c]argo (build|test|clippy)|[r]ustc '; then
  echo 'other CPU load is running; aborting' >&2
  exit 1
fi
while pgrep -f '^/opt/minase/minase --protocol usi' >/dev/null; do sleep 30; done
docker stop "$BOT" >/dev/null
trap 'docker start "$BOT" >/dev/null' EXIT
printf 'setoption name RuleSet value JCSA\nsetoption name EvalFile value %s\nisready\nbench depth 16 threads 1 repetitions 1\nquit\n' \
  "$MS3/net.bin" > "$OUT/bench16.usi"
for event in 'cpu_core/cycles,period=500009/u' 'cpu_core/branch-misses,period=5003/u'; do
  tag=$(echo "$event" | sed 's/cpu_core\/\([a-z-]*\).*/\1/')
  taskset -c 3 perf record -q -o "$OUT/bench16-$tag.data" --user-callchains --call-graph fp -e "$event" -- \
    "$BIN" --protocol usi < "$OUT/bench16.usi" > /dev/null
  rm -rf "$OUT/teacher-out"
  (cd "$MS3/cwd" && taskset -c 3 perf record -q -o "$OUT/teacher-$tag.data" --user-callchains --call-graph fp \
    -e "$(echo "$event" | sed 's/period=500009/period=2000003/; s/period=5003/period=20011/')" -- \
    "$BIN" data selfplay generate --eval-file "$MS3/net.bin" --output-dir "$OUT/teacher-out" --chunk-games 2 \
    --games 2 --seed 9010200000 --nodes 20000 --random-moves 0 --concurrency 1 --hash-mb 16 > /dev/null)
done
rm -rf "$OUT/teacher-out"
