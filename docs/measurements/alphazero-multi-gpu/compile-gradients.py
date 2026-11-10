"""同じ重みとミクロバッチで、コンパイルとbf16による勾配の相対誤差を測る。

リポジトリの外からも、tools/train/zeroを作業ディレクトリとして起動する。
たとえば、リポジトリが/path/to/minaseにある場合は次のコマンドを使う。

    cd /path/to/minase/tools/train/zero
    uv run --frozen python /path/to/minase/docs/measurements/alphazero-multi-gpu/compile-gradients.py \
        --checkpoint /path/to/checkpoint.pt --data /path/to/teacher.mnsd \
        --micro-batch-size 256 --seed 42 --device cuda --out /path/to/gradients.json

--dataの全ファイルを最新サイクルとして読み、学習器の抽出列の先頭から1ミクロバッチを選ぶ。
価値の混合係数は学習チェックポイントのvalue_lambdaを使う。
initのチェックポイントは学習条件を持たないため、測定条件としてvalue_lambda=0.333を使い、出力にも記録する。
3通りともtrainモードで、各計算の前に重みとBatchNormの移動統計をチェックポイントの値に戻す。
CPUでは3通りともfp32で実行し、道具の動作確認だけに使う。
採否の条件はCUDAで測ったコンパイルの相対誤差が、bf16とfp32の相対誤差より小さいことである。
"""

import argparse
from contextlib import nullcontext
import json
import math
from pathlib import Path
import platform
import sys

# package=falseの環境でスクリプトの絶対パスを指定しても、同じリポジトリの学習器を読む。
sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tools/train/zero"))

import numpy as np
import torch
import torch._inductor.config

from minase_zero.data import read_replay_buffer
from minase_zero.encoding import policy_mask
from minase_zero.train import (
    _batch, _load_checkpoint, _model, _policy_batch, _validate_device,
    compile_training, compiled_forward_context, compute_training_losses, total_loss,
)


def compare_gradients(checkpoint: Path, paths: list[Path], micro_batch_size: int,
                      seed: int, device_name: str) -> dict:
    """全パラメータのクリップ前の勾配を比較し、測定条件とともに返す。"""
    if micro_batch_size <= 0:
        raise ValueError("micro-batch-size must be positive")
    if not 0 <= seed <= 0xFFFF_FFFF_FFFF_FFFF:
        raise ValueError("seed must fit in uint64")
    precision = "bf16" if device_name == "cuda" else "fp32"
    device = _validate_device(device_name, precision)
    state = _load_checkpoint(checkpoint)
    if state["optimizer"] is None:
        value_lambda, value_lambda_source = 0.333, "init measurement condition"
    else:
        value_lambda, value_lambda_source = state["config"]["value_lambda"], "checkpoint"
    if not math.isfinite(value_lambda) or not 0 <= value_lambda <= 1:
        raise ValueError("value_lambda must be between 0 and 1")
    data, _, selected, retained = read_replay_buffer(
        paths, paths, seed, rank=0, world_size=1,
        batch_size=micro_batch_size, micro_batch_size=micro_batch_size, replay_threads=1,
    )
    indices = np.flatnonzero(~data.validation)[np.searchsorted(retained, selected[:micro_batch_size])]
    mask = torch.from_numpy(policy_mask()).to(device)
    opponent_mask = torch.from_numpy(policy_mask(opponent=True)).to(device)
    opponent = _policy_batch(data, indices, device, "opponent_")
    short_term = torch.from_numpy(data.short_term_values[indices]).to(device)
    network = _model(state).to(device).train()
    compiled = compile_training(network)
    gradients = []
    for forward, case_precision, context in (
        (network.forward_logits, precision, nullcontext()),
        (compiled, precision, compiled_forward_context()),
        (network.forward_logits, "fp32", nullcontext()),
    ):
        network.load_state_dict(state["model"])
        network.zero_grad(set_to_none=True)
        with context:
            tensors = _batch(forward, data, indices, device, {"precision": case_precision})
        losses = compute_training_losses(*tensors, opponent, short_term, value_lambda, mask, opponent_mask)
        total_loss(losses).backward()
        parts = []
        for name, parameter in network.named_parameters():
            if parameter.grad is None:
                raise ValueError(f"parameter has no gradient: {name}")
            parts.append(parameter.grad.detach().flatten().cpu())
        # ノルムの集計はCPUの倍精度で行い、GPUの逆伝播には単精度を使う。
        gradient = torch.cat(parts).double()
        if not torch.isfinite(gradient).all():
            raise ValueError("non-finite gradient")
        gradients.append(gradient)
    eager, compiled_gradient, fp32 = gradients
    norms = [torch.linalg.vector_norm(gradient).item() for gradient in gradients]
    if norms[0] == 0:
        raise ValueError("eager gradient norm is zero; relative error is undefined")
    compile_error = torch.linalg.vector_norm(eager - compiled_gradient).item() / norms[0]
    precision_error = torch.linalg.vector_norm(eager - fp32).item() / norms[0]
    return {
        "checkpoint": str(checkpoint), "data": [str(path) for path in paths], "seed": seed,
        "sample_indices": selected[:micro_batch_size].tolist(),
        "device": device_name,
        "device_name": torch.cuda.get_device_name(device) if device.type == "cuda" else platform.machine(),
        "pytorch_version": str(torch.__version__), "micro_batch_size": micro_batch_size,
        "emulate_precision_casts": torch._inductor.config.emulate_precision_casts,
        "precision": precision, "cpu_smoke_test": device.type == "cpu",
        "precision_note": ("CPU smoke test: all three runs use fp32; bf16 is not measured."
                           if device.type == "cpu" else "Runs 1 and 2 use bf16 autocast; run 3 uses fp32."),
        "value_lambda": value_lambda, "value_lambda_source": value_lambda_source,
        "gradient_norms": {"eager": norms[0], "compiled": norms[1], "eager_fp32": norms[2]},
        "relative_error_compile": compile_error,
        "relative_error_bf16_fp32": precision_error,
        "acceptance_condition": "relative_error_compile < relative_error_bf16_fp32",
        "acceptance_applicable": device.type == "cuda",
        "acceptance_passed": compile_error < precision_error,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument("--data", type=Path, nargs="+", required=True)
    parser.add_argument("--micro-batch-size", type=int, required=True)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--device", choices=("cuda", "cpu"), required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    result = compare_gradients(args.checkpoint, args.data, args.micro_batch_size, args.seed, args.device)
    args.out.write_text(json.dumps(result, ensure_ascii=False, allow_nan=False, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
