#!/usr/bin/env bash
# 世代反復の第N世代のネットを、作業機で診断する。
# 使い方: diagnose-generation.sh <世代N>
# 評価値の分布、合法手の評価差、および第0世代とのあいだの対局単位の汎化差を、
# data/nnue-generation-iteration/gen-<N>/diag/ に書く。
# 合法手の評価差の局面は、全世代で第1世代の番号0の教師ファイルに固定する。
# 対局単位の汎化差は、同じディレクトリの outcome_gap.rs をビルドした集計器と、
# docs/measurements/nnue-architecture-revision/nnue-architecture-revision-init-training/gap_report.py で求める。
set -euo pipefail
generation=$1
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
"$T" diag nnue-distribution --pair gen0 "$PST" "$V0" --pair "gen$generation" "$PST" "$NET" \
  --data "$G"/teacher/book-*.mnsd --sample 10000 --seed 1 --output "$G/diag/distribution.json"
"$T" diag nnue-moves --net "$NET" --base "$PST" \
  --positions "$D/gen-1/teacher/book-0.mnsd" --output "$G/diag/moves.json"
"$D/outcome-gap-target/release/outcome-gap" "$G/diag/gap.csv" --scale "$SCALE" \
  --net "gen0=$V0" --net "gen$generation=$NET" --data "$G"/teacher/book-*.mnsd
python3 -I "$W/docs/measurements/nnue-architecture-revision/nnue-architecture-revision-init-training/gap_report.py" \
  "$G/diag/gap.csv" > "$G/diag/gap_report.out"
echo "診断完了"
