#!/usr/bin/env bash
# Colab機の /content/az/<相対パス> のファイルを、NASの同じ相対パスへ受け取る。
# Colab機で求めたSHA-256と受け取った一時ファイルのSHA-256が一致したときだけ名前を変えて確定させる。
# 使い方: pull.sh <セッション名> <相対パス>...
set -euo pipefail
session=$1
shift
root=/mnt/synology_nas/minase/alphazero
# Colab機は同時に1本のcolab sshしか受け付けないので、ControlMasterで1本の接続を共有する。
rsh="ssh -o ProxyCommand='colab ssh --proxy-mode -s $session' -o ControlMaster=auto -o ControlPath=/tmp/colab-cm-$session -o ControlPersist=900 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
for rel in "$@"; do
    remote=$(eval "$rsh root@colab" "\"cd /content/az && sha256sum -- '$rel'\"" | awk '{print $1}')
    dir=$root/$(dirname -- "$rel")
    base=$(basename -- "$rel")
    mkdir -p -- "$dir"
    rsync -a -e "$rsh" "root@colab:/content/az/$rel" "$dir/.$base.tmp"
    local=$(sha256sum -- "$dir/.$base.tmp" | awk '{print $1}')
    if [[ $remote != "$local" ]]; then
        echo "checksum mismatch: $rel remote=$remote local=$local" >&2
        exit 1
    fi
    mv -- "$dir/.$base.tmp" "$dir/$base"
    echo "$local  $rel"
done
