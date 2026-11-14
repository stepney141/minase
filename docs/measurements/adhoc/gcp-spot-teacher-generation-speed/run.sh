#!/usr/bin/env bash
# 借りた機械でminaseをビルドし、作業機の照合と同じ引数で教師を1,500局生成する。
set -euo pipefail
cd "$HOME"
exec >> "$HOME/setup.log" 2>&1
sudo apt-get update -y -q
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -q build-essential pkg-config git
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
git clone -q "$HOME/minase.bundle" "$HOME/minase"
cd "$HOME/minase"
git checkout -q --detach 95d67282a9c490cf0f8bbf96ce001af1f1f173bc
test "$(git rev-parse HEAD)" = 95d67282a9c490cf0f8bbf96ce001af1f1f173bc
rustc --version
start=$(date +%s)
cargo build --release --locked --bin minase
echo "build_seconds=$(($(date +%s) - start))"
lscpu > "$HOME/lscpu.txt"
sha256sum target/release/minase
date -u +%FT%TZ > "$HOME/gen.start"
nice -n 0 ./target/release/minase data selfplay generate --output-dir "$HOME/gen" --chunk-games 250 \
  --games 1500 --seed 9010000000 --nodes 20000 --random-moves 0 --max-ply 4000 --hash-mb 16 \
  --concurrency 16 > "$HOME/gen.out" 2> "$HOME/gen.err"
date -u +%FT%TZ > "$HOME/DONE"
