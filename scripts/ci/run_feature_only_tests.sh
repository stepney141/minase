#!/usr/bin/env bash
# minaseのライブラリテストのうち、フィーチャsearch-statsとtuningを有効にしたときだけ
# コンパイルされるテストを実行する。両方のビルドにあるテストは`cargo test --workspace`が
# 既定のビルドで実行するので、ここでは繰り返さない。
set -euo pipefail

features=search-stats,tuning

list_tests() {
    cargo test -p minase --lib "$@" -- --list --format terse | sed -n 's/: test$//p' | sort
}

default_tests=$(mktemp)
feature_tests=$(mktemp)
trap 'rm -f "$default_tests" "$feature_tests"' EXIT

list_tests >"$default_tests"
list_tests --features "$features" >"$feature_tests"
mapfile -t feature_only < <(comm -13 "$default_tests" "$feature_tests")

# 絞り込みを1つも渡さないと`cargo test`は全テストを実行するので、空なら失敗させる。
if [[ ${#feature_only[@]} -eq 0 ]]; then
    echo "error: no minase library test is compiled only with --features $features" >&2
    exit 1
fi

cargo test -p minase --lib --features "$features" -- --exact "${feature_only[@]}"
