#!/bin/bash
# 1問をfmcsで解く。引数: 問題ID。環境変数 B（測定ディレクトリ）と LT（制限時間、分）を使う。
set -u
id=$1; d=$B/runs/$id
rm -rf "$d"; mkdir -p "$d"; cp "$B/problems/$id.fmi" "$d/p.fmi"; cd "$d"
start=$(date +%s.%N)
WINEPREFIX=$B/wineprefix WINEDEBUG=-all /usr/bin/time -v -o time.txt timeout $((LT*60+120)) \
  script -f -q -e -c "wine $B/bin/fmcs.exe /Ip.fmi /Oout.fmo /L2 /LT=${LT}M /XV" screen.raw.log < /dev/null > /dev/null 2>&1
echo "exit=$? wall=$(echo "$(date +%s.%N) - $start" | bc)" > status.txt
