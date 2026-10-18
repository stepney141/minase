#!/usr/bin/env bash
# 使い方: bash /workspace/job/remote.sh <セッション名>
# 調整機でビルド、負荷時の速度測定、事前確認、本番を実行し、終了コードを保存する。
# sourceした場合は、設定検査と計算の関数だけを定義する。
set -euo pipefail

spsa_error() { printf '%s\n' "$*" >&2; return 1; }

spsa_name() {
    [[ $1 =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]*$ ]] || {
        spsa_error 'セッション名には英数字、ハイフン、アンダースコアだけを使ってください'; return 1;
    }
    case "$1" in
        minase|logs|started) spsa_error "ジョブの予約名は使えません: $1"; return 1 ;;
    esac
}

# 引数: session.envとparamsを置いたディレクトリ。
# 呼出し元の環境変数で欠けた設定を補わない。
spsa_load_session() {
    local directory=$1 name
    [[ -f $directory/session.env ]] || { spsa_error "session.envがありません: $directory"; return 1; }
    unset COMMIT RUNNER_COMMIT SEED ITERATIONS PAIRS_PER_ITERATION RULES PRECHECK PRECHECK_SEED LOCAL_NPS GAME_RSS_BYTES
    # session.envは利用者が作るBashの設定ファイルである。
    # shellcheck disable=SC1091
    source "$directory/session.env"
    for name in COMMIT RUNNER_COMMIT SEED ITERATIONS PAIRS_PER_ITERATION RULES PRECHECK LOCAL_NPS GAME_RSS_BYTES; do
        [[ -v $name && -n ${!name} ]] || { spsa_error "session.envに$nameがありません"; return 1; }
    done
    [[ $PRECHECK == yes || $PRECHECK == no ]] || { spsa_error 'PRECHECKはyesかnoにしてください'; return 1; }
    [[ -s $directory/params ]] || { spsa_error "paramsがないか空です: $directory"; return 1; }
    if [[ $PRECHECK == yes ]]; then
        [[ -v PRECHECK_SEED && -n $PRECHECK_SEED ]] || { spsa_error 'session.envにPRECHECK_SEEDがありません'; return 1; }
        [[ -s $directory/safety.params ]] || { spsa_error 'PRECHECK=yesですがsafety.paramsがないか空です'; return 1; }
    fi
    local -a values=("SEED=$SEED" "ITERATIONS=$ITERATIONS" "PAIRS_PER_ITERATION=$PAIRS_PER_ITERATION" "GAME_RSS_BYTES=$GAME_RSS_BYTES")
    if [[ $PRECHECK == yes ]]; then values+=("PRECHECK_SEED=$PRECHECK_SEED"); fi
    python3 - "$LOCAL_NPS" "${values[@]}" <<'PY'
import math, re, sys
nps = float(sys.argv[1])
if not math.isfinite(nps) or nps <= 0:
    sys.exit("LOCAL_NPSは有限の正数にしてください")
for field in sys.argv[2:]:
    name, value = field.split("=", 1)
    minimum = 0 if name in ("SEED", "PRECHECK_SEED") else 1
    if not re.fullmatch(r"0|[1-9][0-9]*", value) or not minimum <= int(value) < 2**64:
        sys.exit(f"{name}の整数値が不正です: {value}")
PY
}

# 引数: 本番manifest。別のセッションの記録や設定の取り違えを転送前に拒む。
spsa_check_manifest() {
    python3 - "$1" "$COMMIT" "$SEED" "$ITERATIONS" "$PAIRS_PER_ITERATION" "$RULES" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
if m["engine"]["kind"] != "commit" or m["engine"]["hash"] != sys.argv[2]:
    sys.exit("再開用の記録とCOMMITが一致しません")
for field, argument in (("seed", 3), ("iterations", 4), ("pairs_per_iteration", 5)):
    if m["settings"][field] != int(sys.argv[argument]):
        sys.exit(f"再開用の記録とsession.envの{field}が一致しません")
if m["rules_source"] != sys.argv[6]:
    sys.exit("再開用の記録とRULESが一致しません")
PY
}

# 引数: 物理コア数、実メモリのバイト数、1局の最大常駐メモリのバイト数。
spsa_concurrency() {
    python3 - "$@" <<'PY'
import sys
cores, memory, rss = map(int, sys.argv[1:])
if min(cores, memory, rss) <= 0:
    sys.exit("コア数とメモリは正数でなければなりません")
c = min(cores - 1, memory * 4 // (rss * 5))
if c < 1:
    sys.exit("1局を実行するCPUまたはメモリが足りません")
print(c)
PY
}

# 引数: 測定機NPS、調整機NPS、反復数、ペア数、同時対局数。
spsa_sizing() {
    python3 - "$@" <<'PY'
from decimal import Decimal, ROUND_HALF_UP
import sys
local, remote, iterations, pairs, c = map(Decimal, sys.argv[1:])
if any(not v.is_finite() or v <= 0 for v in (local, remote, iterations, pairs, c)):
    sys.exit("速度と実行規模は有限の正数にしてください")
f = (local / remote).quantize(Decimal("0.01"), rounding=ROUND_HALF_UP)
if f <= 0:
    sys.exit("丸めた換算係数が0です")
base, increment = ((v * f).quantize(Decimal(1), rounding=ROUND_HALF_UP) for v in (10000, 100))
hours = iterations * pairs * Decimal("9.05") * 16 * f / (c * 3600)
print(f"{f} time={base}+{increment} {hours:.8f}")
PY
}

# lscpuは英語表示に固定し、cgroupの無制限値maxも明示的に扱う。
spsa_resources() {
    LC_ALL=C lscpu --json | python3 -c '
import json, pathlib, sys
rows = {r["field"]: r["data"] for r in json.load(sys.stdin)["lscpu"]}
cores = int(rows["Socket(s):"]) * int(rows["Core(s) per socket:"])
memory = next(int(line.split()[1]) * 1024 for line in pathlib.Path("/proc/meminfo").read_text().splitlines() if line.startswith("MemTotal:"))
v1 = pathlib.Path("/sys/fs/cgroup/memory/memory.limit_in_bytes")
v2 = pathlib.Path("/sys/fs/cgroup/memory.max")
if v1.is_file():
    memory = min(memory, int(v1.read_text()))
elif v2.is_file():
    limit = v2.read_text().strip()
    if limit != "max":
        memory = min(memory, int(limit))
else:
    sys.exit("cgroupのメモリ上限を取得できません")
print(cores, memory)
'
}

# 引数: 調整用ビルド、同時対局数、ログディレクトリ。
spsa_bench() {
    local binary=$1 c=$2 logs=$3 index pid failed=0
    local -a pids=()
    for ((index=1; index<=c; index++)); do
        "$binary" dev bench --depth 9 --repetitions 3 > "$logs/bench-$index.log" 2>&1 &
        pids+=("$!")
    done
    for pid in "${pids[@]}"; do wait "$pid" || failed=1; done
    ((failed == 0)) || { spsa_error 'benchが失敗しました'; return 1; }
    python3 - "$logs" "$c" <<'PY'
from decimal import Decimal
from pathlib import Path
import re, sys
values = []
for i in range(1, int(sys.argv[2]) + 1):
    text = (Path(sys.argv[1]) / f"bench-{i}.log").read_text()
    matches = re.findall(r"^median:.*\bnps=([0-9]+)\s*$", text, re.M)
    if len(matches) != 1 or int(matches[0]) <= 0:
        sys.exit(f"bench-{i}.logの中央値NPSが不正です")
    values.append(Decimal(matches[0]))
print(sum(values) / len(values))
PY
}

# 引数: 最終要約のログ、必要な反復数、異常0件を要求するか（yes/no）。
spsa_summary_ok() {
    python3 - "$@" <<'PY'
from pathlib import Path
import re, sys
text = Path(sys.argv[1]).read_text()
summaries = re.findall(r"^summary: applied=([0-9]+) valid_pairs=[0-9]+ discarded_pairs=[0-9]+$", text, re.M)
failures = re.findall(r"^engine_failures: illegal_moves=([0-9]+) crashes=([0-9]+) timeouts=([0-9]+) time_forfeits=([0-9]+) rejected_moves=([0-9]+)$", text, re.M)
if not summaries or not failures or int(summaries[-1]) != int(sys.argv[2]):
    sys.exit("最終要約がないか、完了反復数が一致しません")
if sys.argv[3] == "yes" and any(int(n) for n in failures[-1]):
    sys.exit("時間切れの事前確認でエンジン異常が発生しました")
PY
}

# 引数: 実行ディレクトリ。新規作成用の空ディレクトリだけを除く。
spsa_prepare_run() {
    if [[ -d $1 && ! -f $1/manifest.json ]]; then
        rmdir "$1" || { spsa_error "manifest.jsonのない実行ディレクトリが空ではありません: $1"; return 1; }
    fi
}

spsa_remote_exit() {
    local rc=$?
    trap - EXIT
    printf '%s\n' "$rc" > /workspace/job/logs/.exit-status.tmp
    mv /workspace/job/logs/.exit-status.tmp /workspace/job/logs/exit-status
    exit "$rc"
}

spsa_remote_main() {
    [[ $# == 1 ]] || { spsa_error '使い方: remote.sh <セッション名>'; return 1; }
    spsa_name "$1"
    local session=$1 job=/workspace/job
    cd "$job"
    mkdir -p logs
    # 重複起動は進行中の終了コードを上書きしない。
    exec 9> "$job/.remote.lock"
    flock -n 9 || { spsa_error 'remote.shはすでに実行中です'; return 1; }
    rm -f logs/exit-status
    trap spsa_remote_exit EXIT
    trap 'exit 130' INT
    trap 'exit 143' TERM
    # 公式RustイメージにPythonが含まれることを前提にしない。
    if [[ ! -f .dependencies-ready ]]; then
        apt-get update > logs/packages.log 2>&1
        apt-get install -y --no-install-recommends python3 util-linux >> logs/packages.log 2>&1
        touch .dependencies-ready
    fi
    spsa_load_session "$job"
    if [[ ! -d minase ]]; then
        if [[ -d minase-cloning ]]; then rm -rf minase-cloning; fi
        git clone --branch runner "$job/source.bundle" minase-cloning > logs/clone.log 2>&1
        mv minase-cloning minase
    fi
    cd minase
    git checkout --detach "$RUNNER_COMMIT" >> "$job/logs/clone.log" 2>&1
    COMMIT=$(git rev-parse --verify --end-of-options "$COMMIT^{commit}")
    if [[ ! -f $job/.build-ready ]]; then
        cargo build --release --locked --bin minase > "$job/logs/build.log" 2>&1
        target/release/minase spsa params --engine "commit:$COMMIT" --rules "$RULES" \
            > "$job/logs/declared.params" 2>> "$job/logs/build.log"
        touch "$job/.build-ready"
    fi
    local binary="$job/minase/target/match-cache/$COMMIT-tuning/minase"
    [[ -x $binary ]] || { spsa_error "調整用ビルドがありません: $binary"; return 1; }
    local resources cores memory c remote_nps sizing f each hours
    resources=$(spsa_resources)
    read -r cores memory <<< "$resources"
    c=$(spsa_concurrency "$cores" "$memory" "$GAME_RSS_BYTES")
    remote_nps=$(spsa_bench "$binary" "$c" "$job/logs")
    sizing=$(spsa_sizing "$LOCAL_NPS" "$remote_nps" "$ITERATIONS" "$PAIRS_PER_ITERATION" "$c")
    read -r f each hours <<< "$sizing"
    {
        printf 'measured_at=%s\nphysical_cores=%s\nmemory_bytes=%s\nc=%s\n' "$(date -u +%FT%TZ)" "$cores" "$memory" "$c"
        printf 'local_nps=%s\nloaded_nps=%s\nf=%s\neach=%s\nestimated_hours=%s\n' "$LOCAL_NPS" "$remote_nps" "$f" "$each" "$hours"
    } >> "$job/logs/sizing.txt"
    local run="$job/$session" safety="$job/$session-safety" safety_iterations
    if [[ -f $run/manifest.json ]]; then
        spsa_check_manifest "$run/manifest.json"
        printf 'resume=%s; measured f and each are not applied\n' "$run" >> "$job/logs/sizing.txt"
        target/release/minase spsa --resume "$run" > "$job/logs/$session.log" 2>&1 || return 5
        spsa_summary_ok "$job/logs/$session.log" "$ITERATIONS" no || return 5
        return 0
    fi
    if ! python3 -c 'import sys; sys.exit(0 if float(sys.argv[1]) <= 24 else 1)' "$hours"; then return 3; fi
    if [[ $PRECHECK == yes ]]; then
        safety_iterations=$(((13 * c + 15) / 16))
        if [[ -f $safety/manifest.json ]]; then
            # 途中の事前確認もmanifestの条件で再開し、本番へ同じ条件を渡す。
            sizing=$(python3 -c 'import json,sys; m=json.load(open(sys.argv[1])); print(m["settings"]["iterations"], m["settings"]["concurrency"], m["each"])' "$safety/manifest.json")
            read -r safety_iterations c each <<< "$sizing"
            if [[ ! -f $job/logs/$session-safety.log ]] || ! spsa_summary_ok "$job/logs/$session-safety.log" "$safety_iterations" yes; then
                target/release/minase spsa --resume "$safety" > "$job/logs/$session-safety.log" 2>&1 || return 4
            fi
        else
            spsa_prepare_run "$safety"
            target/release/minase spsa --run-dir "$safety" --seed "$PRECHECK_SEED" \
                --engine "commit:$COMMIT" --params "$job/safety.params" --rules "$RULES" \
                --each "$each" --concurrency "$c" --iterations "$safety_iterations" --pairs-per-iteration 8 \
                > "$job/logs/$session-safety.log" 2>&1 || return 4
        fi
        spsa_summary_ok "$job/logs/$session-safety.log" "$safety_iterations" yes || return 4
    fi
    printf 'production_c=%s\nproduction_each=%s\n' "$c" "$each" >> "$job/logs/sizing.txt"
    spsa_prepare_run "$run"
    target/release/minase spsa --run-dir "$run" --seed "$SEED" \
        --engine "commit:$COMMIT" --params "$job/params" --rules "$RULES" \
        --each "$each" --concurrency "$c" --iterations "$ITERATIONS" --pairs-per-iteration "$PAIRS_PER_ITERATION" \
        > "$job/logs/$session.log" 2>&1 || return 5
    spsa_summary_ok "$job/logs/$session.log" "$ITERATIONS" no || return 5
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then spsa_remote_main "$@"; fi
