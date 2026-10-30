#!/usr/bin/env bash
# rl-c21: 導入の完了を待ち、サイクル21〜30を回し、終了の仕方によらずセッションを止める。
set -u
T=/home/stepney141/board-games/minase/data/worktrees/alphazero-rl/docs/measurements/alphazero-rl/alphazero-rl-trial
Z=/home/stepney141/board-games/minase/data/worktrees/alphazero-rl/tools/train/zero
A=/mnt/synology_nas/minase/alphazero
C=4ec063c1b75190af80dc47f885fe5b20a2b37364
until timeout 60 "$T/rsh.sh" rl-c21 'grep -q setup-done /content/setup.log' < /dev/null; do
    timeout 60 "$T/rsh.sh" rl-c21 'pgrep -f [c]olab-setup.sh > /dev/null' < /dev/null || { echo "setup failed"; break; }
    sleep 20
done
timeout 60 "$T/rsh.sh" rl-c21 'grep -E "^20|Prepared|Finished" /content/setup.log' < /dev/null
cd "$Z" && uv run --frozen python -m minase_zero run-cycles --root "$A" --session rl-c21 \
    --generation-commit "$C" --trainer-commit "$C" --first-cycle 21 --last-cycle 30 \
    --deadline 2026-10-30T22:58:17+09:00 --cycle-minutes 45 --cycle-cu 1.2 \
    --target-positions 1000000 --chunks 20 --simulations 32 --threads 4 --games-per-thread 128 \
    --window-positions 34625240 --batch-size 256 --lr 0.01 --momentum 0.9 --weight-decay 0.0001 \
    --clip-grad-norm 10 --value-lambda 0.333 --precision bf16 --checkpoint-steps 200 \
    --train-seed-base 20261100 \
    --teacher-stats /home/stepney141/board-games/minase/data/worktrees/alphazero-rl/tools/train/zero/feasibility/target/release/teacher_stats \
    --poll-seconds 60 --selfplay-weights raw --ema-period-samples 500000 --ema-scale 8
rc=$?
echo "runner exit $rc at $(date -Is)"
colab stop -s rl-c21
B=$(colab usage 2>&1 | sed -n 's/Current balance: \([0-9.]*\).*/\1/p')
printf '{"session":"rl-c21","stopped_at":"%s","balance_after":%s,"note":"runner exit code %s"}\n' "$(date -Is)" "$B" "$rc" >> "$A/rentals.jsonl"
colab sessions
echo "chain-done"
