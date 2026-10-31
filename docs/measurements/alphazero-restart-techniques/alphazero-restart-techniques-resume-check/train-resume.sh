#!/usr/bin/env bash
# 作業機で新しい構成の学習を、強制終了して再開した場合と止めない場合で比べ、続けて2サイクル目を学ぶ。
set -euo pipefail
W=/home/stepney141/board-games/minase/data/worktrees/alphazero-restart-techniques
O=$W/data/alphazero-restart-techniques/resume-check
I=$W/data/alphazero-restart-techniques/net-check/cycle-0000/checkpoint.pt
C=$(git -C "$W" rev-parse HEAD)
cd "$W/tools/train/zero"
mapfile -t files < <(python3 -c "import json,sys; print('\n'.join('$O/gen/'+f for f in json.load(open('$O/gen/summary.json'))['training_files']))")
train() {  # <出力先> <初期> <サイクル> <シード>
  uv run --frozen python -m minase_zero train --device cuda --init "$2" --data "${files[@]}" --latest-data "${files[@]}" \
    --out-dir "$1" --cycle "$3" --seed "$4" --batch-size 256 --checkpoint-steps 50 --ema-period-samples 0 --ema-scale 0 \
    --lr 0.01 --momentum 0.9 --weight-decay 0.0001 --clip-grad-norm 10 --value-lambda 0.333 --precision bf16 \
    --lr-schedule cosine --trainer-commit "$C"
}
echo "commit $C"
setsid bash -c "$(declare -f train); $(declare -p files I O C W); cd '$W/tools/train/zero'; train '$O/train-b' '$I' 1 20261101" > "$O/train-b-run1.log" 2>&1 < /dev/null &
until grep -q "^steps=100/" "$O/train-b-run1.log"; do
  pgrep -f "[m]inase_zero train" > /dev/null || { echo "trainer exited before step 100"; cat "$O/train-b-run1.log"; exit 1; }
  sleep 0.1
done
pkill -KILL -f "[m]inase_zero train"; sleep 1
pgrep -af "[m]inase_zero train" && { echo "trainer still running"; exit 1; }
echo "killed after steps=100"
uv run --frozen python -m minase_zero train --resume "$O/train-b" --device cuda > "$O/train-b-run2.log" 2>&1
train "$O/train-a" "$I" 1 20261101 > "$O/train-a.log" 2>&1
sha256sum "$O"/train-a/model.onnx "$O"/train-b/model.onnx "$O"/train-a/checkpoint.pt "$O"/train-b/checkpoint.pt
train "$O/train-c2" "$O/train-a/checkpoint.pt" 2 20261102 > "$O/train-c2.log" 2>&1
python3 - "$O" <<'PY'
import json, sys
o = sys.argv[1]
a, b, c = (json.load(open(f"{o}/{d}/summary.json")) for d in ("train-a", "train-b", "train-c2"))
skip = {"elapsed_seconds", "peak_rss_bytes", "environment", "timings", "wall_seconds", "out_dir"}
diff = sorted(k for k in set(a) | set(b) if k not in skip and a.get(k) != b.get(k))
print("summary keys differing between train-a and train-b:", diff)
for k in ("total_steps", "completed_steps", "sample_count", "training_count", "positions_seen", "samples_seen_start", "samples_seen", "last_lr", "data_order_sha256", "sample_sha256"):
    print(k, a.get(k), b.get(k), c.get(k))
PY
