"""Measure ZeroNet training steps on synthetic data, excluding data loading."""

import argparse
from contextlib import nullcontext
import json
import time

import torch
from torch.nn import functional as F

from make_nets import INPUT_PLANES, ZeroNet


def positive_int(value: str) -> int:
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return number


def nonnegative_int(value: str) -> int:
    number = int(value)
    if number < 0:
        raise argparse.ArgumentTypeError("must be a nonnegative integer")
    return number


def batch_sizes(value: str) -> list[int]:
    try:
        return [positive_int(part) for part in value.split(",")]
    except (ValueError, argparse.ArgumentTypeError) as error:
        raise argparse.ArgumentTypeError(
            "must be comma-separated positive integers"
        ) from error


def make_batch(
    batch: int, device: torch.device
) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
    inputs = torch.empty(
        batch, INPUT_PLANES, 12, 12, device=device, dtype=torch.float32
    ).bernoulli_(0.01)
    policy_target = torch.zeros(batch, 35_856, device=device, dtype=torch.float32)
    # Sampling with replacement leaves about 80 positive entries in each row.
    indices = torch.randint(35_856, (batch, 80), device=device)
    weights = 1.0 - torch.rand(batch, 80, device=device, dtype=torch.float32)
    policy_target.scatter_add_(1, indices, weights)
    policy_target.div_(policy_target.sum(dim=1, keepdim=True))
    value_target = torch.rand(batch, 1, device=device, dtype=torch.float32)
    return inputs, policy_target, value_target


def benchmark(args: argparse.Namespace, batch: int) -> dict[str, int | float | str | None]:
    device = torch.device(args.device)
    model = ZeroNet(args.blocks, args.channels).to(device).train()
    optimizer = torch.optim.SGD(
        model.parameters(), lr=0.01, momentum=0.9, weight_decay=1e-4
    )
    inputs, policy_target, value_target = make_batch(batch, device)

    def step() -> None:
        optimizer.zero_grad(set_to_none=True)
        with (
            torch.autocast("cuda", dtype=torch.bfloat16)
            if args.precision == "bf16"
            else nullcontext()
        ):
            policy_logits, value = model(inputs)
            policy_loss = -(
                policy_target * F.log_softmax(policy_logits, dim=1)
            ).sum(dim=1).mean()
        # BCE on sigmoid probabilities must run outside CUDA autocast.
        value_loss = F.binary_cross_entropy(value.float(), value_target.float())
        loss = policy_loss + value_loss
        loss.backward()
        optimizer.step()

    for _ in range(args.warmup):
        step()
    if device.type == "cuda":
        torch.cuda.synchronize(device)
        torch.cuda.reset_peak_memory_stats(device)

    start = time.perf_counter()
    for _ in range(args.steps):
        step()
    if device.type == "cuda":
        torch.cuda.synchronize(device)
    elapsed = time.perf_counter() - start

    positions_per_sec = batch * args.steps / elapsed
    return {
        "blocks": args.blocks,
        "channels": args.channels,
        "precision": args.precision,
        "batch": batch,
        "steps": args.steps,
        "elapsed_secs": elapsed,
        "secs_per_step": elapsed / args.steps,
        "positions_per_sec": positions_per_sec,
        "max_memory_allocated_mib": (
            torch.cuda.max_memory_allocated(device) / (1024 * 1024)
            if device.type == "cuda"
            else None
        ),
        "torch_version": torch.__version__,
        "device_name": (
            torch.cuda.get_device_name(device) if device.type == "cuda" else "cpu"
        ),
        "secs_per_4m_positions": 4_000_000 / positions_per_sec,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--blocks", type=positive_int, required=True)
    parser.add_argument("--channels", type=positive_int, required=True)
    parser.add_argument("--batches", type=batch_sizes, required=True)
    parser.add_argument("--precision", choices=("fp32", "bf16"), required=True)
    parser.add_argument("--steps", type=positive_int, required=True)
    parser.add_argument("--warmup", type=nonnegative_int, required=True)
    parser.add_argument("--device", choices=("cuda", "cpu"), required=True)
    args = parser.parse_args()
    if args.device == "cpu" and args.precision == "bf16":
        parser.error("--precision bf16 requires --device cuda")
    if args.device == "cuda" and not torch.cuda.is_available():
        parser.error("--device cuda requested, but CUDA is unavailable")

    for batch in args.batches:
        print(json.dumps(benchmark(args, batch), allow_nan=False), flush=True)


if __name__ == "__main__":
    main()
