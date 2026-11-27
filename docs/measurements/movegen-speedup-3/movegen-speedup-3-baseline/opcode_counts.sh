#!/usr/bin/env bash
# リリースのバイナリから、後段の関数`LayerStackNetwork::dense`を逆アセンブルし、命令名ごとの出現数を数える。
#
#   opcode_counts.sh <バイナリ> [<関数名の正規表現>]
#
# 関数名の既定値は`<minase::eval::nnue::network::layer_stack::LayerStackNetwork>::dense`である。
# インライン展開されずに残った呼び出し先（たとえば`call`の先）は数えない。
set -euo pipefail
BIN=$1
NAME=${2:-'<minase::eval::nnue::network::layer_stack::LayerStackNetwork>::dense'}
ADDR=$(nm -C "$BIN" | awk -v n="$NAME" '{ sym = $0; sub(/^[0-9a-f]+ [a-zA-Z] /, "", sym); if (sym == n) print $1 }')
SIZE=$(nm -C -S "$BIN" | awk -v n="$NAME" '{ sym = $0; sub(/^[0-9a-f]+ [0-9a-f]+ [a-zA-Z] /, "", sym); if (sym == n) print $2 }')
if [ -z "$ADDR" ] || [ -z "$SIZE" ]; then
  echo "symbol not found: $NAME" >&2
  exit 1
fi
START=$((16#$ADDR))
STOP=$((START + 16#$SIZE))
objdump -d -M intel --no-show-raw-insn --start-address="$START" --stop-address="$STOP" "$BIN" \
  | awk -F'\t' 'NF >= 2 && $1 ~ /^ *[0-9a-f]+:$/ { split($2, a, " "); print a[1] }' \
  | sort | uniq -c | sort -rn
