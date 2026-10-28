#!/usr/bin/env bash
# NASの<相対パス>のファイルを、Colab機の /content/az の同じ相対パスへ送る。
# 送った後にColab機で求めたSHA-256がNASのファイルと一致しなければ失敗する。
# 使い方: push.sh <セッション名> <相対パス>...
set -euo pipefail
session=$1
shift
root=/mnt/synology_nas/minase/alphazero
# Colab機は同時に1本のcolab sshしか受け付けないので、ControlMasterで1本の接続を共有する。
rsh="ssh -o ProxyCommand='colab ssh --proxy-mode -s $session' -o ControlMaster=auto -o ControlPath=/tmp/colab-cm-$session -o ControlPersist=900 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR"
for rel in "$@"; do
    local=$(sha256sum -- "$root/$rel" | awk '{print $1}')
    eval "$rsh root@colab" "\"mkdir -p '/content/az/$(dirname -- "$rel")'\""
    rsync -a -e "$rsh" "$root/$rel" "root@colab:/content/az/$rel"
    remote=$(eval "$rsh root@colab" "\"cd /content/az && sha256sum -- '$rel'\"" | awk '{print $1}')
    if [[ $remote != "$local" ]]; then
        echo "checksum mismatch: $rel remote=$remote local=$local" >&2
        exit 1
    fi
    echo "$local  $rel"
done
