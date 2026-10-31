#!/usr/bin/env bash
# Colab機でシェルのコマンドを実行する。pull.shとpush.shと同じControlMasterの接続を共有する。
# 使い方: rsh.sh <セッション名> <コマンド>
set -euo pipefail
session=$1
shift
exec ssh -o "ProxyCommand=colab ssh --proxy-mode -s $session" -o ControlMaster=auto \
    -o "ControlPath=/tmp/colab-cm-$session" -o ControlPersist=900 -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR root@colab "$@"
