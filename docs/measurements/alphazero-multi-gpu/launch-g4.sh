#!/usr/bin/env bash
# ColabのG4を1台借りて導入し、複数GPU化の設計書のフェーズ4の1サイクル（系列multi-gpu-g4のサイクル1）を回す。
# 続けて約3,000万局面の読み込みの時間を測り、終了コードによらず機械を止める。
# 使い方: launch-g4.sh <セッション名> <コミット40桁> <結果を置く作業機のディレクトリ>
# 作業機で nohup setsid bash launch-g4.sh ... > <ログ> 2>&1 & として起動する。
# 集計には作業ツリーのtarget/release/minaseを使うので、起動前にcargo build --release --bin minaseを実行する。
set -u
session=$1; C=$2; out=$3
R=$(git -C "$(dirname "$(readlink -f "$0")")" rev-parse --show-toplevel)
H=$R/docs/measurements/alphazero-multi-gpu
T=$R/docs/measurements/alphazero-rl/alphazero-rl-trial
Z=$R/tools/train/zero
A=/mnt/synology_nas/minase/alphazero
mkdir -p "$out" || exit 1
"$R/target/release/minase" data teacher-stats --help > /dev/null || exit 1
[ -e "$A/src/minase-$C.tar" ] || git -C "$R" archive --format=tar -o "$A/src/minase-$C.tar" "$C"
colab new -s "$session" --gpu G4 || exit 1
rented=$(date -Is)
balance=$(colab usage 2>&1 | sed -n 's/Current balance: \([0-9.]*\).*/\1/p')
printf '{"session":"%s","gpu_requested":"G4","rented_at":"%s","balance_before":%s,"purpose":"multi-gpu phase 4 cycle 1 and loading"}\n' \
    "$session" "$rented" "$balance" >> "$A/rentals.jsonl"
deadline=$(date -d "$rented + 8 hours" -Is)
for i in 1 2 3 4 5 6; do
    timeout 150 "$T/rsh.sh" "$session" 'nvidia-smi --query-gpu=name --format=csv,noheader' < /dev/null && break
    sleep 20
done
rsh="ssh -o ProxyCommand='colab ssh --proxy-mode -s $session' -o ControlMaster=auto -o ControlPath=/tmp/colab-cm-$session -o ControlPersist=900 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
rsync -a -e "$rsh" "$A/src/minase-$C.tar" "$T/colab-setup.sh" root@colab:/content/
timeout 60 "$T/rsh.sh" "$session" "setsid nohup bash /content/colab-setup.sh /content/minase-$C.tar > /content/setup.log 2>&1 < /dev/null &" < /dev/null
until timeout 60 "$T/rsh.sh" "$session" 'grep -q setup-done /content/setup.log' < /dev/null; do
    timeout 60 "$T/rsh.sh" "$session" 'pgrep -f [c]olab-setup.sh > /dev/null' < /dev/null || { echo "setup failed"; break; }
    sleep 20
done
rsync -a -e "$rsh" root@colab:/content/setup.log "$out/setup.log"
started=$(date +%s)
(cd "$Z" && uv run --frozen python -m minase_zero run-cycles --machine colab --root "$A" --session "$session" \
    --cycles-dir multi-gpu-g4 --base-cycles-dir cycles-rl2 --branch-from 0 \
    --cycle1-reference-summary "$A/cycles/cycle-0001/selfplay/summary.json" \
    --generation-commit "$C" --trainer-commit "$C" --first-cycle 1 --last-cycle 1 \
    --deadline "$deadline" --cycle-minutes 90 --cycle-cu 14 \
    --start-position shuffled-setup --generation-seed-offset 150000000 \
    --target-positions 1000000 --chunks 20 --simulations 32 --threads-per-gpu 4 --games-per-thread 128 \
    --replay-buffer-first-cycle 1 --replay-buffer-positions 280520080 \
    --batch-size 1024 --lr 0.04 --momentum 0.9 --weight-decay 0.0001 --clip-grad-norm 10 \
    --value-lambda 0.333 --precision bf16 --checkpoint-steps 50 \
    --train-seed-base 20261100 --minase "$R/target/release/minase" --poll-seconds 60)
rc=$?
echo "runner exit $rc at $(date -Is) after $(( $(date +%s) - started )) s"
# 読み込みの測定: サイクル3〜31の生成ファイルを送り、学習器を最初の保存まで動かす。
transfer_started=$(date +%s)
rsync -a -e "$rsh" --files-from="$H/replay-lists/cycles-0003-0031.txt" "$A/" root@colab:/content/m/
transfer_seconds=$(( $(date +%s) - transfer_started ))
echo "transfer of cycles 3-31: $transfer_seconds s"
python3 - "$H/replay-lists" "$out" <<'EOF'
import json, sys
from pathlib import Path
lists, out = Path(sys.argv[1]), Path(sys.argv[2])
for name, source in (("buf.json", "cycles-0003-0031.txt"), ("latest.json", "cycle-0031.txt")):
    paths = ["/content/m/" + line for line in (lists / source).read_text().splitlines()]
    (out / name).write_text(json.dumps(paths) + "\n")
EOF
rsync -a -e "$rsh" "$out/buf.json" "$out/latest.json" "$A/cycles-rl2/cycle-0000/checkpoint.pt" root@colab:/content/m/
timeout 3600 "$T/rsh.sh" "$session" "cd /content/src/tools/train/zero && source env.sh && uv run --frozen python $(printf %q "/content/src/docs/measurements/alphazero-multi-gpu/load-check.py") --out /content/m/load --commit $C --init /content/m/checkpoint.pt --data-list /content/m/buf.json --latest-list /content/m/latest.json --nproc 1 --replay-threads \$(nproc)" < /dev/null > "$out/load.log" 2>&1
echo "load-check exit $? at $(date -Is)"
rsync -a -e "$rsh" root@colab:/content/m/load/result.json "$out/load-result.json"
printf '{"transfer_seconds":%s,"bytes":%s}\n' "$transfer_seconds" \
    "$(cd "$A" && xargs -d '\n' stat -c %s < "$H/replay-lists/cycles-0003-0031.txt" | paste -sd+ | bc)" > "$out/transfer.json"
colab stop -s "$session"
balance=$(colab usage 2>&1 | sed -n 's/Current balance: \([0-9.]*\).*/\1/p')
printf '{"session":"%s","stopped_at":"%s","balance_after":%s,"note":"runner exit code %s"}\n' \
    "$session" "$(date -Is)" "$balance" "$rc" >> "$A/rentals.jsonl"
colab sessions
echo "chain-done"
