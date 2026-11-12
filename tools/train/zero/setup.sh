#!/usr/bin/env bash
set -euo pipefail

source "$(dirname -- "${BASH_SOURCE[0]}")/env.sh"

if [[ ! -x "$ZERO_DIR/.runtime/bin/uv" ]] ||
    [[ "$("$ZERO_DIR/.runtime/bin/uv" --version | awk '{print $2}')" != "$UV_VERSION" ]]; then
    curl -LsSf "https://astral.sh/uv/$UV_VERSION/install.sh" | env UV_UNMANAGED_INSTALL="$ZERO_DIR/.runtime/bin" sh
fi

if [[ ! -x "$CARGO_HOME/bin/rustup" ]]; then
    curl -LsSf https://sh.rustup.rs | sh -s -- --default-toolchain none --profile minimal --no-modify-path -y
fi
rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal --component clippy --component rustfmt

cd -- "$ZERO_DIR"
# Leave mnsd_reader uninstalled here; sync-env.sh builds it from source on the machine that runs it.
uv sync --frozen --no-install-local

if [[ ! -d "$ORT_DIR" ]]; then
    mkdir -p .runtime
    ORT_ARCHIVE="onnxruntime-linux-x64-gpu_cuda13-$ORT_VERSION.tgz"
    curl -LfsS "https://github.com/microsoft/onnxruntime/releases/download/v$ORT_VERSION/$ORT_ARCHIVE" -o ".runtime/$ORT_ARCHIVE"
    printf '%s  %s\n' "$ORT_SHA256" ".runtime/$ORT_ARCHIVE" | sha256sum -c -
    tar -xzf ".runtime/$ORT_ARCHIVE" -C .runtime
    rm -- ".runtime/$ORT_ARCHIVE"
fi

uv run --frozen --no-sync python -c "import torch; print(torch.__version__, torch.version.cuda, torch.cuda.is_available())"
rustc --version
cargo clippy --version
cargo fmt --version
ls "$ORT_DYLIB_PATH"
