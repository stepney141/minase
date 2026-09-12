#!/bin/sh
# 指定コミットの minase を含む lishogi Bot イメージをビルドする。
# 使い方: tools/lishogi-bot/build-image.sh <コミット>
# 作業ツリーではなく `git archive` の内容だけをビルド文脈にするため、
# 未コミットの変更はイメージへ入らない。イメージは
# minase-lishogi-bot:<完全ハッシュ> と minase-lishogi-bot:latest に付ける。
set -eu
if [ "$#" -ne 1 ]; then
    echo "usage: $0 <commit>" >&2
    exit 2
fi
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
commit=$(git -C "$repo" rev-parse --verify "$1^{commit}")
git -C "$repo" archive --format=tar "$commit" \
    | docker build \
        --file tools/lishogi-bot/Dockerfile \
        --build-arg "MINASE_COMMIT=$commit" \
        --tag "minase-lishogi-bot:$commit" \
        --tag minase-lishogi-bot:latest \
        -
echo "built minase-lishogi-bot:$commit"
