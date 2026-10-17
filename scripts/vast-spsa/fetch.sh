#!/usr/bin/env bash
# 使い方: bash scripts/vast-spsa/fetch.sh <セッション名>
# 10分ごとに実行記録を取得し、再開用の写しを更新する。終了時に検査して削除または停止する。
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/vast/common.sh
source "$SCRIPT_DIR/../vast/common.sh"
# shellcheck source=scripts/vast-spsa/remote.sh
source "$SCRIPT_DIR/remote.sh"

# 引数: ID、セッション名、測定機のセッションディレクトリ。
# 転送先を毎回新しくし、run/と共有するinodeへの上書きを防ぐ。
spsa_fetch_once() (
    local id=$1 session=$2 directory=$3 temporary rc
    temporary=$(mktemp -d "$directory/.fetch.XXXXXX") || return 1
    trap 'rm -rf "$temporary"' EXIT
    mkdir -p "$directory/logs" "$temporary/incoming" "$temporary/logs" || return 1
    rm -f "$directory/logs/exit-status" || return 1
    vast_copy "$id" "/workspace/job/$session" "$temporary/incoming/" || return 1
    if [[ $PRECHECK == yes ]]; then
        mkdir "$temporary/safety" || return 1
        vast_copy "$id" "/workspace/job/$session-safety" "$temporary/safety/" || return 1
    fi
    vast_copy "$id" /workspace/job/logs/ "$temporary/logs/" || return 1
    # 前のincomingを除いても、runのハードリンクは残る。
    if [[ -d $directory/incoming ]]; then mv "$directory/incoming" "$temporary/old-incoming" || return 1; fi
    mv "$temporary/incoming" "$directory/incoming" || return 1
    if [[ $PRECHECK == yes ]]; then
        if [[ -d $directory/safety ]]; then mv "$directory/safety" "$temporary/old-safety" || return 1; fi
        mv "$temporary/safety" "$directory/safety" || return 1
    fi
    # 過去の起動のログを残しつつ、現在取得したログを反映する。
    cp -a "$temporary/logs/." "$directory/logs/" || return 1
    rc=0
    python3 "$SCRIPT_DIR/sync_run.py" "$directory/incoming/$session" "$directory/run" || rc=$?
    # 部分取得は前回のrunを保ち、次の取得で改めて検査する。
    [[ $rc == 0 || $rc == 2 ]] || return "$rc"
)

# 引数: 連続失敗回数、失敗した処理。通信障害では調整機を操作しない。
spsa_fetch_retry() {
    printf '%s。連続失敗 %s/6回。取得済みの記録は残しています\n' "$2" "$1" >&2
    if (($1 >= 6)); then
        spsa_error '6回連続で失敗したため取得を終了します。調整機の状態を確認してください'
        return 1
    fi
    sleep 600
}

spsa_fetch_main() (
    [[ $# == 1 ]] || { spsa_error '使い方: fetch.sh <セッション名>'; exit 1; }
    spsa_name "$1"
    local session=$1 root directory ids id state deadline ended exit_status source_file failures=0
    root=$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel)
    directory="$root/data/vast-spsa/$session"
    spsa_load_session "$directory"
    COMMIT=$(git -C "$root" rev-parse --verify --end-of-options "$COMMIT^{commit}")
    [[ -f $directory/deadline ]] || { spsa_error 'deadlineがありません'; exit 1; }
    deadline=$(cat "$directory/deadline")
    [[ $deadline =~ ^[1-9][0-9]*$ ]] || { spsa_error 'deadlineが不正です'; exit 1; }
    exec 8> "$directory/.operation.lock"
    flock -n 8 || { spsa_error '同じセッションの起動または取得が実行中です'; exit 1; }
    cd "$root"
    while :; do
        if ! ids=$(vast_ids "minase-spsa-$session"); then
            failures=$((failures + 1))
            spsa_fetch_retry "$failures" "セッション $session: 調整機の照会に失敗しました"
            continue
        fi
        [[ -n $ids ]] || { spsa_error '調整機がありません。取得済みの記録は残しています'; exit 1; }
        [[ $ids != *$'\n'* ]] || { spsa_error "調整機が複数あります。操作しません: $ids"; exit 1; }
        id=$ids
        if ! spsa_fetch_once "$id" "$session" "$directory"; then
            failures=$((failures + 1))
            spsa_fetch_retry "$failures" "インスタンス $id: 取得に失敗しました"
            continue
        fi
        if ! state=$(vast_instance "$id" 600) ||
            ! state=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["actual_status"])' "$state"); then
            failures=$((failures + 1))
            spsa_fetch_retry "$failures" "インスタンス $id: 状態の照会に失敗しました"
            continue
        fi
        failures=0
        ended=no
        case "$state" in
            stopped|exited|offline|unknown) ended=yes ;;
            frozen) vast_stop "$id"; ended=yes ;;
            running)
                if (( $(date +%s) >= deadline )); then vast_stop "$id"; ended=yes; fi ;;
            loading|rebooting) ;;
            *) spsa_error "インスタンス $id: 未対応の状態です: $state"; exit 1 ;;
        esac
        if [[ -f $directory/logs/exit-status ]]; then ended=yes; fi
        if [[ $ended == no ]]; then sleep 600; continue; fi
        # 終了通知より前に取得した反復もあるため、最後にもう一度取得する。
        if ! spsa_fetch_once "$id" "$session" "$directory"; then
            spsa_error "インスタンス $id: 最終取得に失敗しました。調整機は操作せず、取得済みの記録を残します"
            exit 1
        fi
        exit_status=missing
        if [[ -f $directory/logs/exit-status ]]; then exit_status=$(cat "$directory/logs/exit-status"); fi
        if [[ $exit_status == 0 ]] && python3 "$SCRIPT_DIR/sync_run.py" --check "$directory/run" "$ITERATIONS"; then
            if ! spsa_check_manifest "$directory/run/manifest.json"; then vast_stop "$id"; exit 1; fi
            source_file=$(mktemp "$directory/.params.XXXXXX.rs")
            trap 'rm -f "$source_file"' EXIT
            if ! git show "$COMMIT:crates/minase/src/search/alphabeta/params.rs" > "$source_file"; then
                vast_stop "$id"; exit 1
            fi
            if [[ ! -x target/release/minase ]]; then
                if ! cargo build --release --bin minase; then vast_stop "$id"; exit 1; fi
            fi
            if ! target/release/minase spsa apply --run-dir "$directory/run" --source "$source_file"; then
                vast_stop "$id"
                spsa_error "インスタンス $id: applyによる検査に失敗しました"
                exit 1
            fi
            vast_destroy "$id"
            printf 'セッション %s の全反復を取得し、検査と調整機の削除を完了しました\n' "$session"
            exit 0
        elif [[ $exit_status == 3 ]]; then
            if [[ -f $directory/logs/sizing.txt ]]; then cat "$directory/logs/sizing.txt"; fi
            vast_destroy "$id"
            spsa_error '所要時間の見込みが24時間を超えました'
            exit 3
        else
            vast_stop "$id"
            spsa_error "セッション $session は未完了です。終了コード: $exit_status。インスタンス $id は停止しました"
            exit 1
        fi
    done
)

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then spsa_fetch_main "$@"; fi
