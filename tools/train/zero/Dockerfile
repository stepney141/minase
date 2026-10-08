# syntax=docker/dockerfile:1
# AlphaZero型探索の学習、自己対局、および推論に使う土台のイメージ（docs/plans/alphazero.md
# 「実行環境はコンテナイメージにまとめる」）。ライブラリとツールチェーンだけを含み、
# minaseのソースもバイナリも含めない。ビルドの文脈はこのディレクトリであり、
# 取り込むのは pyproject.toml と uv.lock だけである（.dockerignore）。
#
# 基底はCUDA 13.4とcuDNN 9.26を含むNVIDIAの公式イメージとする。
# ONNX Runtime 1.28はCUDA 13.2以上とcuDNN 9.23以上を要件とするが、PyTorch 2.13は
# pipのcuDNNを9.20.0.48に固定しているので、ONNX RuntimeのcuDNNはpipから得られない。
# このため、ONNX RuntimeのCUDAとcuDNNはこの基底イメージのものを使い、PyTorchは自分のwheelが
# 同梱する版を使う。CUDAのコンパイラは使わないので、devel版ではなくruntime版を選ぶ。
# NVIDIAの-tensorrt-のイメージはTensorRT 11系であり、ONNX Runtime 1.28のTensorRT実行プロバイダが
# FP16の設定を無効にする版なので使わない。
FROM nvidia/cuda:13.4.2-cudnn-runtime-ubuntu24.04

ARG RUST_VERSION=1.98.0

# cc はRustのリンカ、git はハーネスの commit: 指定（git archive）に使う。
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl build-essential git \
    && rm -rf /var/lib/apt/lists/*

COPY --from=ghcr.io/astral-sh/uv:0.12.17 /uv /uvx /usr/local/bin/

# 仮想環境、uvのキャッシュ、およびCargoの置き場は、ホストの利用者のuidで起動したコンテナからも
# 書けるように/opt/の下へ置いて全員に書込みを許す。
ENV UV_PROJECT_ENVIRONMENT=/opt/zero-venv \
    UV_PYTHON_INSTALL_DIR=/opt/uv-python \
    UV_CACHE_DIR=/opt/uv-cache \
    UV_LINK_MODE=copy \
    RUSTUP_HOME=/opt/rustup \
    CARGO_HOME=/opt/cargo \
    PATH=/opt/cargo/bin:$PATH

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain ${RUST_VERSION} --component clippy,rustfmt

# TensorRTはNVIDIAのaptリポジトリではなくpipのtensorrt-cu13で入れる。
# ONNX Runtime 1.28の公式ビルドと同じ10.14.1.48をuv.lockで固定でき、学習器と推論が
# 同じ版を使うことをロックファイル1つで管理できるからである。
COPY pyproject.toml uv.lock /opt/zero-project/
# uvのキャッシュは仮想環境の複製（約12 GB）になるので、同じ層の中で空にしてイメージに残さない。
RUN uv python install 3.12 \
    && uv sync --project /opt/zero-project --frozen --no-install-project \
    && uv cache clean \
    && mkdir -p /opt/uv-cache \
    && chmod -R a+rwX /opt/zero-venv /opt/uv-cache /opt/cargo

# RustのminaseがONNX Runtimeを実行時に読み込むための設定。pipのonnxruntime-gpuが同梱する
# 共有ライブラリを指し、TensorRTの共有ライブラリを動的リンカの検索パスへ登録する。
RUN /opt/zero-venv/bin/python -c 'import os, tensorrt_libs; print(os.path.dirname(tensorrt_libs.__file__))' \
        > /etc/ld.so.conf.d/tensorrt.conf \
    && ldconfig
ENV ORT_DYLIB_PATH=/opt/zero-venv/lib/python3.12/site-packages/onnxruntime/capi/libonnxruntime.so.1.28.0

WORKDIR /work
