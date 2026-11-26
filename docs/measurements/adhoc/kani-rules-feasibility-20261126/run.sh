#!/bin/bash
# 各ハーネスを順に実行し、行ごとに時刻を付けた全出力と、30秒ごとのcbmcの常駐メモリを残す。
cd "$(dirname "$0")/core"
export KANI_HOME=$PWD/../kani-home PATH=$PWD/../cargo-root/bin:$PATH CARGO_BUILD_JOBS=4
run() { # 名前 完全修飾ハーネス名 上限秒
  local name=$1 harness=$2 limit=$3
  ( while sleep 30; do printf '%s %s\n' "$(date +%T)" "$(/usr/bin/ps -C cbmc -o rss= | tr '\n' ' ')"; done ) > ../$name.rss &
  local sampler=$!
  echo "$(date +%T) start $name limit=${limit}s"
  timeout $limit cargo kani --harness $harness --exact 2>&1 \
    | while IFS= read -r line; do printf '%s %s\n' "$(date +%T)" "$line"; done > ../$name.log
  echo "$(date +%T) end $name exit=${PIPESTATUS[0]}"
  kill $sampler 2>/dev/null
  /usr/bin/pkill -x cbmc 2>/dev/null
}
run promotion promotion::kani_proofs::standard_promotion_choice 300
run sliding-symbolic attacks::tables::kani_proofs::sliding_symbolic_origin 1800
run sliding-production attacks::tables::kani_proofs::sliding_production_tables 900
run sliding-enumerated attacks::tables::kani_proofs::sliding_enumerated_origin 1800
