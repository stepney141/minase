#!/usr/bin/env bash
# 世代反復の第N世代のネットを、作業機で診断する。
# 使い方: diagnose-generation.sh <世代N>
# 評価値の分布、合法手の評価差、および第0世代とのあいだの対局単位の汎化差を、
# data/nnue-generation-iteration/gen-<N>/diag/ に書く。
# 第2世代からは、直前の世代のネットも同じ標本で比べる。
# 評価値の分布は全部の本をメモリに開くので、番号がNの倍数の本だけ（約43本）を渡して、作業機のメモリに収める。
# 合法手の評価差の局面は、全世代で data/nnue-generation-iteration/diag-positions/human-1000/teacher.mnsd に固定する。
# このファイルは、人間の対局の中盤の局面967個（data/strength-stage9/human-trial/openings-1000.txt、
# SHA-256は b0d77584…）を開始局面として、反復のコミットの生成器で1手100ノードの対局を生成したものであり、
# 診断は各対局の開始局面だけを使う。教師ファイルを渡すと、全対局の開始局面が初期局面になるからである。
# 生成のコマンド:
#   minase data selfplay generate --eval-file <v0のネット> --output-dir <出力先> --chunk-games 250 \
#     --seed 20261121 --openings <局面一覧> --nodes 100 --random-moves 0 --concurrency 2 --max-ply 4000 --hash-mb 16
# 着手ごとの集計は、同じディレクトリの moves_summary.py で求める。
# 対局単位の汎化差は、同じディレクトリの outcome_gap.rs をビルドした集計器と、
# docs/measurements/nnue-architecture-revision/nnue-architecture-revision-init-training/gap_report.py で求める。
set -euo pipefail
generation=$1
previous=$((generation - 1))
R=/home/stepney141/board-games/minase
D=$R/data/nnue-generation-iteration
W=$R/data/worktrees/nnue-generation-iteration
T=$D/bin/minase-trainer
PST=$W/crates/minase/nets/pst.bin
G=$D/gen-$generation
V0=$(python3 -c "import json; print(json.load(open('$D/gen-0/generation.json'))['network_path'])")
NET=$(python3 -c "import json; print(json.load(open('$G/generation.json'))['network_path'])")
SCALE=$(python3 -c "import json; print(json.load(open('$G/generation.json'))['teacher_cp_per_logit'])")
mkdir -p "$G/diag"
cd "$W"
books=()
for ((i = 0; i < 43 * generation; i += generation)); do books+=("$G/teacher/book-$i.mnsd"); done
pairs=(--pair gen0 "$PST" "$V0")
nets=(--net "gen0=$V0")
if ((previous >= 1)); then
  PREV=$(python3 -c "import json; print(json.load(open('$D/gen-$previous/generation.json'))['network_path'])")
  pairs+=(--pair "gen$previous" "$PST" "$PREV")
  nets+=(--net "gen$previous=$PREV")
fi
"$T" diag nnue-distribution "${pairs[@]}" --pair "gen$generation" "$PST" "$NET" \
  --data "${books[@]}" --sample 10000 --seed 1 --output "$G/diag/distribution.json"
"$T" diag nnue-moves --net "$NET" --base "$PST" \
  --positions "$D/diag-positions/human-1000/teacher.mnsd" --output "$G/diag/moves.json"
python3 -I "$(dirname "$0")/moves_summary.py" "$G/diag/moves.json" > "$G/diag/moves_summary.out"
"$D/outcome-gap-target/release/outcome-gap" "$G/diag/gap.csv" --scale "$SCALE" \
  "${nets[@]}" --net "gen$generation=$NET" --data "$G"/teacher/book-*.mnsd
python3 -I "$W/docs/measurements/nnue-architecture-revision/nnue-architecture-revision-init-training/gap_report.py" \
  "$G/diag/gap.csv" > "$G/diag/gap_report.out"
echo "診断完了"
