#!/usr/bin/env bash
# 使い方: bash scripts/vast-spsa/launch.sh <セッション名> <出品ID> <期限までの分数>
# 設定と出品を検査し、bundleとジョブを転送して、すべて届いてから開始する。
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/vast/common.sh
source "$SCRIPT_DIR/../vast/common.sh"
# shellcheck source=scripts/vast-spsa/remote.sh
source "$SCRIPT_DIR/remote.sh"

# 引数: 出品JSONのファイル、要求した出品ID。raw出力のdurationは秒単位。
spsa_check_offer() {
    python3 - "$@" <<'PY'
import json, math, sys
offers = json.load(open(sys.argv[1]))
if len(offers) != 1 or str(offers[0]["id"]) != sys.argv[2]:
    sys.exit("指定した出品を一意に取得できません")
offer = offers[0]
errors = []
for key, expected in (("cpu_arch", "amd64"), ("verification", "verified")):
    if key not in offer or offer[key] != expected:
        errors.append(f"{key}は{expected}でなければなりません")
for key, minimum in (("cpu_cores", 192), ("duration", 172800), ("reliability2", 0.95), ("direct_port_count", 1), ("inet_up", 100), ("disk_space", 50)):
    if key not in offer or type(offer[key]) not in (int, float) or not math.isfinite(offer[key]) or offer[key] < minimum:
        errors.append(f"{key}は{minimum}以上でなければなりません")
if "gpu_frac" not in offer or offer["gpu_frac"] != 1:
    errors.append("gpu_fracは1でなければなりません")
if "cpu_cores_effective" not in offer or "cpu_cores" not in offer or offer["cpu_cores_effective"] != offer["cpu_cores"]:
    errors.append("cpu_cores_effectiveとcpu_coresが一致しません")
if errors:
    sys.exit("\n".join(errors))
PY
}

spsa_launch_main() (
    [[ $# == 3 ]] || { spsa_error '使い方: launch.sh <セッション名> <出品ID> <期限までの分数>'; exit 1; }
    spsa_name "$1"
    [[ $2 =~ ^[1-9][0-9]*$ && $3 =~ ^[1-9][0-9]*$ ]] || { spsa_error '出品IDと期限までの分数には正の整数が必要です'; exit 1; }
    local session=$1 offer=$2 minutes=$3 root directory label
    root=$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel)
    directory="$root/data/vast-spsa/$session"
    label="minase-spsa-$session"
    spsa_load_session "$directory"
    COMMIT=$(git -C "$root" rev-parse --verify --end-of-options "$COMMIT^{commit}")
    RUNNER_COMMIT=$(git -C "$root" rev-parse --verify --end-of-options "$RUNNER_COMMIT^{commit}")
    if [[ -d $directory/run ]]; then
        spsa_check_manifest "$directory/run/manifest.json"
    fi
    local command
    for command in python3 timeout vastai ssh scp rustc flock; do
        command -v "$command" > /dev/null || { spsa_error "必要なコマンドがありません: $command"; exit 1; }
    done
    [[ -r $HOME/.ssh/id_vastai ]] || { spsa_error "$HOME/.ssh/id_vastaiがありません"; exit 1; }
    # 起動と取得による、同一セッションのローカル記録の競合を防ぐ。
    exec 8> "$directory/.operation.lock"
    flock -n 8 || { spsa_error '同じセッションの起動または取得が実行中です'; exit 1; }
    local temporary instance='' cleanup_instance=no
    temporary=$(mktemp -d "$directory/.launch.XXXXXX")
    trap 'rc=$?; trap - EXIT; if [[ $cleanup_instance == yes ]]; then if ! vast_destroy "$instance"; then rc=1; fi; fi; rm -rf "$temporary"; exit "$rc"' EXIT
    vast_call 600 search offers "id=$offer" --raw > "$temporary/offer.json"
    spsa_check_offer "$temporary/offer.json" "$offer"
    local ids
    ids=$(vast_ids "$label")
    [[ -z $ids ]] || { spsa_error "同じラベルのインスタンスがあります: $ids"; exit 1; }
    python3 - "$minutes" > "$directory/deadline" <<'PY'
import sys, time
deadline = int(time.time()) + int(sys.argv[1]) * 60
if deadline >= 2**63:
    sys.exit("停止期限が大きすぎます")
print(deadline)
PY
    local deadline version length
    deadline=$(cat "$directory/deadline")
    vast_onstart_prefix "$deadline" > "$temporary/onstart.sh"
    cat >> "$temporary/onstart.sh" <<SH
nohup setsid bash -c 'set -euo pipefail; until [[ -f /workspace/job/started ]]; do sleep 2; done; exec bash /workspace/job/remote.sh $session' >> /workspace/job/logs/remote.log 2>&1 < /dev/null &
SH
    length=$(wc -m < "$temporary/onstart.sh")
    ((length <= 4048)) || { spsa_error "起動時スクリプトが4048文字を超えています: $length"; exit 1; }
    version=$(rustc --version | awk '{print $2}')
    [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { spsa_error "公式イメージを指定できないrustcの版です: $version"; exit 1; }
    cp "$temporary/offer.json" "$directory/offer.json"
    cp "$temporary/onstart.sh" "$directory/onstart.sh"
    if ! instance=$(vast_create "$offer" "$label" "$temporary/onstart.sh" "rust:$version-bookworm" 50); then
        # 応答を失っても作成が成立している場合があるため、ラベルでIDを回収する。
        if ids=$(vast_ids "$label") && [[ -n $ids && $ids != *$'\n'* ]]; then
            instance=$ids
            cleanup_instance=yes
            printf 'インスタンス %s: 作成応答を確認できないため削除します\n' "$instance" >&2
        else
            printf 'ラベル %s: 作成操作は未確認です。照会できたID: %s\n' "$label" "$ids" >&2
        fi
        exit 1
    fi
    cleanup_instance=yes
    printf '%s\n' "$instance" > "$directory/instance-id"
    # この関数は起動失敗時に自分で削除する。
    if ! vast_wait_running "$instance" 600; then cleanup_instance=no; exit 1; fi
    local address host port
    address=$(vast_ssh_address "$instance" 600)
    read -r host port <<< "$address"
    printf '%s\n' "$address" > "$directory/ssh-address"
    # 一時bareリポジトリの参照を使い、共同作業中の参照やindexを変更しない。
    git init --bare "$temporary/bundle.git" > /dev/null
    git --git-dir="$temporary/bundle.git" fetch --no-tags "$root" \
        "$COMMIT:refs/heads/session" "$RUNNER_COMMIT:refs/heads/runner"
    git --git-dir="$temporary/bundle.git" bundle create "$temporary/source.bundle" refs/heads/session refs/heads/runner
    # 省略ハッシュやブランチ名は解決済みの完全ハッシュへ固定する。
    local name
    for name in COMMIT RUNNER_COMMIT SEED ITERATIONS PAIRS_PER_ITERATION RULES PRECHECK LOCAL_NPS GAME_RSS_BYTES; do
        printf '%s=%q\n' "$name" "${!name}"
    done > "$temporary/session.env"
    if [[ $PRECHECK == yes ]]; then printf 'PRECHECK_SEED=%q\n' "$PRECHECK_SEED" >> "$temporary/session.env"; fi
    # 本番開始前の終了でも、存在しないパスと転送障害を混同せず取得できる。
    vast_wait_ssh "$instance" "$host" "$port"
    vast_ssh "$host" "$port" "mkdir -p /workspace/job/logs /workspace/job/$session"
    if [[ $PRECHECK == yes ]]; then vast_ssh "$host" "$port" "mkdir -p /workspace/job/$session-safety"; fi
    vast_scp "$host" "$port" /workspace/job/ "$temporary/source.bundle" "$temporary/session.env" "$directory/params" "$SCRIPT_DIR/remote.sh"
    if [[ -f $directory/safety.params ]]; then vast_scp "$host" "$port" /workspace/job/ "$directory/safety.params"; fi
    if [[ -d $directory/run ]]; then
        vast_scp "$host" "$port" "/workspace/job/$session/" "$directory/run/."
    fi
    vast_ssh "$host" "$port" 'touch /workspace/job/started'
    cleanup_instance=no
    printf 'セッション %s を開始しました。インスタンスID: %s\n' "$session" "$instance"
)

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then spsa_launch_main "$@"; fi
