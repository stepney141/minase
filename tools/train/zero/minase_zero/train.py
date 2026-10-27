"""自己対局の学習窓を1回ずつ読み、更新の境界で保存して再開する。

trainのsummary.jsonの欄は次のとおりである。

| 欄 | 意味 |
|---|---|
| cycle | サイクル番号。 |
| config | ネットの大きさと、新しいサイクルに指定した設定。出力先は含めない。 |
| provenance | 学習器と環境定義のコミット、実行環境、初期重みと教師ファイルの来歴。 |
| training_count / validation_count | 学習窓の訓練局面数と検証局面数。 |
| completed_steps / total_steps | 完了した更新回数と、このサイクルの全更新回数。 |
| positions_seen | 完了した更新で読んだ訓練局面数。 |
| training_losses / validation_losses | 局面数で平均したpolicy、value、value_resultの損失。 |
| learning_curve | 保存間隔ごとのcompleted_stepsと、その区間のtraining_losses。 |
| data_order_sha256 | 更新に使った局面の通し番号を連鎖させたSHA-256。 |
| network_sha256 | model.onnxのSHA-256。 |
| elapsed_seconds | loading、training、validationの秒数。 |
| positions_per_second | positions_seenをtrainingの秒数で割った値。 |

policyは全35,856ラベルとの交差エントロピー、valueは結果と根の価値を混ぜた
教師に対する二値交差エントロピー、value_resultは結果だけに対する同じ損失である。
損失の計算と累計はfloat32で行う。訓練集合か検証集合が空なら、平均を定義できない
ためエラーにする。シードはuint64、サイクルは0以上の整数とする。

loadingは教師の読み込みと初回の検査和計算、trainingはバッチの準備と更新、
validationは検証のバッチの準備と推論を測る。保存とONNX出力は含めない。
再開時は保存済みの時間に今回の読み込みと更新の時間を加え、中断で失われた更新や
停止中の時間は含めない。再開に指定した機器は実行に使い、configとprovenanceには
サイクル開始時の設定と環境を保持する。

initのsummary.jsonはblocks、channels、seed、parameters、network_sha256、
provenanceだけを持つ。チェックポイントの内部形式は本モジュールだけで扱う。
"""

from collections.abc import Iterator
from contextlib import contextmanager, nullcontext
import copy
from dataclasses import asdict
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import sys
import tempfile
import time

import numpy as np
from numpy.typing import NDArray
import torch
from torch.nn import functional as F

from .data import Dataset, FileInfo, read_window
from .encoding import encode_planes
from .fixture import evaluate_fixture
from .net import ZeroNet, export_onnx


LOSS_NAMES = ("policy", "value", "value_result")


@contextmanager
def _atomic_path(path: Path) -> Iterator[Path]:
    descriptor, name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    os.close(descriptor)
    temporary = Path(name)
    try:
        yield temporary
        with temporary.open("rb") as source:
            os.fsync(source.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        temporary.unlink(missing_ok=True)


def _write_json(path: Path, value: dict) -> None:
    text = json.dumps(value, ensure_ascii=False, allow_nan=False, indent=2) + "\n"
    with _atomic_path(path) as temporary:
        temporary.write_text(text, encoding="utf-8")


def _save_checkpoint(path: Path, state: dict) -> None:
    with _atomic_path(path) as temporary:
        torch.save(state, temporary)


def _sha256(path: Path) -> str:
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def _prepare_output(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    if any(path.iterdir()):
        raise ValueError(f"output directory must be empty: {path}")


def _validate_common(seed: int, trainer_commit: str) -> None:
    if not 0 <= seed <= 0xFFFF_FFFF_FFFF_FFFF:
        raise ValueError("seed must fit in uint64")
    if re.fullmatch(r"[0-9a-fA-F]{40}", trainer_commit) is None:
        raise ValueError("trainer-commit must be 40 hexadecimal digits")


def _validate_device(device: str, precision: str) -> torch.device:
    if device not in ("cpu", "cuda") or precision not in ("fp32", "bf16"):
        raise ValueError("device must be cpu or cuda and precision must be fp32 or bf16")
    if precision == "bf16" and device != "cuda":
        raise ValueError("precision bf16 requires device cuda")
    if device == "cuda" and not torch.cuda.is_available():
        raise ValueError("device cuda requested, but CUDA is unavailable")
    if precision == "bf16" and not torch.cuda.is_bf16_supported():
        raise ValueError("device cuda does not support bf16")
    return torch.device(device)


def _validate_training(config: dict) -> None:
    _validate_common(config["seed"], config["trainer_commit"])
    for name in ("batch_size", "checkpoint_steps"):
        if config[name] <= 0:
            raise ValueError(f"{name} must be positive")
    if config["cycle"] < 0:
        raise ValueError("cycle must be nonnegative")
    for name in ("lr", "momentum", "clip_grad_norm"):
        if not math.isfinite(config[name]) or config[name] <= 0:
            raise ValueError(f"{name} must be finite and positive")
    if not math.isfinite(config["weight_decay"]) or config["weight_decay"] < 0:
        raise ValueError("weight_decay must be finite and nonnegative")
    if not math.isfinite(config["value_lambda"]) or not 0 <= config["value_lambda"] <= 1:
        raise ValueError("value_lambda must be between 0 and 1")


def _environment(trainer_commit: str, device: torch.device) -> dict:
    return {
        "trainer_commit": trainer_commit,
        "python_version": platform.python_version(),
        "pytorch_version": str(torch.__version__),
        "cuda_version": torch.version.cuda,
        "numpy_version": str(np.__version__),
        "device_name": torch.cuda.get_device_name(device) if device.type == "cuda" else platform.machine(),
    }


def _file_provenance(path: str, info: FileInfo) -> dict:
    header = asdict(info.header)
    header["network_checksum"] = info.header.network_checksum.hex()
    return {
        "path": path,
        "size": info.size,
        "sha256": _sha256(info.path),
        "header": header,
        "position_count": info.position_count,
        "training_count": info.training_count,
        "validation_count": info.validation_count,
    }


def _load_checkpoint(path: Path) -> dict:
    state = torch.load(path, map_location="cpu", weights_only=True)
    if not isinstance(state, dict):
        raise ValueError(f"{path}: checkpoint must be a dictionary")
    completed, total = state["completed_steps"], state["total_steps"]
    if type(completed) is not int or type(total) is not int or not 0 <= completed <= total:
        raise ValueError(f"{path}: invalid completed_steps or total_steps")
    if state["config"]["blocks"] <= 0 or state["config"]["channels"] <= 0:
        raise ValueError(f"{path}: blocks and channels must be positive")
    return state


def _model(state: dict) -> ZeroNet:
    model = ZeroNet(state["config"]["blocks"], state["config"]["channels"])
    model.load_state_dict(state["model"])
    return model


def initialize(blocks: int, channels: int, seed: int, out_dir: Path, trainer_commit: str) -> None:
    """指定のシードの直後にネットを構築し、初期重みとONNXと来歴を確定させる。"""
    _validate_common(seed, trainer_commit)
    if blocks <= 0 or channels <= 0:
        raise ValueError("blocks and channels must be positive")
    _prepare_output(out_dir)
    torch.manual_seed(seed)
    model = ZeroNet(blocks, channels)
    provenance = _environment(trainer_commit, torch.device("cpu"))
    state = {
        "model": model.state_dict(), "optimizer": None,
        "completed_steps": 0, "total_steps": 0, "cycle": 0, "seed": seed,
        "config": {"blocks": blocks, "channels": channels, "seed": seed, "trainer_commit": trainer_commit},
        "provenance": provenance,
    }
    _save_checkpoint(out_dir / "checkpoint.pt", state)
    with _atomic_path(out_dir / "model.onnx") as temporary:
        export_onnx(model.eval(), temporary)
    _write_json(out_dir / "summary.json", {
        "blocks": blocks, "channels": channels, "seed": seed,
        "parameters": sum(parameter.numel() for parameter in model.parameters()),
        "network_sha256": _sha256(out_dir / "model.onnx"), "provenance": provenance,
    })


def compute_losses(
    policy_logits: torch.Tensor, value_logits: torch.Tensor,
    rows: torch.Tensor, labels: torch.Tensor, probabilities: torch.Tensor,
    results: torch.Tensor, values: torch.Tensor, value_lambda: float,
) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
    """疎な方策と量子化した価値から、単精度の平均損失3つを計算する。

    rowsとlabelsは候補の行番号とラベル、probabilitiesとvaluesは0〜65,535、
    resultsは手番側の結果である。結果だけの監視用損失には勾配を持たせない。
    """
    policy_logits, value_logits = policy_logits.float(), value_logits.float().flatten()
    probabilities, results, values = probabilities.float(), results.float(), values.float()
    candidate_losses = -F.log_softmax(policy_logits, dim=1)[rows, labels] * (probabilities / 65_535)
    per_position = torch.zeros(len(results), dtype=torch.float32, device=policy_logits.device)
    policy_loss = per_position.scatter_add_(0, rows, candidate_losses).mean()
    target = (1 - value_lambda) * results + value_lambda * (values / 65_535)
    value_loss = F.binary_cross_entropy_with_logits(value_logits, target)
    with torch.no_grad():
        result_loss = F.binary_cross_entropy_with_logits(value_logits, results)
    return policy_loss, value_loss, result_loss


def _batch_losses(
    model: ZeroNet, data: Dataset, indices: NDArray[np.int64],
    device: torch.device, config: dict,
) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
    boards, sides, lions = (
        torch.from_numpy(array[indices]).to(device) for array in (data.boards, data.sides, data.lions)
    )
    inputs = encode_planes(boards, sides, lions)
    starts, ends = data.policy_offsets[indices], data.policy_offsets[indices + 1]
    candidates = np.concatenate([np.arange(start, end, dtype=np.int64) for start, end in zip(starts, ends)])
    rows = torch.from_numpy(np.repeat(np.arange(len(indices)), (ends - starts).astype(np.int64))).to(device)
    labels = torch.from_numpy(data.policy_labels[candidates].astype(np.int64)).to(device)
    probabilities = torch.from_numpy(data.policy_probabilities[candidates].astype(np.float32)).to(device)
    results = torch.from_numpy(data.results[indices]).to(device)
    values = torch.from_numpy(data.values[indices].astype(np.float32)).to(device)
    with torch.autocast("cuda", dtype=torch.bfloat16) if config["precision"] == "bf16" else nullcontext():
        policy_logits, value_logits = model.forward_logits(inputs)
    return compute_losses(
        policy_logits, value_logits, rows, labels, probabilities, results, values, config["value_lambda"],
    )


def _zero_losses() -> dict[str, float]:
    return dict.fromkeys(LOSS_NAMES, 0.0)


def _accumulate(sums: dict[str, float], losses: tuple[torch.Tensor, ...], count: int) -> None:
    for name, loss in zip(LOSS_NAMES, losses):
        sums[name] = float(np.float32(sums[name]) + np.float32(loss.detach().item()) * np.float32(count))


def _mean(sums: dict[str, float], count: int) -> dict[str, float]:
    return {name: float(np.float32(value) / np.float32(count)) for name, value in sums.items()}


def _counts(data: Dataset) -> tuple[int, int]:
    validation = int(np.count_nonzero(data.validation))
    training = len(data) - validation
    if training == 0 or validation == 0:
        raise ValueError("training and validation sets must both contain positions")
    return training, validation


def train_new(config: dict, out_dir: Path) -> None:
    """完了したチェックポイントから、指定した学習窓で新しいサイクルを始める。"""
    _validate_training(config)
    device = _validate_device(config["device"], config["precision"])
    initial = _load_checkpoint(Path(config["init"]))
    if initial["completed_steps"] != initial["total_steps"]:
        raise ValueError("init checkpoint has unfinished updates")
    _prepare_output(out_dir)
    config = {**config, "blocks": initial["config"]["blocks"], "channels": initial["config"]["channels"]}
    started = time.perf_counter()
    data, infos = read_window(config["data"])
    training_count, validation_count = _counts(data)
    provenance = _environment(config["trainer_commit"], device)
    provenance["init_sha256"] = _sha256(Path(config["init"]))
    provenance["files"] = [_file_provenance(path, info) for path, info in zip(config["data"], infos)]
    state = {
        "model": initial["model"], "optimizer": initial["optimizer"],
        "completed_steps": 0, "total_steps": (training_count + config["batch_size"] - 1) // config["batch_size"],
        "cycle": config["cycle"], "seed": config["seed"], "config": config, "provenance": provenance,
        "training_count": training_count, "validation_count": validation_count,
        "positions_seen": 0, "loss_sums": _zero_losses(), "learning_curve": [],
        "data_order_sha256": bytes(32).hex(),
        "elapsed_seconds": {"loading": time.perf_counter() - started, "training": 0.0},
    }
    _run(state, data, out_dir, device)


def resume(out_dir: Path, device_name: str) -> None:
    """保存済みのパス・大きさ・局面数を照合し、最後に確定した更新から再開する。"""
    if (out_dir / "summary.json").exists():
        raise ValueError(f"training is already complete: {out_dir}")
    state = _load_checkpoint(out_dir / "checkpoint.pt")
    if state["optimizer"] is None:
        raise ValueError("resume requires a training checkpoint")
    config = state["config"]
    _validate_training(config)
    device = _validate_device(device_name, config["precision"])
    records = state["provenance"]["files"]
    paths = [record["path"] for record in records]
    if config["data"] != paths:
        raise ValueError("training window path sequence differs from provenance")
    started = time.perf_counter()
    data, infos = read_window(paths)
    for record, info in zip(records, infos):
        if (record["size"], record["training_count"], record["validation_count"]) != (
            info.size, info.training_count, info.validation_count,
        ):
            raise ValueError(f"training window size or position counts differ: {record['path']}")
    if _counts(data) != (state["training_count"], state["validation_count"]):
        raise ValueError("training window total position counts differ")
    batch_size = config["batch_size"]
    if state["total_steps"] != (state["training_count"] + batch_size - 1) // batch_size:
        raise ValueError("checkpoint total_steps disagrees with training_count and batch_size")
    if state["positions_seen"] != min(state["completed_steps"] * batch_size, state["training_count"]):
        raise ValueError("checkpoint positions_seen disagrees with completed_steps")
    state["elapsed_seconds"]["loading"] += time.perf_counter() - started
    print(f"resume: completed_steps={state['completed_steps']}/{state['total_steps']}", file=sys.stderr, flush=True)
    _run(state, data, out_dir, device)


def _run(state: dict, data: Dataset, out_dir: Path, device: torch.device) -> None:
    config = state["config"]
    model = _model(state).to(device).train()
    optimizer = torch.optim.SGD(
        model.parameters(), lr=config["lr"], momentum=config["momentum"],
        weight_decay=config["weight_decay"], nesterov=True,
    )
    if state["optimizer"] is not None:
        optimizer.load_state_dict(state["optimizer"])
        for group in optimizer.param_groups:
            group.update(lr=config["lr"], momentum=config["momentum"], weight_decay=config["weight_decay"],
                         dampening=0, nesterov=True)
    indices = np.flatnonzero(~data.validation)
    order = indices[np.random.Generator(np.random.PCG64(config["seed"])).permutation(len(indices))]
    batch_size = config["batch_size"]
    interval_sums, interval_count = _zero_losses(), 0
    interval_started = time.perf_counter()
    for step in range(state["completed_steps"], state["total_steps"]):
        batch = order[step * batch_size:(step + 1) * batch_size]
        optimizer.zero_grad(set_to_none=True)
        losses = _batch_losses(model, data, batch, device, config)
        if not all(torch.isfinite(loss).item() for loss in losses):
            raise ValueError(f"non-finite training loss at update {step + 1}")
        (losses[0] + losses[1]).backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), config["clip_grad_norm"], error_if_nonfinite=True)
        optimizer.step()
        _accumulate(state["loss_sums"], losses, len(batch))
        _accumulate(interval_sums, losses, len(batch))
        state["positions_seen"] += len(batch)
        interval_count += len(batch)
        state["completed_steps"] = step + 1
        state["data_order_sha256"] = hashlib.sha256(
            bytes.fromhex(state["data_order_sha256"]) + batch.astype("<i8").tobytes()
        ).hexdigest()
        if (step + 1) % config["checkpoint_steps"] == 0 or step + 1 == state["total_steps"]:
            elapsed = time.perf_counter() - interval_started
            state["elapsed_seconds"]["training"] += elapsed
            averages = _mean(interval_sums, interval_count)
            state["learning_curve"].append({"completed_steps": step + 1, "training_losses": averages})
            state["model"], state["optimizer"] = model.state_dict(), optimizer.state_dict()
            _save_checkpoint(out_dir / "checkpoint.pt", state)
            print(
                f"steps={step + 1}/{state['total_steps']} "
                + " ".join(f"{name}={averages[name]:.6f}" for name in LOSS_NAMES)
                + f" positions_per_second={interval_count / elapsed:.2f}",
                file=sys.stderr, flush=True,
            )
            interval_sums, interval_count = _zero_losses(), 0
            interval_started = time.perf_counter()

    model.eval()
    started = time.perf_counter()
    validation = np.flatnonzero(data.validation)
    sums = _zero_losses()
    with torch.inference_mode():
        for start in range(0, len(validation), batch_size):
            batch = validation[start:start + batch_size]
            losses = _batch_losses(model, data, batch, device, config)
            _accumulate(sums, losses, len(batch))
    validation_seconds = time.perf_counter() - started
    with _atomic_path(out_dir / "model.onnx") as temporary:
        export_onnx(copy.deepcopy(model).cpu().float().eval(), temporary)
    _write_json(out_dir / "summary.json", {
        "cycle": state["cycle"], "config": config, "provenance": state["provenance"],
        "training_count": state["training_count"], "validation_count": state["validation_count"],
        "completed_steps": state["completed_steps"], "total_steps": state["total_steps"],
        "positions_seen": state["positions_seen"],
        "training_losses": _mean(state["loss_sums"], state["positions_seen"]),
        "validation_losses": _mean(sums, len(validation)), "learning_curve": state["learning_curve"],
        "data_order_sha256": state["data_order_sha256"], "network_sha256": _sha256(out_dir / "model.onnx"),
        "elapsed_seconds": {**state["elapsed_seconds"], "validation": validation_seconds},
        "positions_per_second": state["positions_seen"] / state["elapsed_seconds"]["training"],
    })


def fixture_eval(checkpoint: Path, onnx: Path, fixture: Path, out: Path) -> None:
    """チェックポイントとONNXの一致を確かめ、固定局面集をCPUで推論する。"""
    model = _model(_load_checkpoint(checkpoint)).eval()
    onnx_bytes = onnx.read_bytes()
    with tempfile.TemporaryDirectory(prefix="minase-zero-fixture-") as directory:
        path = Path(directory) / "model.onnx"
        export_onnx(model, path)
        if path.read_bytes() != onnx_bytes:
            raise ValueError("checkpoint ONNX bytes differ from --onnx; weights are not identical")
    _write_json(out, evaluate_fixture(model, fixture, onnx_bytes))
