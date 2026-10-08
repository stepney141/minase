"""MNZTの学習データからネットを学習し、ONNXへ出力する。

損失は、方策の教師分布との交差エントロピーと、価値の二値交差エントロピーの重みつき和である
（docs/plans/alphazero.md の「教師あり学習」と「強化学習」）。
学習の状態は実行ディレクトリの`checkpoint.pt`へ一定のステップごとに保存し、`--resume`で最後の保存から続ける。

実行ディレクトリには次のファイルを書く。

- `checkpoint.pt`: 重み、最適化器の状態、ステップ数、乱数の状態、学習の設定、および学習データの検査和。
- `log.jsonl`: 学習損失（`"kind": "train"`）と検証の指標（`"kind": "validation"`）を1行ずつ追記する。
- `net.onnx`、`net.sha256`: 学習を終えたネットと、その検査和。
- `provenance.json`: 学習データの経路と検査和、設定、シード、PyTorchの版、および開始と終了の時刻。

使い方:
    python -m minase_zero.train --data a.mnzt b.mnzt --out run_dir --lr 0.02 --seed 1
    python -m minase_zero.train --resume run_dir
"""

from __future__ import annotations

import argparse
import copy
import json
import math
import os
import sys
import time
from dataclasses import dataclass
from datetime import UTC, datetime
from pathlib import Path

import torch
import torch.nn.functional as F

from minase_zero.dataset import (
    BatchDataset,
    split_by_game,
    train_loader,
    validation_loader,
)
from minase_zero.export import export_onnx
from minase_zero.mnzt import MnztCollection, sha256_file
from minase_zero.model import ZeroNet

CHECKPOINT_FORMAT = "minase-zero-checkpoint"
"""チェックポイントの形式名。"""
CHECKPOINT_VERSION = 1
"""チェックポイントの形式の版。"""

# 学習の結果を決める設定の名前。チェックポイントに保存し、`--resume`ではこの値を使う。
CONFIG_KEYS = (
    "blocks",
    "channels",
    "batch_size",
    "epochs",
    "lr",
    "lr_schedule",
    "momentum",
    "weight_decay",
    "value_lambda",
    "policy_weight",
    "value_weight",
    "val_fraction",
    "seed",
    "log_interval",
    "eval_interval",
    "checkpoint_interval",
)


def policy_loss(logits: torch.Tensor, labels: torch.Tensor, probs: torch.Tensor) -> torch.Tensor:
    """局面ごとの方策の交差エントロピー`−Σ_a π(a)·log softmax(logit)(a)`を返す。

    教師分布`π`は（ラベル、確率）の組で与え、埋め草の組は確率0なので和に寄与しない。
    教師にない手（非合法手を含む）の目標は0である。
    1局面の合法手のラベルは互いに異なるので、密な目標`[N, 35856]`へ`scatter`して内積をとる計算と同じ値になる。
    """
    log_probs = torch.log_softmax(logits, dim=1)
    return -(probs * log_probs.gather(1, labels)).sum(dim=1)


def value_loss(value: torch.Tensor, target: torch.Tensor) -> torch.Tensor:
    """局面ごとの価値の二値交差エントロピーを返す。`value`はシグモイド後の`[N, 1]`である。"""
    return F.binary_cross_entropy(value.squeeze(1), target, reduction="none")


def policy_agreement(logits: torch.Tensor, labels: torch.Tensor, probs: torch.Tensor) -> torch.Tensor:
    """方策のlogitが最大の手と、教師の確率が最大の手が一致したかを局面ごとに返す。

    ネット側は合法手に限らず全出力の最大をとる。学習器は合法手の集合を持たないからである。
    """
    teacher = labels.gather(1, probs.argmax(dim=1, keepdim=True)).squeeze(1)
    return logits.argmax(dim=1) == teacher


def learning_rate(config: dict, step: int, total_steps: int) -> float:
    """ステップ`step`の学習率を返す。コサインでは総ステップ数で0へ下げる。"""
    if config["lr_schedule"] == "constant":
        return config["lr"]
    return 0.5 * config["lr"] * (1.0 + math.cos(math.pi * step / total_steps))


@dataclass
class TrainingState:
    """学習中に変わる状態。"""

    model: ZeroNet
    optimizer: torch.optim.Optimizer
    step: int


def now() -> str:
    """現在時刻をUTCのISO 8601表記で返す。"""
    return datetime.now(UTC).isoformat(timespec="seconds")


def atomic_write_bytes(path: Path, write) -> None:
    """一時ファイルへ`write(stream)`で書いて同期し、`path`へ名前を変えて確定させる。"""
    temporary = path.with_name(path.name + ".tmp")
    with temporary.open("wb") as stream:
        write(stream)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)


def atomic_write_text(path: Path, text: str) -> None:
    """文字列を原子的に書く。"""
    atomic_write_bytes(path, lambda stream: stream.write(text.encode("utf-8")))


def save_checkpoint(path: Path, state: TrainingState, meta: dict) -> None:
    """重み、最適化器の状態、ステップ数、乱数の状態、および`meta`を原子的に保存する。

    CUDAの乱数の状態は学習に使う装置の分だけを保存する。GPUの数が異なる機械でも再開できるようにするためである。
    """
    device = next(state.model.parameters()).device
    payload = {
        "format": CHECKPOINT_FORMAT,
        "version": CHECKPOINT_VERSION,
        **meta,
        "model": state.model.state_dict(),
        "optimizer": state.optimizer.state_dict(),
        "step": state.step,
        "torch_rng": torch.get_rng_state(),
        "cuda_rng": torch.cuda.get_rng_state(device) if device.type == "cuda" else None,
    }
    atomic_write_bytes(path, lambda stream: torch.save(payload, stream))


def load_checkpoint(path: Path) -> dict:
    """チェックポイントを読み、形式名と版を検査する。

    Raises:
        ValueError: 形式名または版が異なる場合。
    """
    payload = torch.load(path, map_location="cpu", weights_only=True)
    if payload.get("format") != CHECKPOINT_FORMAT or payload.get("version") != CHECKPOINT_VERSION:
        raise ValueError(f"{path}: not a {CHECKPOINT_FORMAT} version {CHECKPOINT_VERSION} file")
    return payload


def truncate_log(path: Path, step: int) -> None:
    """`log.jsonl`から、ステップ数が`step`を超える行を除く。保存後に書かれて再開で書き直す行である。"""
    if not path.exists():
        return
    lines = path.read_text(encoding="utf-8").splitlines()
    kept = [line for line in lines if json.loads(line)["step"] <= step]
    atomic_write_text(path, "".join(line + "\n" for line in kept))


def append_log(path: Path, entry: dict) -> None:
    """`log.jsonl`へ1行を追記する。"""
    with path.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(entry) + "\n")


@torch.no_grad()
def evaluate(model: ZeroNet, loader, device: torch.device, config: dict) -> dict:
    """検証集合の方策の交差エントロピー、方策の最大の手の一致率、および価値の損失を平均して返す。"""
    model.eval()
    totals = {"policy_loss": 0.0, "policy_agreement": 0.0, "value_loss": 0.0}
    positions = 0
    for batch in loader:
        batch = {key: tensor.to(device, non_blocking=True) for key, tensor in batch.items()}
        logits, value = model(batch["input"])
        labels, probs = batch["policy_labels"], batch["policy_probs"]
        totals["policy_loss"] += policy_loss(logits, labels, probs).sum().item()
        totals["policy_agreement"] += policy_agreement(logits, labels, probs).sum().item()
        totals["value_loss"] += value_loss(value, batch["value"]).sum().item()
        positions += len(labels)
    result = {key: total / positions for key, total in totals.items()}
    result["loss"] = (
        config["policy_weight"] * result["policy_loss"]
        + config["value_weight"] * result["value_loss"]
    )
    result["positions"] = positions
    return result


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    """コマンドライン引数を読み、新規の学習と再開の組合せを検査する。"""
    parser = argparse.ArgumentParser(description="MNZTの学習データからZeroNetを学習する。")
    parser.add_argument("--data", type=Path, nargs="+", help="学習データのMNZTファイル")
    parser.add_argument("--out", type=Path, help="新しく作る実行ディレクトリ")
    parser.add_argument(
        "--resume", type=Path, help="中断した実行ディレクトリ。設定はチェックポイントから読む"
    )
    parser.add_argument(
        "--init", type=Path, help="重みだけを読み込むチェックポイント（前のサイクルのcheckpoint.pt）"
    )
    parser.add_argument("--blocks", type=int, default=10, help="残差ブロックの数")
    parser.add_argument("--channels", type=int, default=128, help="チャネル数")
    parser.add_argument("--batch-size", type=int, default=1024, help="バッチの大きさ")
    parser.add_argument("--epochs", type=int, default=1, help="エポック数")
    parser.add_argument("--lr", type=float, help="学習率の初期値（必須）")
    parser.add_argument(
        "--lr-schedule", choices=("cosine", "constant"), default="cosine", help="学習率の推移"
    )
    parser.add_argument("--momentum", type=float, default=0.9, help="SGDのモーメンタム")
    parser.add_argument("--weight-decay", type=float, default=1e-4, help="SGDの重み減衰")
    parser.add_argument(
        "--lambda", dest="value_lambda", type=float, default=0.333,
        help="価値の教師で根の価値に掛ける重みλ",
    )
    parser.add_argument("--policy-weight", type=float, default=1.0, help="方策の損失の重み")
    parser.add_argument("--value-weight", type=float, default=1.0, help="価値の損失の重み")
    parser.add_argument("--val-fraction", type=float, default=0.05, help="検証に回す対局の割合")
    parser.add_argument("--seed", type=int, help="初期化、分割、およびバッチの順序のシード（必須）")
    parser.add_argument("--log-interval", type=int, default=100, help="学習損失を記録する間隔のステップ数")
    parser.add_argument("--eval-interval", type=int, default=1000, help="検証する間隔のステップ数")
    parser.add_argument(
        "--checkpoint-interval", type=int, default=1000, help="チェックポイントを保存する間隔のステップ数"
    )
    parser.add_argument("--workers", type=int, default=4, help="バッチを作るワーカーの数")
    parser.add_argument("--device", default="cuda", help="学習に使う装置（cuda、cpuなど）")
    args = parser.parse_args(argv)

    if args.resume is not None:
        given = [
            f"--{key.replace('_', '-')}" for key in (*CONFIG_KEYS, "out", "init")
            if getattr(args, key) != parser.get_default(key)
        ]
        if given:
            parser.error(f"--resume reads the settings from the checkpoint; remove {', '.join(given)}")
        return args
    missing = [name for name, value in (("--data", args.data), ("--out", args.out),
                                        ("--lr", args.lr), ("--seed", args.seed)) if value is None]
    if missing:
        parser.error(f"a new run requires {', '.join(missing)}")
    for name in ("batch_size", "epochs", "log_interval", "eval_interval", "checkpoint_interval"):
        if getattr(args, name) < 1:
            parser.error(f"--{name.replace('_', '-')} must be positive")
    return args


def data_provenance(paths: list[Path]) -> list[dict]:
    """学習データの経路と検査和の一覧を作る。"""
    return [{"path": str(path), "sha256": sha256_file(path)} for path in paths]


def main(argv: list[str] | None = None) -> None:
    """コマンドラインの入口。"""
    args = parse_args(argv)
    device = torch.device(args.device)

    if args.resume is not None:
        run_dir = args.resume
        checkpoint = load_checkpoint(run_dir / "checkpoint.pt")
        config = checkpoint["config"]
        data_paths = [Path(path) for path in args.data] if args.data else [
            Path(entry["path"]) for entry in checkpoint["data"]
        ]
        data = data_provenance(data_paths)
        recorded = [entry["sha256"] for entry in checkpoint["data"]]
        if [entry["sha256"] for entry in data] != recorded:
            sys.exit("error: the training data checksums differ from the checkpoint")
        meta = {"config": config, "data": data, "init": checkpoint["init"],
                "started_at": checkpoint["started_at"]}
        print(f"resuming {run_dir} from step {checkpoint['step']}", flush=True)
    else:
        run_dir = args.out
        if run_dir.exists():
            sys.exit(f"error: run directory {run_dir} already exists")
        config = {key: getattr(args, key) for key in CONFIG_KEYS}
        data_paths = args.data
        data = data_provenance(data_paths)
        init = None if args.init is None else {"path": str(args.init), "sha256": sha256_file(args.init)}
        meta = {"config": config, "data": data, "init": init, "started_at": now()}
        checkpoint = None

    collection = MnztCollection.open(data_paths)
    split = split_by_game(collection, config["val_fraction"], config["seed"])
    batch_size = config["batch_size"]
    steps_per_epoch = len(split.train) // batch_size
    if steps_per_epoch == 0:
        sys.exit(f"error: {len(split.train)} training positions are fewer than one batch of {batch_size}")
    total_steps = steps_per_epoch * config["epochs"]
    print(
        f"positions: {len(collection)} in {split.games} games; "
        f"training {len(split.train)}, validation {len(split.validation)} "
        f"in {split.validation_games} games",
        flush=True,
    )
    print(f"steps per epoch: {steps_per_epoch}; total steps: {total_steps}", flush=True)
    if len(split.validation) == 0:
        print("validation set is empty; validation is skipped", flush=True)

    torch.manual_seed(config["seed"])
    model = ZeroNet(config["blocks"], config["channels"])
    if checkpoint is None and args.init is not None:
        model.load_state_dict(load_checkpoint(args.init)["model"])
    model.to(device)
    optimizer = torch.optim.SGD(
        model.parameters(), lr=config["lr"], momentum=config["momentum"],
        weight_decay=config["weight_decay"],
    )
    state = TrainingState(model, optimizer, 0)
    checkpoint_path = run_dir / "checkpoint.pt"
    log_path = run_dir / "log.jsonl"
    if checkpoint is None:
        run_dir.mkdir(parents=True)
        save_checkpoint(checkpoint_path, state, meta)
    else:
        model.load_state_dict(checkpoint["model"])
        optimizer.load_state_dict(checkpoint["optimizer"])
        state.step = checkpoint["step"]
        torch.set_rng_state(checkpoint["torch_rng"])
        if checkpoint["cuda_rng"] is not None and device.type == "cuda":
            torch.cuda.set_rng_state(checkpoint["cuda_rng"], device)
        truncate_log(log_path, state.step)

    dataset = BatchDataset(collection, config["value_lambda"])
    pin_memory = device.type == "cuda"
    validation = validation_loader(dataset, split.validation, batch_size, args.workers, pin_memory)

    def run_validation(epoch: int) -> None:
        if len(split.validation) == 0:
            return
        result = evaluate(model, validation, device, config)
        append_log(log_path, {"kind": "validation", "step": state.step, "epoch": epoch, **result})
        print(f"step {state.step} validation {result}", flush=True)

    sums = {"loss": 0.0, "policy_loss": 0.0, "value_loss": 0.0}
    window_steps = 0
    window_start = time.perf_counter()
    while state.step < total_steps:
        epoch, start_batch = divmod(state.step, steps_per_epoch)
        loader = train_loader(
            dataset, split.train, batch_size, config["seed"], epoch, start_batch, args.workers,
            pin_memory,
        )
        for batch in loader:
            model.train()
            lr = learning_rate(config, state.step, total_steps)
            for group in optimizer.param_groups:
                group["lr"] = lr
            batch = {key: tensor.to(device, non_blocking=True) for key, tensor in batch.items()}
            logits, value = model(batch["input"])
            p_loss = policy_loss(logits, batch["policy_labels"], batch["policy_probs"]).mean()
            v_loss = value_loss(value, batch["value"]).mean()
            loss = config["policy_weight"] * p_loss + config["value_weight"] * v_loss
            optimizer.zero_grad(set_to_none=True)
            loss.backward()
            optimizer.step()
            state.step += 1

            sums["loss"] += loss.item()
            sums["policy_loss"] += p_loss.item()
            sums["value_loss"] += v_loss.item()
            window_steps += 1
            end_of_epoch = state.step % steps_per_epoch == 0
            if state.step % config["log_interval"] == 0 or end_of_epoch:
                elapsed = time.perf_counter() - window_start
                entry = {"kind": "train", "step": state.step, "epoch": epoch, "lr": lr}
                entry.update({key: total / window_steps for key, total in sums.items()})
                entry["positions_per_second"] = window_steps * batch_size / elapsed
                append_log(log_path, entry)
                print(f"step {state.step} train {entry}", flush=True)
                sums = dict.fromkeys(sums, 0.0)
                window_steps = 0
                window_start = time.perf_counter()
            if state.step % config["eval_interval"] == 0 or end_of_epoch:
                run_validation(epoch)
            if state.step % config["checkpoint_interval"] == 0 or end_of_epoch:
                save_checkpoint(checkpoint_path, state, meta)

    export_model = copy.deepcopy(model).cpu()
    onnx_path = run_dir / "net.onnx"
    export_onnx(export_model, onnx_path)
    atomic_write_text(run_dir / "net.sha256", f"{sha256_file(onnx_path)}  net.onnx\n")
    provenance = {
        "data": data,
        "init": meta["init"],
        "config": config,
        "seed": config["seed"],
        "device": args.device,
        "workers": args.workers,
        "torch_version": torch.__version__,
        "started_at": meta["started_at"],
        "finished_at": now(),
        "steps": state.step,
    }
    atomic_write_text(run_dir / "provenance.json", json.dumps(provenance, indent=2) + "\n")
    print(f"wrote {onnx_path}", flush=True)


if __name__ == "__main__":
    main()
