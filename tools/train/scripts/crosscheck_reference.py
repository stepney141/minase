#!/usr/bin/env python3
"""RustのチェックポイントからPyTorchで3更新し、損失・全勾配・重みを照合する。

PYTHONPATH=tools/train/src python tools/train/scripts/crosscheck_reference.py \
    --data /path/to/generated.bin --dump /tmp/nnue-reference/reference.json

CPU専用。reference.jsonの係数とλを適用し、同じ先読み条件で教師用Kも照合する。
チェックポイントの配列順はcrates/nnue-trainer/README.mdを参照。
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys
from types import SimpleNamespace

import numpy as np
import torch
from torch.nn import functional as F

from minase_train.data.features import FEATURE_COUNT, canonical_feature_indices
from minase_train.data.mnpt import initial_weights
from minase_train.data.mnsd import Dataset
from minase_train.data.taper import piece_counts
from minase_train.nnue.feature_sets import get_feature_set
from minase_train.nnue.model import NNUE
from minase_train.nnue.train import _batch_inputs, make_optimizer
from minase_train.pst.removal import make_removal_reference, removal_loss
from minase_train.pst.teacher import build_targets, estimate_generation_ks


NAMES = ("linear.weight", "ft.embedding.weight", "ft.virtual.weight",
         "head.biases", "head.output_weights", "head.output_bias")
RTOL, ATOL = 1e-4, 1e-5


class Reader:
    def __init__(self, raw: bytes):
        self.raw, self.offset = raw, 0

    def take(self, count: int) -> bytes:
        end = self.offset + count
        if end > len(self.raw):
            raise ValueError(f"truncated checkpoint at byte {self.offset}")
        result = self.raw[self.offset:end]
        self.offset = end
        return result

    def unpack(self, fmt: str):
        return struct.unpack("<" + fmt, self.take(struct.calcsize("<" + fmt)))

    def array(self, shape: tuple[int, ...]) -> np.ndarray:
        return np.frombuffer(self.take(4 * int(np.prod(shape))), dtype="<f4").reshape(shape).copy()


def checkpoint(path: Path) -> dict:
    raw = path.read_bytes()
    if len(raw) < 32 or hashlib.sha256(raw[:-32]).digest() != raw[-32:]:
        raise ValueError(f"{path}: invalid checkpoint SHA-256")
    r = Reader(raw[:-32])
    if r.take(8) != b"MNTRCKPT" or r.unpack("I") != (2,):
        raise ValueError(f"{path}: unsupported checkpoint format/version")
    architecture = r.take(96)
    width, k = r.unpack("If")
    if not 2 <= width <= 512 or width % 2 or not np.isfinite(k) or not 0 < k <= 100000:
        raise ValueError(f"{path}: invalid width or K")
    sets = [name for name in ("P", "HalfKP-hm-b8")
            if architecture == f"features={name};ft={width};linear=tapered2;head=direct-screlu".encode().ljust(96, b"\0")]
    if len(sets) != 1:
        raise ValueError(f"{path}: invalid architecture")
    feature_set = sets[0]
    units = width // 2 if feature_set == "P" else width
    rows = 6841 if feature_set == "P" else 109441
    virtual_rows = 0 if feature_set == "P" else 13681
    step, epoch, best_epoch = r.unpack("QQQ")
    best_loss, elapsed, seed = r.unpack("ddQ")
    config_hash, input_hash = r.take(32), r.take(32)
    groups = [r.unpack("dd") for _ in range(3)]
    removal_penalty, lambda_override = r.unpack("dd")
    shapes = ((6841, 2), (rows, width), (virtual_rows, width),
              (units,), (2, units), ())
    tensors = [{name: r.array(shape) for name, shape in zip(NAMES, shapes)} for _ in range(3)]
    masks = [np.frombuffer(r.take(count), dtype=np.uint8).copy()
             for count in (6841, rows, virtual_rows)]
    if any(np.any(mask > 1) or (mask.size and mask[-1] != 0) for mask in masks):
        raise ValueError(f"{path}: invalid observation masks")
    fixed_linear, fixed_virtual = r.array((6841, 2)), r.array((virtual_rows, width))
    if r.offset != len(r.raw):
        raise ValueError(f"{path}: trailing checkpoint bytes")
    for params in tensors:
        if any(not np.isfinite(value).all() for value in params.values()):
            raise ValueError(f"{path}: non-finite checkpoint tensor")
    if any(np.any(v < 0) for v in tensors[2].values()):
        raise ValueError(f"{path}: negative AdamW variance")
    return dict(feature_set=feature_set, width=width, k=k, step=step, epoch=epoch,
                best_epoch=best_epoch, best_loss=best_loss, elapsed=elapsed, seed=seed,
                config_hash=config_hash, input_hash=input_hash, groups=groups,
                removal_penalty=removal_penalty, lambda_override=lambda_override,
                shapes=shapes, parameters=tensors[0], m=tensors[1], v=tensors[2],
                masks=[m.astype(bool) for m in masks], fixed_linear=fixed_linear,
                fixed_virtual=fixed_virtual)


def compare(name: str, rust: object, python: object, *, exact: bool = False) -> bool:
    rust, python = np.asarray(rust), np.asarray(python)
    if rust.shape != python.shape:
        print(f"{name}: Rust shape={rust.shape}, Python shape={python.shape}", file=sys.stderr)
        return False
    equal = rust == python if exact else np.isclose(rust, python, rtol=RTOL, atol=ATOL)
    if np.all(equal):
        return True
    location = tuple(int(v) for v in np.argwhere(~equal)[0])
    difference = abs(float(rust[location]) - float(python[location]))
    print(f"{name}{location}: Rust={rust[location]}, Python={python[location]}, "
          f"absolute difference={difference:.9g}", file=sys.stderr)
    return False


def restore(saved: dict, training: dict) -> tuple[NNUE, torch.optim.AdamW]:
    spec = get_feature_set(saved["feature_set"])
    canonical = canonical_feature_indices(np.arange(FEATURE_COUNT))
    initial = saved["fixed_linear"][canonical]
    observed = saved["masks"][0][canonical]
    feature_observed = saved["masks"][1][:-1] if spec.factorized else observed
    model = NNUE(initial, observed, saved["width"], saved["k"], torch.device("cpu"),
                 saved["seed"], spec, feature_observed)
    with torch.no_grad():
        for name, parameter in model.named_parameters():
            parameter.copy_(torch.from_numpy(saved["parameters"][name]))
        model.linear.initial.copy_(torch.from_numpy(saved["fixed_linear"]))
        if spec.factorized:
            model.ft.initial_virtual.copy_(torch.from_numpy(saved["fixed_virtual"]))
            if not np.array_equal(model.ft.virtual_observed.numpy(), saved["masks"][2]):
                raise ValueError("virtual observation masks disagree")
    expected_groups = [(training[f"lr_{name}"], training[f"weight_decay_{name}"])
                       for name in ("linear", "ft", "head")]
    if saved["groups"] != expected_groups or any(
            saved[name] != training[name] for name in ("removal_penalty", "lambda_override")):
        raise ValueError("reference coefficients disagree with checkpoint")
    for lr, decay in expected_groups:
        if not np.isfinite(lr) or lr <= 0 or not np.isfinite(decay) or decay < 0:
            raise ValueError("invalid optimizer coefficients")
    if not np.isfinite(training["removal_penalty"]) or training["removal_penalty"] < 0:
        raise ValueError("invalid removal penalty")
    args = SimpleNamespace(**training)
    optimizer = make_optimizer(model, args)
    for name, parameter in model.named_parameters():
        optimizer.state[parameter] = {
            "step": torch.tensor(float(saved["step"])),
            "exp_avg": torch.from_numpy(saved["m"][name]).clone(),
            "exp_avg_sq": torch.from_numpy(saved["v"][name]).clone(),
        }
    return model, optimizer


def check_batch(batch: dict, dataset: Dataset, records: np.ndarray, spec) -> bool:
    valid = True
    sentinel = batch["sentinel"]
    bases, inputs = [], []
    for side in (records["stm"], 1 - records["stm"]):
        base, feature = spec.indices(records["board"], records["lion"], side)
        bases.append(base)
        if spec.name == "P":
            packed = canonical_feature_indices(feature).astype(np.uint32) * 2 + (feature % 12 >= 6)
            packed[feature == FEATURE_COUNT] = sentinel
        else:
            packed = feature.astype(np.uint32)
            packed[feature == spec.feature_count] = sentinel
        inputs.append(packed)
    valid &= compare("input_features", batch["input_features"], np.stack(inputs, axis=1), exact=True)
    linear = canonical_feature_indices(bases[0]).astype(np.uint32)
    linear[bases[0] == FEATURE_COUNT] = sentinel
    valid &= compare("linear_features", batch["linear_features"], linear, exact=True)
    valid &= compare("piece_counts", batch["piece_counts"], piece_counts(records["board"]), exact=True)
    indices = np.asarray(batch["record_indices"], dtype=np.int64)
    valid &= compare("teacher_scores", batch["teacher_scores"], dataset.teacher_scores(indices))
    valid &= compare("teacher_classes", batch["teacher_classes"], dataset.generations(indices), exact=True)
    return bool(valid)


def crosscheck(paths: list[Path], manifest_path: Path, require_auxiliary: bool = False) -> bool:
    torch.set_num_threads(1)
    torch.use_deterministic_algorithms(True)
    manifest = json.loads(manifest_path.read_text())
    if manifest["format"] != "minase-nnue-reference" or manifest["version"] != 2 or len(manifest["steps"]) != 3:
        raise ValueError("unsupported reference dump format or step count")
    if require_auxiliary and not any(step["losses"]["auxiliary"] > 0 for step in manifest["steps"]):
        raise ValueError("--require-auxiliary requires a positive auxiliary loss in at least one of the 3 recorded updates")
    directory = manifest_path.parent
    saved = checkpoint(directory / manifest["checkpoint"])
    if saved["step"] == 0 or not np.any(saved["parameters"]["head.output_weights"]):
        raise ValueError("comparison requires nonzero output weights and AdamW state; use --steps 3 or more")
    model, optimizer = restore(saved, manifest["training"])
    batches = [json.loads((directory / step["batch"]).read_text()) for step in manifest["steps"]]
    for batch in batches:
        if batch["format"] != "minase-nnue-batch-indices" or batch["version"] != 1:
            raise ValueError("unsupported batch format/version")
        if [p.resolve() for p in paths] != [Path(p).resolve() for p in batch["data"]]:
            raise ValueError("--data must match the recorded file order")
        if batch["feature_set"] != saved["feature_set"] or batch["lookahead"] != batches[0]["lookahead"]:
            raise ValueError("inconsistent batch feature set or lookahead")
    dataset = Dataset(paths, lookahead=batches[0]["lookahead"], lambda_override=manifest["training"]["lambda_override"])
    teacher_ks, _ = estimate_generation_ks(dataset, indices=dataset.training_indices)
    recorded_ks = np.asarray(manifest["teacher_ks"], dtype=np.float64)
    valid = compare("teacher_lambdas", manifest["teacher_lambdas"], dataset.teacher_lambdas, exact=True)
    active = dataset.teacher_lambdas != 0
    if recorded_ks.shape != teacher_ks.shape:
        raise ValueError("teacher K count differs from dataset")
    valid &= compare("teacher_ks", recorded_ks[active], teacher_ks[active])
    if not np.all(np.isnan(recorded_ks[~active])):
        raise ValueError("lambda=0 requires a null teacher K")
    v0 = initial_weights()
    reference = make_removal_reference(v0, v0, torch.device("cpu"))
    for step_number, (step, batch) in enumerate(zip(manifest["steps"], batches), start=1):
        indices = np.asarray(batch["record_indices"], dtype=np.int64)
        records = dataset.gather(indices)
        valid &= check_batch(batch, dataset, records, model.spec)
        own, opposite, ratios = _batch_inputs(records, torch.device("cpu"), model.spec)
        targets = build_targets(records, teacher_ks, dataset.generations(indices), dataset.teacher_lambdas,
                                scores=dataset.teacher_scores(indices))
        optimizer.zero_grad(set_to_none=True)
        bce = F.binary_cross_entropy_with_logits(model(own, opposite, ratios), torch.from_numpy(targets))
        auxiliary = manifest["training"]["removal_penalty"] * removal_loss(model.removal_deltas(own, opposite, torch.from_numpy(piece_counts(records["board"]))),
                                        own[0], reference, torch.from_numpy(piece_counts(records["board"])), model.k)
        (bce + auxiliary).backward()
        valid &= compare(f"step {step_number} BCE", step["losses"]["bce"], float(bce.detach()))
        valid &= compare(f"step {step_number} auxiliary", step["losses"]["auxiliary"], float(auxiliary.detach()))
        reader = Reader((directory / step["gradients"]).read_bytes())
        gradients = {name: reader.array(shape) for name, shape in zip(NAMES, saved["shapes"])}
        if reader.offset != len(reader.raw):
            raise ValueError("trailing gradient bytes")
        for name, parameter in model.named_parameters():
            valid &= compare(f"step {step_number} gradient {name}", gradients[name], parameter.grad.numpy())
        optimizer.step()
        model.constrain()
        after = checkpoint(directory / step["after"])
        if after["step"] != saved["step"] + step_number or after["config_hash"] != saved["config_hash"] or after["input_hash"] != saved["input_hash"]:
            raise ValueError("after checkpoint step or hashes disagree")
        for name, parameter in model.named_parameters():
            valid &= compare(f"step {step_number} weight {name}", after["parameters"][name], parameter.detach().numpy())
            for field, key in (("m", "exp_avg"), ("v", "exp_avg_sq")):
                valid &= compare(f"step {step_number} {field} {name}", after[field][name], optimizer.state[parameter][key].numpy())
        print(f"step {step_number}: BCE={float(bce.detach()):.9g}, auxiliary={float(auxiliary.detach()):.9g}")
    if valid:
        print(f"一致: {saved['feature_set']}, 幅{saved['width']}, 3更新, rtol={RTOL}, atol={ATOL}")
    return bool(valid)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", nargs="+", required=True, type=Path)
    parser.add_argument("--dump", required=True, type=Path, help="dump-batchが書いたreference.json")
    parser.add_argument("--require-auxiliary", action="store_true",
                        help="reference.jsonの3更新のうち少なくとも1回で補助損失が正であることを要求する")
    args = parser.parse_args()
    try:
        return 0 if crosscheck(args.data, args.dump, args.require_auxiliary) else 1
    except (OSError, ValueError, KeyError, IndexError, TypeError, RuntimeError, struct.error) as error:
        print(f"crosscheck_reference: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
