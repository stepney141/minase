#!/usr/bin/env bash
set -euo pipefail
mkdir -p /workspace/job/logs || printf 'ログディレクトリを作成できません\n' >&2
nohup setsid bash -s -- @DEADLINE@ > /dev/null 2>&1 <<'STOP' &
set -euo pipefail
exec >> /workspace/job/logs/onstart.log 2>&1 || printf '自己停止のログを開けません\n' >&2
: "${CONTAINER_ID:?CONTAINER_ID is required}" "${CONTAINER_API_KEY:?CONTAINER_API_KEY is required}"
delay=$(($1 - $(date +%s)))
if ((delay > 0)); then sleep "$delay"; fi
end=$((SECONDS + 1800))
while ((SECONDS < end)); do
    remaining=$((end - SECONDS))
    ((remaining > 0)) || break
    limit=60
    if ((remaining < limit)); then limit=$remaining; fi
    rc=0
    code=$(curl -sS --max-time "$limit" -o /workspace/job/logs/selfstop-response -w '%{http_code}' -X PUT -H "Authorization: Bearer $CONTAINER_API_KEY" -H 'Content-Type: application/json' -d '{"state":"stopped"}' "https://console.vast.ai/api/v0/instances/$CONTAINER_ID/") || rc=$?
    printf '%s id=%s curl=%s http=%s\n' "$(date -u +%FT%TZ)" "$CONTAINER_ID" "$rc" "$code"
    if ((rc == 0)); then
        case "$code" in
            2??) exit 0 ;;
            4??) exit 1 ;;
            5??) ;;
            *) exit 1 ;;
        esac
    fi
    remaining=$((end - SECONDS))
    if ((remaining <= 0)); then break; fi
    if ((remaining > 60)); then remaining=60; fi
    sleep "$remaining"
done
echo "id=$CONTAINER_ID self-stop unconfirmed" >&2
exit 1
STOP
# 権限の修正に失敗しても、すでに予約した自己停止を妨げない。
repair_permissions() {
    if ! "$@"; then
        printf '起動時の修正に失敗: %s\n' "$*" >> /workspace/job/logs/onstart.log || printf '起動時の修正に失敗: %s\n' "$*" >&2
    fi
}
repair_permissions chown root:root /root /root/.ssh /root/.ssh/authorized_keys
repair_permissions chmod 700 /root /root/.ssh
repair_permissions chmod 600 /root/.ssh/authorized_keys
repair_permissions touch /root/.no_auto_tmux
