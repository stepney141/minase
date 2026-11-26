#!/bin/bash
# Kaniが起動するCBMCのコマンドラインから`--json-ui`だけを除いて、同じGOTOバイナリを検査する。
# 使い方: plain.sh 名前 ハーネス名 上限秒。<ハーネス名>.cmdline は実行中のcbmcの/proc/PID/cmdlineを保存したもの。
cd "$(dirname "$0")"
name=$1 limit=$3
mapfile -t args < <(grep -v -x -e '--json-ui' -e 'cbmc' "$2.cmdline")
( while sleep 30; do printf '%s %s\n' "$(date +%T)" "$(/usr/bin/ps -C cbmc -o rss= | tr '\n' ' ')"; done ) > $name-plain.rss &
sampler=$!
echo "$(date +%T) start $name limit=${limit}s"
( ulimit -v 18000000
  timeout $limit kani-home/kani-0.68.0/bin/cbmc "${args[@]}" 2>&1 \
    | while IFS= read -r line; do printf '%s %s\n' "$(date +%T)" "$line"; done > $name-plain.log
  echo "$(date +%T) end $name exit=${PIPESTATUS[0]}" )
kill $sampler 2>/dev/null
