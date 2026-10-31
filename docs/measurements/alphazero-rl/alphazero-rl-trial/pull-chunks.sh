#!/usr/bin/env bash
# Colab機の生成の出力先 /content/az/<出力先> から、確定した塊のファイル（chunk-*.json、chunk-*.mnsd、
# chunk-*.resigned.mnsd）と生成条件 config.json のうち、NASにまだないものを pull.sh で受け取る。
# 塊は集計の chunk-*.json を最後に確定させるので、集計があり3つがそろった塊だけを受け取る。
# 使い方: pull-chunks.sh <セッション名> <出力先の相対パス>
set -euo pipefail
here=$(dirname -- "$0")
session=$1
out=$2
root=/mnt/synology_nas/minase/alphazero
listing=$("$here/rsh.sh" "$session" "cd /content/az/$out && ls")
files=()
[[ -e $root/$out/config.json ]] || files+=("$out/config.json")
for json in $(grep -E '^chunk-[0-9]+\.json$' <<< "$listing"); do
    stem=${json%.json}
    for name in "$stem.mnsd" "$stem.resigned.mnsd" "$json"; do
        grep -qx -- "$name" <<< "$listing" || { echo "incomplete chunk on Colab: $out/$stem" >&2; exit 1; }
        [[ -e $root/$out/$name ]] || files+=("$out/$name")
    done
done
if (( ${#files[@]} )); then
    "$here/pull.sh" "$session" "${files[@]}"
fi
