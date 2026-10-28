#!/usr/bin/env bash
# 試走：ランダムに初期化したネットで、スレッド数とスレッドごとの同時対局数の5通りの組を順に生成する。
# 投了は無効にし、各組の対局数は同時対局数の4倍、塊の対局数は同時対局数の半分とする。
# 使い方: trial.sh <生成コミット>
set -euo pipefail
commit=$1
run() {
    bash /content/gen.sh "trial/$1" cycles/cycle-0000/model.onnx "$commit" "$2" "$3" "$4" "$5" "$6" 100
}
run t2g128 101000000 1024 128 2 128
run t2g256 102000000 2048 256 2 256
run t4g64 103000000 1024 128 4 64
run t4g128 104000000 2048 256 4 128
run t6g64 105000000 1536 192 6 64
echo trial-done
