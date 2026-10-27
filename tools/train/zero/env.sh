#!/usr/bin/env bash

ZERO_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
RUST_TOOLCHAIN="1.98.0"
ORT_VERSION="1.28.3"
ORT_SHA256="33e91f819324449bede2d1e853c8d46bb6253fa8890bac98ae676f041c8f73a6"
ORT_DIR="$ZERO_DIR/.runtime/onnxruntime-linux-x64-gpu_cuda13-$ORT_VERSION"
ZERO_SITE_PACKAGES="$ZERO_DIR/.venv/lib/python3.13/site-packages"

export ORT_DYLIB_PATH="$ORT_DIR/lib/libonnxruntime.so"
export LD_LIBRARY_PATH="$ZERO_SITE_PACKAGES/tensorrt_libs:$ZERO_SITE_PACKAGES/nvidia/cu13/lib:$ZERO_SITE_PACKAGES/nvidia/cudnn/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export PATH="$HOME/.cargo/bin:$PATH"
