"""ONNX Runtimeでネットの毎秒の評価数とGPUメモリの使用量を測る。

docs/plans/alphazero.md「フェーズ0」の第1の測定に使う。
実行プロバイダごとに1つのセッションを作り、バッチの大きさごとにウォームアップの後で一定回数の推論の
所要時間を測って、1行1条件のTSVを標準出力へ書く。

使い方:
    python -m minase_zero.bench --model net.onnx --provider tensorrt --trt-cache-dir cache/
    python -m minase_zero.bench --model net.onnx --provider tensorrt --no-fp16
    python -m minase_zero.bench --model net.onnx --provider cpu --batches 64
"""

from __future__ import annotations

import argparse
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import onnxruntime as ort

from minase_zero.encoding import BOARD_SIZE, INPUT_PLANES
from minase_zero.export import INPUT_NAME

PROVIDERS = {
    "tensorrt": "TensorrtExecutionProvider",
    "cuda": "CUDAExecutionProvider",
    "cpu": "CPUExecutionProvider",
}
"""コマンドラインの名前とONNX Runtimeの実行プロバイダ名の対応。"""

HEADER = (
    "provider",
    "fp16",
    "batch",
    "warmup",
    "iterations",
    "seconds",
    "positions_per_second",
    "gpu_memory_used_mib",
    "gpu_memory_baseline_mib",
)
"""TSVの見出し。"""

PIECE_DENSITY = 92 / (INPUT_PLANES * BOARD_SIZE * BOARD_SIZE)
"""計測用の入力で1にする要素の割合。初期局面の駒数92枚を全要素数で割った値である。"""


def gpu_memory_used_mib() -> int:
    """`nvidia-smi`が報告するGPU 0のメモリの使用量（MiB）を返す。

    GPU全体の使用量であり、他のプロセスの分も含む。

    Raises:
        RuntimeError: `nvidia-smi`を実行できない場合。
    """
    try:
        result = subprocess.run(
            ["nvidia-smi", "--query-gpu=memory.used", "--format=csv,noheader,nounits", "-i", "0"],
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        raise RuntimeError(f"failed to query GPU memory with nvidia-smi: {error}") from error
    return int(result.stdout.strip())


def provider_options(
    provider: str, fp16: bool, max_batch: int, cache_dir: Path | None, verbose: bool
) -> dict[str, object]:
    """実行プロバイダの設定を返す。"""
    if provider != "tensorrt":
        return {}
    shape = f"{INPUT_NAME}:{{}}x{INPUT_PLANES}x{BOARD_SIZE}x{BOARD_SIZE}"
    options: dict[str, object] = {
        "trt_fp16_enable": fp16,
        "trt_profile_min_shapes": shape.format(1),
        "trt_profile_opt_shapes": shape.format(max_batch),
        "trt_profile_max_shapes": shape.format(max_batch),
        "trt_detailed_build_log": verbose,
    }
    if cache_dir is not None:
        options["trt_engine_cache_enable"] = True
        options["trt_engine_cache_path"] = str(cache_dir)
    return options


def create_session(
    model: Path, provider: str, fp16: bool, max_batch: int, cache_dir: Path | None, verbose: bool
) -> ort.InferenceSession:
    """指定の実行プロバイダだけでセッションを作る。

    Raises:
        RuntimeError: 指定の実行プロバイダを初期化できず、ONNX Runtimeが別の実行プロバイダへ
            切り替えた場合。
    """
    if provider == "tensorrt":
        # pipのtensorrt-cu13が同梱するlibnvinfer.so.10をプロセスへ読み込み、
        # TensorRT実行プロバイダが共有ライブラリの検索パスに依存せずに見つけられるようにする。
        import tensorrt  # noqa: F401
    options = ort.SessionOptions()
    if verbose:
        ort.set_default_logger_severity(0)
        options.log_severity_level = 0
    name = PROVIDERS[provider]
    session = ort.InferenceSession(
        str(model),
        sess_options=options,
        providers=[(name, provider_options(provider, fp16, max_batch, cache_dir, verbose))],
    )
    if session.get_providers()[0] != name:
        raise RuntimeError(
            f"{name} was not initialized; session providers are {session.get_providers()}"
        )
    return session


def measure(session: ort.InferenceSession, batch: int, warmup: int, iterations: int) -> float:
    """ウォームアップの後、`iterations`回の推論に要した秒数を返す。"""
    rng = np.random.default_rng(0)
    shape = (batch, INPUT_PLANES, BOARD_SIZE, BOARD_SIZE)
    inputs = {INPUT_NAME: (rng.random(shape) < PIECE_DENSITY).astype(np.float32)}
    for _ in range(warmup):
        session.run(None, inputs)
    start = time.perf_counter()
    for _ in range(iterations):
        session.run(None, inputs)
    return time.perf_counter() - start


def parse_batches(text: str) -> list[int]:
    """カンマ区切りのバッチの大きさを読む。"""
    try:
        batches = [int(item) for item in text.split(",")]
    except ValueError as error:
        raise argparse.ArgumentTypeError(f"invalid batch list {text!r}") from error
    if not batches or any(batch < 1 for batch in batches):
        raise argparse.ArgumentTypeError(f"invalid batch list {text!r}")
    return batches


def main(argv: list[str] | None = None) -> None:
    """コマンドラインの入口。"""
    parser = argparse.ArgumentParser(description="ONNXのネットの推論速度を測る。")
    parser.add_argument("--model", type=Path, required=True, help="計測するONNXファイル")
    parser.add_argument("--provider", choices=sorted(PROVIDERS), required=True)
    parser.add_argument(
        "--batches", type=parse_batches, default=[64, 128, 256, 512], help="例 64,128,256,512"
    )
    parser.add_argument("--warmup", type=int, default=20, help="計測前の推論の回数")
    parser.add_argument("--iterations", type=int, default=100, help="計測する推論の回数")
    parser.add_argument(
        "--fp16",
        action=argparse.BooleanOptionalAction,
        help="TensorRTのFP16を有効にするか（tensorrtだけで指定でき、既定は有効）",
    )
    parser.add_argument(
        "--trt-cache-dir", type=Path, help="TensorRTのエンジンのキャッシュを置くディレクトリ"
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="ONNX Runtimeの詳細ログとTensorRTの詳細なビルドログを標準エラーへ出す",
    )
    args = parser.parse_args(argv)
    if args.provider != "tensorrt" and (args.fp16 is not None or args.trt_cache_dir is not None):
        parser.error("--fp16/--no-fp16 and --trt-cache-dir require --provider tensorrt")
    if args.warmup < 0 or args.iterations < 1:
        parser.error("--warmup must be >= 0 and --iterations must be >= 1")
    fp16 = args.provider == "tensorrt" and args.fp16 is not False
    uses_gpu = args.provider != "cpu"

    baseline = gpu_memory_used_mib() if uses_gpu else None
    session = create_session(
        args.model, args.provider, fp16, max(args.batches), args.trt_cache_dir, args.verbose
    )
    print("\t".join(HEADER))
    for batch in args.batches:
        seconds = measure(session, batch, args.warmup, args.iterations)
        used = gpu_memory_used_mib() if uses_gpu else None
        row = (
            args.provider,
            str(fp16).lower() if args.provider == "tensorrt" else "-",
            batch,
            args.warmup,
            args.iterations,
            f"{seconds:.6f}",
            f"{batch * args.iterations / seconds:.1f}",
            "-" if used is None else used,
            "-" if baseline is None else baseline,
        )
        print("\t".join(str(value) for value in row))
        sys.stdout.flush()


if __name__ == "__main__":
    main()
