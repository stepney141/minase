#!/usr/bin/env bash
# 使い方: source scripts/vast/common.sh
# vast.aiの作成、状態確認、転送、停止と削除を共通化する。
set -euo pipefail

vast_error() { printf '%s\n' "$*" >&2; return 1; }

# 引数: ロックファイル、スクリプトのパス、スクリプトの引数。
# flockだけがロックを保持し、再実行するスクリプトと子孫には記述子を渡さない。
vast_session_lock() {
    local lock=$1 script=$2
    shift 2
    if [[ -v MINASE_VAST_SESSION_LOCK ]]; then
        [[ $MINASE_VAST_SESSION_LOCK == "$lock" ]] || { vast_error '再実行時のセッションロックが一致しません'; return 1; }
        unset MINASE_VAST_SESSION_LOCK
        return 0
    fi
    # --verboseの競合通知だけを従来の日本語へ変換する。
    # flockの成功時の出力を捨て、実処理の標準出力は記述子3から元へ戻す。
    exec env MINASE_VAST_SESSION_LOCK="$lock" LC_ALL=C flock -n -o --verbose "$lock" \
        bash -c 'exec bash "$@" >&3 3>&-' bash "$script" "$@" \
        3>&1 > /dev/null 2> >(sed -u 's/^flock: failed to get lock$/同じセッションの起動または取得が実行中です/' >&2)
}

# 引数: 秒数、vastaiの引数。通信の失敗と時間超過を成功扱いしない。
vast_call() {
    local seconds=$1
    shift
    [[ $seconds =~ ^[1-9][0-9]*$ ]] || { vast_error "vastaiの上限秒数には正の整数が必要です: $seconds"; return 1; }
    timeout --kill-after=5s "${seconds}s" vastai "$@"
}

# 引数: ラベル。該当するIDを1行ずつ出す。
vast_ids() {
    vast_call 600 show instances --raw | python3 -c '
import json, sys
for instance in json.load(sys.stdin):
    if instance["label"] == sys.argv[1]:
        print(int(instance["id"]))
' "$1"
}

# 引数: ID、照会の上限秒数。削除済みならnullを出す。
vast_instance() {
    vast_call "$2" show instances --raw | python3 -c '
import json, sys
matches = [i for i in json.load(sys.stdin) if str(i["id"]) == sys.argv[1]]
if len(matches) > 1:
    sys.exit("同じIDのインスタンスが複数あります")
print(json.dumps(matches[0] if matches else None))
' "$1"
}

# 引数: 出品ID、ラベル、起動時スクリプト、イメージ、ディスクGB。
vast_create() {
    vast_call 600 create instance "$1" --image "$4" --disk "$5" \
        --ssh --direct --cancel-unavail --label "$2" --onstart "$3" --raw |
        python3 -c '
import json, sys
value = json.load(sys.stdin)["new_contract"]
if type(value) is not int or value <= 0:
    sys.exit("作成応答に正しいnew_contractがありません")
print(value)
'
}

# 引数: ID。操作と確認を合わせて10分以内に収める。
vast_stop() {
    local id=$1 end=$((SECONDS + 600)) state remaining
    if ! vast_call 600 stop instance "$id" >&2; then
        vast_error "インスタンス $id: 停止要求が失敗し、停止を確認できません"; return 1
    fi
    while :; do
        remaining=$((end - SECONDS))
        ((remaining > 0)) || break
        if ! state=$(vast_instance "$id" "$remaining"); then
            vast_error "インスタンス $id: 停止の状態照会に失敗しました"; return 1
        fi
        if python3 -c '
import json, sys
i = json.loads(sys.argv[1])
sys.exit(0 if i is not None and i["intended_status"] == "stopped" and i["actual_status"] in ("stopped", "exited") else 1)
' "$state"; then return 0; fi
        sleep 5
    done
    vast_error "インスタンス $id: 10分以内に停止を確認できません"
}

# 引数: ID。削除要求の応答だけでなく一覧からの消失を確認する。
vast_destroy() {
    local id=$1 end=$((SECONDS + 600)) state remaining
    if ! vast_call 600 destroy instance "$id" -y >&2; then
        vast_error "インスタンス $id: 削除要求が失敗し、削除を確認できません"; return 1
    fi
    while :; do
        remaining=$((end - SECONDS))
        ((remaining > 0)) || break
        if ! state=$(vast_instance "$id" "$remaining"); then
            vast_error "インスタンス $id: 削除の状態照会に失敗しました"; return 1
        fi
        [[ $state != null ]] || return 0
        sleep 5
    done
    vast_error "インスタンス $id: 10分以内に削除を確認できません"
}

# 引数: ID、上限秒数。起動に失敗したインスタンスは削除する。
vast_wait_running() {
    local id=$1 end=$((SECONDS + $2)) state remaining
    while :; do
        remaining=$((end - SECONDS))
        ((remaining > 0)) || break
        if ! state=$(vast_instance "$id" "$remaining"); then break; fi
        if ! state=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["actual_status"])' "$state"); then break; fi
        case "$state" in
            running) return 0 ;;
            stopped|exited|offline|unknown) break ;;
        esac
        sleep 5
    done
    printf 'インスタンス %s: 起動を確認できないため削除します\n' "$id" >&2
    vast_destroy "$id" || return 1
    return 1
}

# 引数: ID、上限秒数。公開IPとSSHポートを空白区切りで出す。
vast_ssh_address() {
    local id=$1 end=$((SECONDS + $2)) state address remaining
    while :; do
        remaining=$((end - SECONDS))
        ((remaining > 0)) || break
        state=$(vast_instance "$id" "$remaining") || return 1
        address=$(python3 -c '
import ipaddress, json, sys
i = json.loads(sys.argv[1])
if i is None:
    sys.exit("インスタンスがありません")
if i["public_ipaddr"] and i["ports"] and "22/tcp" in i["ports"] and i["ports"]["22/tcp"]:
    ip = ipaddress.ip_address(i["public_ipaddr"])
    port = int(i["ports"]["22/tcp"][0]["HostPort"])
    if not 1 <= port <= 65535:
        sys.exit("SSHポートが不正です")
    print(ip, port)
' "$state") || return 1
        if [[ -n $address ]]; then printf '%s\n' "$address"; return 0; fi
        sleep 5
    done
    vast_error "インスタンス $id: SSH接続先の割り当てを確認できません"
}

# 引数: IP、ポート、リモートコマンド。
vast_ssh() {
    local host=$1 port=$2
    shift 2
    ssh -i "$HOME/.ssh/id_vastai" -p "$port" -o BatchMode=yes \
        -o StrictHostKeyChecking=accept-new -o ConnectTimeout=20 \
        -o ServerAliveInterval=30 "root@$host" "$@"
}

# 引数: ID、IP、ポート。SSHが使用可能になるまで最大5分、5秒おきに確認する。
# 失敗した場合の削除は、呼出し元の作成処理が行う。
vast_wait_ssh() (
    local id=$1 host=$2 port=$3 end=$((SECONDS + 300)) remaining
    export -f vast_ssh
    while :; do
        remaining=$((end - SECONDS))
        ((remaining > 0)) || break
        if timeout --kill-after=5s "${remaining}s" bash -c 'vast_ssh "$@"' -- "$host" "$port" true; then
            return 0
        fi
        printf 'インスタンス %s: SSHの準備を確認できないため再試行します\n' "$id" >&2
        remaining=$((end - SECONDS))
        ((remaining > 0)) || break
        if ((remaining > 5)); then remaining=5; fi
        sleep "$remaining"
    done
    vast_error "インスタンス $id: 5分以内にSSHの準備を確認できません"
)

# 引数: IP、ポート、リモートの宛先パス、送信元（複数可）。
vast_scp() {
    local host=$1 port=$2 destination=$3
    shift 3
    scp -r -i "$HOME/.ssh/id_vastai" -P "$port" -o BatchMode=yes \
        -o StrictHostKeyChecking=accept-new -o ConnectTimeout=20 \
        -o ServerAliveInterval=30 "$@" "root@[$host]:$destination"
}

# 引数: ID、調整機のパス、測定機の絶対パス。
# コンテナのSSH経由で取得する。再起動に備え、接続先は転送ごとに照会する。
vast_rsync() {
    [[ $3 == /* ]] || { vast_error "取得先は絶対パスで指定してください: $3"; return 1; }
    local address host port ssh_command
    address=$(vast_ssh_address "$1" 600) || return 1
    read -r host port <<< "$address"
    if [[ $host == *:* ]]; then host="[$host]"; fi
    printf -v ssh_command 'ssh -i "%s" -p %s -o BatchMode=yes -o StrictHostKeyChecking=accept-new -o ConnectTimeout=20 -o ServerAliveInterval=30' \
        "$HOME/.ssh/id_vastai" "$port"
    timeout --kill-after=5s 3600s rsync -a -e "$ssh_command" "root@$host:$2" "$3"
}

# 引数: ID、調整機のパス、測定機の絶対パス。
vast_copy() (
    [[ $3 == /* ]] || { vast_error "取得先は絶対パスで指定してください: $3"; return 1; }
    local output rc=0
    output=$(mktemp) || return 1
    trap 'rm -f "$output"' EXIT
    LC_ALL=C vast_call 3600 copy -i "$HOME/.ssh/id_vastai" "C.$1:$2" "local:$3" > "$output" 2>&1 || rc=$?
    cat "$output" || return 1
    ((rc == 0)) || { vast_error "インスタンス $1: 取得が失敗しました（終了コード $rc）"; return "$rc"; }
    vast_copy_complete "$output" || { vast_error "インスタンス $1: rsyncの取得完了を確認できません"; return 1; }
)

# 現行vastaiはrsyncの終了コードを伝播しないため、転送結果も確認する。
vast_copy_complete() {
    python3 - "$1" <<'PY'
from pathlib import Path
import re, sys
text = Path(sys.argv[1]).read_text(errors="replace")
complete = re.search(r"^total size is [0-9,]+\s+speedup is [0-9,.]+\s*$", text, re.M)
failed = re.search(r"rsync (?:error|warning):", text)
sys.exit(0 if complete and not failed else 1)
PY
}

# 引数: 停止期限のUNIX時刻。起動時スクリプトの共通部分を出す。
vast_onstart_prefix() {
    [[ $1 =~ ^[1-9][0-9]*$ ]] || { vast_error '停止期限が不正です'; return 1; }
    cat <<'SH'
#!/usr/bin/env bash
set -euo pipefail
mkdir -p /workspace/job/logs || printf 'ログディレクトリを作成できません\n' >&2
SH
    printf 'nohup setsid bash -s -- %s > /dev/null 2>&1 <<\x27STOP\x27 &\n' "$1"
    cat <<'SH'
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
SH
}
