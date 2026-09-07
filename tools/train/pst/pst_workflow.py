"""PST学習の準備、自己対局生成、学習、診断を記録付きで実行する。"""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import fcntl
import hashlib
from importlib.metadata import distributions
import json
import math
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import time
import tomllib

from mnsd import Dataset, read_header

ROOT = Path(__file__).resolve().parents[3]
SOURCES = Path(__file__).resolve().parent
RULES = "L0,P0,R1,E0"


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path: Path, value: object) -> None:
    """既存の記録を上書きしない。"""
    with path.open("x") as stream:
        json.dump(value, stream, indent=2, ensure_ascii=False, allow_nan=False)
        stream.write("\n")


def git(repository: Path, *arguments: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(repository), *arguments], text=True
    ).strip()


def integer(value: object, name: str, minimum: int, maximum: int) -> int:
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError(f"{name} must be an integer in {minimum}..{maximum}")
    return value


def number(value: object, name: str, lower: float, upper: float) -> float:
    if type(value) not in (int, float) or not math.isfinite(value) or not lower <= value <= upper:
        raise ValueError(f"{name} must be finite and in {lower}..{upper}")
    return float(value)


def path_value(value: object, name: str, root: Path) -> Path:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{name} must be a nonempty path")
    return (root / value).resolve()


def load_config(path: Path, root: Path = ROOT) -> dict:
    """省略や綴り間違いを受理せず、設定を準備時に確定する。"""
    with path.open("rb") as stream:
        config = tomllib.load(stream)
    fields = {
        "run": {"directory", "base_commit", "data"},
        "generate": {"seeds", "games", "nodes", "concurrency", "max_ply", "hash_mb", "random_moves"},
        "train": {"learning_rate", "epochs", "batch", "seed", "lambda", "device", "validation_sample"},
        "diagnose": {"sample_size", "seed"},
    }
    if set(config) != set(fields):
        raise ValueError(f"config sections must be {sorted(fields)}")
    for section, expected in fields.items():
        if not isinstance(config[section], dict) or set(config[section]) != expected:
            raise ValueError(f"{section} fields must be {sorted(expected)}")
    run = config["run"]
    directory = path_value(run["directory"], "run.directory", root)
    data_root = (root / "data").resolve()
    if directory == data_root or not directory.is_relative_to(data_root):
        raise ValueError("run.directory must be a new directory below repository data/")
    run["directory"] = str(directory)
    if not isinstance(run["base_commit"], str) or not re.fullmatch(r"[0-9a-fA-F]{40}", run["base_commit"]):
        raise ValueError("run.base_commit must be a full 40-digit commit hash")
    if not isinstance(run["data"], list) or not run["data"]:
        raise ValueError("run.data must list existing MNSD files")
    run["data"] = [str(path_value(p, "run.data", root)) for p in run["data"]]
    if len(set(run["data"])) != len(run["data"]):
        raise ValueError("run.data contains duplicate paths")
    generation = config["generate"]
    for key, maximum in (("games", 2**32 - 1), ("nodes", 2**32 - 1),
                         ("max_ply", 65535), ("concurrency", 2**31 - 1), ("hash_mb", 2**31 - 1)):
        integer(generation[key], f"generate.{key}", 1, maximum)
    integer(generation["random_moves"], "generate.random_moves", 0, 80)
    seeds = generation["seeds"]
    if not isinstance(seeds, list) or not seeds:
        raise ValueError("generate.seeds must be a nonempty list")
    for seed in seeds:
        integer(seed, "generate.seeds", 0, 2**64 - 1 - generation["games"])
    for left, right in zip(sorted(seeds), sorted(seeds)[1:]):
        if right - left < generation["games"]:
            raise ValueError("generation seed ranges overlap")
    training = config["train"]
    for key in ("epochs", "batch", "validation_sample"):
        integer(training[key], f"train.{key}", 1, 2**31 - 1)
    integer(training["seed"], "train.seed", 0, 2**63 - 1)
    number(training["learning_rate"], "train.learning_rate", sys.float_info.min, sys.float_info.max)
    number(training["lambda"], "train.lambda", 0, 1)
    if training["device"] not in ("cpu", "cuda"):
        raise ValueError("train.device must explicitly be cpu or cuda")
    integer(config["diagnose"]["seed"], "diagnose.seed", 0, 2**63 - 1)
    integer(config["diagnose"]["sample_size"], "diagnose.sample_size", 1, 2**31 - 1)
    return config


def check_existing_data(config: dict) -> list[dict]:
    dataset = Dataset(config["run"]["data"])
    files = []
    for path, header, records in zip(config["run"]["data"], dataset.headers, dataset.records):
        if header.rule_set != RULES or len(records) == 0:
            raise ValueError(f"{path}: requires nonempty data with rules {RULES}")
        # src/rng.rs::derive_seed の base_seed + game_number に依存する。
        # 記録されていない破棄対局まで復元はできない。過去ログとの照合も必要。
        start = header.seed + 1
        end = header.seed + int(records["game"].max()) + 1
        if end > 2**64:
            raise ValueError(f"{path}: historical seed range wraps")
        for seed in config["generate"]["seeds"]:
            if max(start, seed + 1) < min(end, seed + config["generate"]["games"] + 1):
                raise ValueError(f"new generation seed overlaps {path}")
        files.append({"path": path, "sha256": digest(Path(path))})
    return files


@contextmanager
def locked(run: Path):
    with (run / ".lock").open("a") as stream:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise ValueError(f"another workflow command is running in {run}") from error
        yield


def run_command(run: Path, label: str, command: list[str], cwd: Path) -> None:
    """引数列、作業ディレクトリ、出力、所要時間、終了コードを保存する。"""
    started = time.monotonic()
    with (run / "commands.jsonl").open("a") as history:
        history.write(json.dumps({"label": label, "cwd": str(cwd), "argv": command}) + "\n")
    print(f"{label}: {' '.join(command)}", flush=True)
    with (run / f"{label}.log").open("a") as log:
        result = subprocess.run(command, cwd=cwd, stdout=log, stderr=subprocess.STDOUT)
    with (run / "commands.jsonl").open("a") as history:
        history.write(json.dumps({"label": label, "returncode": result.returncode,
                                  "seconds": time.monotonic() - started}) + "\n")
    result.check_returncode()


def source_hashes() -> dict[str, str]:
    return {p.name: digest(p) for p in sorted(SOURCES.glob("*.py")) if not p.name.startswith("test_")}


def prepare(config_path: Path) -> None:
    from train_pst import read_mnpt
    config = load_config(config_path)
    base = git(ROOT, "rev-parse", config["run"]["base_commit"] + "^{commit}")
    config["run"]["base_commit"] = base
    existing = check_existing_data(config)
    run = Path(config["run"]["directory"])
    run.mkdir()  # 新規実験だけを作り、既存結果を消さない。
    with locked(run):
        shutil.copyfile(config_path, run / "config.toml")
        write_json(run / "settings.json", config)
        generator = run / "generator"
        run_command(run, "worktree", ["git", "worktree", "add", "--detach", str(generator), base], ROOT)
        shutil.copyfile(generator / "nets/pst.bin", run / "pst-base.bin")
        read_mnpt(run / "pst-base.bin")
        run_command(run, "build", ["cargo", "build", "--release", "--locked", "--target-dir",
                    str(generator / "target"), "--bin", "selfplay_gen"], generator)
        write_json(run / "prepared.json", {
            "config": config, "existing_data": existing, "sources": source_hashes(),
            "base_sha256": digest(run / "pst-base.bin"),
            "generator_sha256": digest(generator / "target/release/selfplay_gen"),
            "repository": str(ROOT),
        })
    print(f"Prepared {run}")


def load_prepared(run: Path) -> dict:
    state = json.loads((run / "prepared.json").read_text())
    if state["repository"] != str(ROOT) or state["config"]["run"]["directory"] != str(run):
        raise ValueError("run directory belongs to a different repository or path")
    if source_hashes() != state["sources"]:
        raise ValueError("training tools changed since prepare; start a new run")
    verify_file(run / "pst-base.bin", state["base_sha256"])
    return state


def verify_file(path: Path, checksum: str) -> None:
    if digest(path) != checksum:
        raise ValueError(f"checksum changed: {path}")


def generated_path(run: Path, seed: int) -> Path:
    return run / f"generated-{seed}.bin"


def validate_generated(path: Path, state: dict, seed: int, run: Path) -> None:
    header = read_header(path)
    expected = state["config"]
    # MNPTヘッダの重み本体検査和。ファイル全体のSHA-256とは別。
    checksum = (run / "pst-base.bin").read_bytes()[48:80]
    if (header.generation_commit != expected["run"]["base_commit"]
            or header.network_checksum != checksum or header.rule_set != RULES
            or header.teacher_nodes != expected["generate"]["nodes"] or header.seed != seed
            or header.record_count == 0):
        raise ValueError(f"unexpected or empty generated data: {path}")


def generate(run: Path, selected_seed: int | None) -> None:
    state = load_prepared(run)
    config = state["config"]["generate"]
    generator = run / "generator"
    if git(generator, "rev-parse", "HEAD") != state["config"]["run"]["base_commit"] or git(generator, "status", "--porcelain"):
        raise ValueError("generator worktree changed since prepare")
    binary = generator / "target/release/selfplay_gen"
    verify_file(binary, state["generator_sha256"])
    seeds = config["seeds"] if selected_seed is None else [selected_seed]
    if any(seed not in config["seeds"] for seed in seeds):
        raise ValueError("--seed must be one of the configured generation seeds")
    for seed in seeds:
        output = generated_path(run, seed)
        receipt = run / f"generated-{seed}.json"
        if receipt.exists():
            verify_file(output, json.loads(receipt.read_text())["sha256"])
            validate_generated(output, state, seed, run)
            print(f"Already verified: {output}")
            continue
        if output.exists():
            raise ValueError(f"unverified output exists; isolate it before retrying: {output}")
        command = [str(binary), "generate", "--output", str(output), "--seed", str(seed)]
        for key in ("games", "nodes", "random_moves", "concurrency", "max_ply", "hash_mb"):
            command += ["--" + key.replace("_", "-"), str(config[key])]
        run_command(run, f"generate-{seed}", command, generator)
        run_command(run, f"inspect-{seed}", [str(binary), "inspect", str(output)], generator)
        validate_generated(output, state, seed, run)
        write_json(receipt, {"sha256": digest(output), "records": read_header(output).record_count})


def training_data(run: Path, state: dict) -> list[dict]:
    files = []
    for item in state["existing_data"]:
        verify_file(Path(item["path"]), item["sha256"])
        files.append(item)
    for seed in state["config"]["generate"]["seeds"]:
        path = generated_path(run, seed)
        receipt = json.loads((run / f"generated-{seed}.json").read_text())
        verify_file(path, receipt["sha256"])
        validate_generated(path, state, seed, run)
        files.append({"path": str(path), "sha256": receipt["sha256"]})
    return files


def train(run: Path) -> None:
    state = load_prepared(run)
    destination = run / "training"
    if destination.exists():
        raise ValueError(f"training already started; use a new run or isolate {destination} before retrying")
    import torch
    from train_pst import estimate_generation_ks, estimate_mixed_k, read_mnpt
    config = state["config"]["train"]
    if config["device"] == "cuda" and not torch.cuda.is_available():
        raise ValueError("CUDA unavailable; run training on a GPU host")
    files = training_data(run, state)
    paths = [item["path"] for item in files]
    dataset = Dataset(paths)
    training_count, validation_count = dataset.training_indices.size, dataset.validation_indices.size
    if not training_count or not validation_count:
        raise ValueError("training and validation records must both be nonempty")
    teacher_ks, _ = estimate_generation_ks(dataset)
    k = estimate_mixed_k(dataset)
    del dataset
    destination.mkdir()
    steps = (int(training_count) + config["batch"] - 1) // config["batch"]
    write_json(destination / "inputs.json", {
        "data": files,
        "teacher_ks": teacher_ks.tolist(), "k": k, "training_records": int(training_count),
        "validation_records": int(validation_count), "steps_per_epoch": steps,
        "total_steps": steps * config["epochs"], "options": config,
    })
    write_json(destination / "environment.json", {
        "python": sys.version, "platform": platform.platform(), "torch": torch.__version__,
        "cuda": torch.version.cuda, "device": config["device"],
        "device_name": torch.cuda.get_device_name(0) if config["device"] == "cuda" else platform.processor(),
        "packages": sorted(f"{d.metadata['Name']}=={d.version}" for d in distributions()),
    })
    print(f"K={k}, training={training_count}, validation={validation_count}, steps/epoch={steps}", flush=True)
    command = [sys.executable, str(SOURCES / "train_pst.py"), "train", "--data", *paths,
               "--init", str(run / "pst-base.bin"), "--output", str(destination / "pst.bin"), "--k", repr(k)]
    for key, option in (("learning_rate", "lr"), ("epochs", "epochs"), ("batch", "batch"),
                        ("seed", "seed"), ("lambda", "lambda"), ("device", "device"),
                        ("validation_sample", "validation-sample")):
        command += ["--" + option, str(config[key])]
    run_command(run, "train", command, ROOT)
    read_mnpt(destination / "pst.bin")
    write_json(destination / "complete.json", {"sha256": digest(destination / "pst.bin")})


def diagnose(run: Path) -> None:
    from pst_diagnostics import diagnose as diagnose_weights
    state = load_prepared(run)
    candidate = run / "training/pst.bin"
    verify_file(candidate, json.loads((run / "training/complete.json").read_text())["sha256"])
    paths = [item["path"] for item in training_data(run, state)]
    destination = run / "diagnostics"
    destination.mkdir()
    config = state["config"]["diagnose"]
    report = diagnose_weights(Dataset(paths), run / "pst-base.bin", candidate, destination,
                              config["sample_size"], config["seed"])
    write_json(destination / "report.json", report)
    print(f"Diagnostics: {destination / 'report.json'}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare_parser = subparsers.add_parser("prepare", help="設定を固定し、生成器をビルドする")
    prepare_parser.add_argument("--config", type=Path, required=True)
    for name in ("generate", "train", "diagnose"):
        child = subparsers.add_parser(name)
        child.add_argument("--run-dir", type=Path, required=True)
        if name == "generate":
            child.add_argument("--seed", type=int, help="設定内の1ファイルだけを生成する")
    args = parser.parse_args()
    try:
        if args.command == "prepare":
            prepare(args.config.resolve())
        else:
            run = args.run_dir.resolve()
            with locked(run):
                if args.command == "generate":
                    generate(run, args.seed)
                elif args.command == "train":
                    train(run)
                else:
                    diagnose(run)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"error: {error}\n")


if __name__ == "__main__":
    main()
