#!/usr/bin/env bash

ZERO_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
UV_VERSION="0.13.4"
RUST_TOOLCHAIN="1.98.0"
ORT_VERSION="1.31.1"
ORT_SHA256="b2146862e691f7e338f5aff9c7a9a5567c2ca57cf01442a1adb3a13bf62d9095"
ORT_DIR="$ZERO_DIR/.runtime/onnxruntime-linux-x64-gpu_cuda13-$ORT_VERSION"
ZERO_SITE_PACKAGES="$ZERO_DIR/.venv/lib/python3.13/site-packages"

export RUSTUP_HOME="$ZERO_DIR/.runtime/rustup"
export CARGO_HOME="$ZERO_DIR/.runtime/cargo"
export RUSTUP_TOOLCHAIN="$RUST_TOOLCHAIN"
export UV_PYTHON_INSTALL_DIR="$ZERO_DIR/.runtime/python"
export UV_MANAGED_PYTHON=1
export UV_PROJECT_ENVIRONMENT="$ZERO_DIR/.venv"
export ORT_DYLIB_PATH="$ORT_DIR/lib/libonnxruntime.so"
export LD_LIBRARY_PATH="$ZERO_SITE_PACKAGES/tensorrt_libs:$ZERO_SITE_PACKAGES/nvidia/cu13/lib:$ZERO_SITE_PACKAGES/nvidia/cudnn/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$ZERO_DIR/.runtime/bin:$CARGO_HOME/bin:$PATH"
