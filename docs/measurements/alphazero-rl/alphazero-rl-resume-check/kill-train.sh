#!/usr/bin/env bash
# 学習の再開の確認：学習を起動し、<止める更新回数>のチェックポイントを確定させたことを示す行が
# ログに現れた直後に、学習器のプロセス（uvとPython）を SIGKILL で止める。
# 借りた機械が予告なく止まる場合に合わせ、終了処理の機会を与えないシグナルを使う。
# 使い方: kill-train.sh <ログ> <止める更新回数> <train.sh に渡す引数...>
set -euo pipefail
log=$1
stop=$2
shift 2
setsid bash /content/train.sh "$@" > "$log" 2>&1 < /dev/null &
until grep -q "^steps=$stop/" "$log"; do
    if ! pgrep -f "[m]inase_zero train" > /dev/null && ! grep -q "^steps=$stop/" "$log"; then
        echo "trainer exited before step $stop" >&2
        exit 1
    fi
    sleep 0.1
done
pkill -KILL -f "[m]inase_zero train"
sleep 1
if pgrep -af "[m]inase_zero train"; then
    echo "trainer still running" >&2
    exit 1
fi
echo "killed after steps=$stop"
