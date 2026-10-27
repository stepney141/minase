#!/usr/bin/env bash
set -euo pipefail

source "$(dirname -- "${BASH_SOURCE[0]}")/env.sh"

if ! command -v uv >/dev/null 2>&1; then
    curl -LsSf https://astral.sh/uv/0.13.4/install.sh | env UV_INSTALL_DIR="$HOME/.local/bin" sh
    export PATH="$HOME/.local/bin:$PATH"
fi

if ! command -v rustup >/dev/null 2>&1; then
    curl -LsSf https://sh.rustup.rs | sh -s -- --default-toolchain none --profile minimal -y
fi
rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal

cd -- "$ZERO_DIR"
# uv builds the MNSD reader extension (mnsd_reader/) with cargo; pin it to the same toolchain.
RUSTUP_TOOLCHAIN="$RUST_TOOLCHAIN" uv sync --frozen

if [[ ! -d "$ORT_DIR" ]]; then
    mkdir -p .runtime
    ORT_ARCHIVE="onnxruntime-linux-x64-gpu_cuda13-$ORT_VERSION.tgz"
    curl -LfsS "https://github.com/microsoft/onnxruntime/releases/download/v$ORT_VERSION/$ORT_ARCHIVE" -o ".runtime/$ORT_ARCHIVE"
    printf '%s  %s\n' "$ORT_SHA256" ".runtime/$ORT_ARCHIVE" | sha256sum -c -
    tar -xzf ".runtime/$ORT_ARCHIVE" -C .runtime
fi

uv run --frozen python -c "import torch; print(torch.__version__, torch.version.cuda, torch.cuda.is_available())"
rustc +"$RUST_TOOLCHAIN" --version
ls "$ORT_DYLIB_PATH"
