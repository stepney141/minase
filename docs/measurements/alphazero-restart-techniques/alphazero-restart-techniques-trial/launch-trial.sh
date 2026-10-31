#!/usr/bin/env bash
# Colab機のL4を1台借り、新しい構成の別系列trial-restartのサイクル1を実行器で回し、終了コードによらず止める。
# 使い方: launch-trial.sh <セッション名> <コミット40桁>
# 作業機で nohup setsid bash launch-trial.sh ... > <ログ> 2>&1 & として起動する。
set -u
session=$1; C=$2
R=/home/stepney141/board-games/minase/data/worktrees/alphazero-restart-techniques
T=$R/docs/measurements/alphazero-rl/alphazero-rl-trial
Z=$R/tools/train/zero
A=/mnt/synology_nas/minase/alphazero
[ -e "$A/src/minase-$C.tar" ] || git -C "$R" archive --format=tar -o "$A/src/minase-$C.tar" "$C"
colab new -s "$session" --gpu L4 || exit 1
rented=$(date -Is)
balance=$(colab usage 2>&1 | sed -n 's/Current balance: \([0-9.]*\).*/\1/p')
printf '{"session":"%s","gpu_requested":"L4","rented_at":"%s","balance_before":%s,"purpose":"trial of the restart techniques (trial-restart cycle 1)"}\n' \
    "$session" "$rented" "$balance" >> "$A/rentals.jsonl"
deadline=$(date -d "$rented + 3 hours" -Is)
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
timeout 60 "$T/rsh.sh" "$session" 'cat /content/setup.log' < /dev/null
cd "$Z" && uv run --frozen python -m minase_zero run-cycles --root "$A" --session "$session" \
    --cycles-dir trial-restart --base-cycles-dir trial-restart --branch-from 0 \
    --cycle1-reference-summary "$A/trial-restart/reference-selfplay-summary.json" \
    --generation-commit "$C" --trainer-commit "$C" --first-cycle 1 --last-cycle 1 \
    --deadline "$deadline" --cycle-minutes 100 --cycle-cu 2.5 \
    --target-positions 1000000 --chunks 20 --simulations 32 --threads 4 --games-per-thread 128 \
    --window-positions 47802000 --batch-size 256 --lr 0.01 --momentum 0.9 --weight-decay 0.0001 \
    --clip-grad-norm 10 --value-lambda 0.333 --precision bf16 --checkpoint-steps 200 --lr-schedule cosine \
    --train-seed-base 20261100 --teacher-stats "$Z/feasibility/target/release/teacher_stats" \
    --poll-seconds 60 --selfplay-weights raw --ema-period-samples 0 --ema-scale 0
rc=$?
echo "runner exit $rc at $(date -Is)"
colab stop -s "$session"
balance=$(colab usage 2>&1 | sed -n 's/Current balance: \([0-9.]*\).*/\1/p')
printf '{"session":"%s","stopped_at":"%s","balance_after":%s,"note":"runner exit code %s"}\n' \
    "$session" "$(date -Is)" "$balance" "$rc" >> "$A/rentals.jsonl"
colab sessions
echo "chain-done"
