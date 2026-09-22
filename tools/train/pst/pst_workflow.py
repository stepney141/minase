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

from lookahead import validate_lookahead
from mnsd import Dataset, read_header, provenance_path, validate_lambda_override

ROOT = Path(__file__).resolve().parents[3]
SOURCES = Path(__file__).resolve().parent
RULES = "L0,P0,R1,E0"
# MNPT本体末尾の探索用駒価値(47個のi32)。
PIECE_VALUE_BYTES = 47 * 4


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
        "train": {"model", "k", "learning_rate", "epochs", "batch", "seed",
                  "removal_penalty", "device", "validation_sample"},
        "diagnose": {"sample_size", "seed"},
    }
    if set(config) != set(fields):
        raise ValueError(f"config sections must be {sorted(fields)}")
    extra_fields = {"king_features", "extra_columns", "train_extra", "freeze_pst"}
    if isinstance(config["train"], dict) and extra_fields & set(config["train"]):
        fields["train"] |= extra_fields
    if isinstance(config["train"], dict) and "rescore" in config["train"]:
        fields["train"].add("rescore")
    if isinstance(config["train"], dict) and "lookahead" in config["train"]:
        fields["train"].add("lookahead")
    if isinstance(config["train"], dict) and "lambda_override" in config["train"]:
        fields["train"].add("lambda_override")
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
    if not isinstance(seeds, list):
        raise ValueError("generate.seeds must be a list; [] trains on existing data only")
    for seed in seeds:
        integer(seed, "generate.seeds", 0, 2**64 - 1 - generation["games"])
    for left, right in zip(sorted(seeds), sorted(seeds)[1:]):
        if right - left < generation["games"]:
            raise ValueError("generation seed ranges overlap")
    training = config["train"]
    validate_lambda_override(training.get("lambda_override"))
    if "rescore" in training:
        if not isinstance(training["rescore"], list) or len(training["rescore"]) != len(run["data"]) + len(seeds):
            raise ValueError("train.rescore must match all existing and generated MNSD files")
        training["rescore"] = ["-" if p == "-" else str(path_value(p, "train.rescore", root))
                              for p in training["rescore"]]
    else:
        training["rescore"] = ["-"] * (len(run["data"]) + len(seeds))
    if "lookahead" in training:
        lookahead = training["lookahead"]
        if not isinstance(lookahead, dict) or set(lookahead) != {"gamma", "plies"}:
            raise ValueError("train.lookahead requires exactly gamma and plies")
        validate_lookahead(**lookahead)
        if any(path != "-" for path in training["rescore"]):
            raise ValueError("lookahead and rescore cannot be combined")
    for key in ("epochs", "batch", "validation_sample"):
        integer(training[key], f"train.{key}", 1, 2**31 - 1)
    integer(training["seed"], "train.seed", 0, 2**63 - 1)
    number(training["k"], "train.k", sys.float_info.min, sys.float_info.max)
    number(training["learning_rate"], "train.learning_rate", sys.float_info.min, sys.float_info.max)
    number(training["removal_penalty"], "train.removal_penalty", 0, sys.float_info.max)
    if training["device"] not in ("cpu", "cuda"):
        raise ValueError("train.device must explicitly be cpu or cuda")
    if training["model"] not in ("single", "tapered", "mirrored"):
        raise ValueError("train.model must explicitly be single, tapered, or mirrored")
    if training["removal_penalty"] > 0 and training["model"] != "mirrored":
        raise ValueError("positive train.removal_penalty requires train.model = mirrored")
    if "king_features" in training:
        from train_pst import parse_ranges
        if type(training["freeze_pst"]) is not bool:
            raise ValueError("train.freeze_pst must be boolean")
        if not isinstance(training["king_features"], list):
            raise ValueError("train.king_features must be a list")
        if not all(isinstance(training[key], str) for key in ("extra_columns", "train_extra")):
            raise ValueError("extra column ranges must be strings")
        if training["king_features"]:
            if len(training["king_features"]) != len(run["data"]) + len(seeds):
                raise ValueError("train.king_features must match all existing and generated MNSD files")
            training["king_features"] = [str(path_value(p, "train.king_features", root))
                                         for p in training["king_features"]]
            columns = parse_ranges(training["extra_columns"], None)
            parse_ranges(training["train_extra"], len(columns))
        elif training["extra_columns"] or training["train_extra"] or training["freeze_pst"]:
            raise ValueError("extra training options require train.king_features")
        if training["freeze_pst"] and training["removal_penalty"] != 0:
            raise ValueError("train.freeze_pst requires train.removal_penalty = 0")
    integer(config["diagnose"]["seed"], "diagnose.seed", 0, 2**63 - 1)
    integer(config["diagnose"]["sample_size"], "diagnose.sample_size", 1, 2**31 - 1)
    return config


def check_existing_data(config: dict) -> list[dict]:
    dataset = Dataset(config["run"]["data"],
                      lambda_override=config["train"].get("lambda_override"),
                      lookahead=config["train"].get("lookahead"),
                      rescore=config["train"]["rescore"][:len(config["run"]["data"])])
    files = []
    for path, header, records, provenance in zip(config["run"]["data"], dataset.headers, dataset.records, dataset.provenance):
        if header.rule_set != RULES or len(records) == 0:
            raise ValueError(f"{path}: requires nonempty data with rules {RULES}")
        if provenance[0].result_origin == "selfplay":
            # src/rng.rs::derive_seed の base_seed + game_number に依存する。
            # 記録されていない破棄対局まで復元はできない。過去ログとの照合も必要。
            start = header.seed + 1
            end = header.seed + int(records["game"].max()) + 1
            if end > 2**64:
                raise ValueError(f"{path}: historical seed range wraps")
            for seed in config["generate"]["seeds"]:
                if max(start, seed + 1) < min(end, seed + config["generate"]["games"] + 1):
                    raise ValueError(f"new generation seed overlaps {path}")
        files.append(input_receipt(Path(path)))
    return files


def input_receipt(path: Path) -> dict:
    provenance = provenance_path(path)
    return {"path": str(path), "sha256": digest(path),
            "provenance": {"path": str(provenance), "sha256": digest(provenance)}}


def verify_input(item: dict) -> None:
    verify_file(Path(item["path"]), item["sha256"])
    verify_file(Path(item["provenance"]["path"]), item["provenance"]["sha256"])


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
    probe_commit = git(ROOT, "rev-parse", "HEAD")
    config["run"]["base_commit"] = base
    existing = check_existing_data(config)
    rescores = [{"path": path, "sha256": digest(Path(path))}
                for path in config["train"]["rescore"] if path != "-"]
    run = Path(config["run"]["directory"])
    run.mkdir()  # 新規実験だけを作り、既存結果を消さない。
    with locked(run):
        shutil.copyfile(config_path, run / "config.toml")
        write_json(run / "settings.json", config)
        generator = run / "generator"
        run_command(run, "worktree", ["git", "worktree", "add", "--detach", str(generator), base], ROOT)
        shutil.copyfile(generator / "nets/pst.bin", run / "pst-base.bin")
        read_mnpt(run / "pst-base.bin", feature_count=None)
        run_command(run, "build", ["cargo", "build", "--release", "--locked", "--target-dir",
                    str(generator / "target"), "--bin", "selfplay_gen"], generator)
        probe = run / "probe"
        run_command(run, "probe-worktree", ["git", "worktree", "add", "--detach", str(probe), probe_commit], ROOT)
        run_command(run, "probe-build", ["cargo", "build", "--release", "--locked", "--target-dir",
                    str(probe / "target"), "--bin", "pst_probe"], probe)
        write_json(run / "prepared.json", {
            "config": config, "lambda_override": config["train"].get("lambda_override"),
            "lookahead": config["train"].get("lookahead"), "existing_data": existing,
            "rescores": rescores, "sources": source_hashes(),
            "base_sha256": digest(run / "pst-base.bin"),
            "base_piece_values_sha256": hashlib.sha256(
                (run / "pst-base.bin").read_bytes()[-PIECE_VALUE_BYTES:]).hexdigest(),
            "generator_sha256": digest(generator / "target/release/selfplay_gen"),
            "probe_commit": probe_commit,
            "probe_sha256": digest(probe / "target/release/pst_probe"),
            "repository": str(ROOT),
        })
    print(f"Prepared {run}")


def load_prepared(run: Path) -> dict:
    state = json.loads((run / "prepared.json").read_text())
    if state["repository"] != str(ROOT) or state["config"]["run"]["directory"] != str(run):
        raise ValueError("run directory belongs to a different repository or path")
    validate_lambda_override(state["lambda_override"])
    if state["lambda_override"] != state["config"]["train"].get("lambda_override"):
        raise ValueError("lambda_override changed since prepare")
    if state["lookahead"] != state["config"]["train"].get("lookahead"):
        raise ValueError("lookahead changed since prepare")
    if source_hashes() != state["sources"]:
        raise ValueError("training tools changed since prepare; start a new run")
    verify_file(run / "pst-base.bin", state["base_sha256"])
    for item in state["existing_data"]:
        verify_input(item)
    for item in state["rescores"]:
        verify_file(Path(item["path"]), item["sha256"])
    for seed in state["config"]["generate"]["seeds"]:
        receipt = run / f"generated-{seed}.json"
        if receipt.exists():
            verify_input(json.loads(receipt.read_text()))
    inputs_path = run / "training/inputs.json"
    if inputs_path.exists():
        inputs = json.loads(inputs_path.read_text())
        if (inputs["lambda_override"] != state["lambda_override"]
                or inputs["options"].get("lambda_override") != state["lambda_override"]):
            raise ValueError("lambda_override changed since training")
        if (inputs["lookahead"] != state["lookahead"]
                or inputs["options"].get("lookahead") != state["lookahead"]):
            raise ValueError("lookahead changed since training")
        dataset = training_dataset([item["path"] for item in training_data(run, state)], state["config"]["train"])
        if inputs["teacher_classes"] != dataset.class_metadata():
            raise ValueError("teacher classes changed since training")
        trained_path = run / "training/pst.training.json"
        if trained_path.exists():
            trained = json.loads(trained_path.read_text())
            if trained["lambda_override"] != state["lambda_override"]:
                raise ValueError("lambda_override differs in training result")
            if trained["lookahead"] != state["lookahead"] or trained["teacher_classes"] != inputs["teacher_classes"]:
                raise ValueError("lookahead or teacher classes differ in training result")
        report_path = run / "diagnostics/report.json"
        if report_path.exists():
            report = json.loads(report_path.read_text())
            if (report["lambda_override"] != state["lambda_override"]
                    or report["teacher_classes"] != inputs["teacher_classes"]):
                raise ValueError("lambda_override or teacher classes differ in diagnostics")
    if inputs_path.parent.exists() and state["config"]["train"].get("king_features"):
        inputs = json.loads(inputs_path.read_text())
        config = state["config"]["train"]
        if [item["path"] for item in inputs["king_features"]] != config["king_features"]:
            raise ValueError("MNKF inputs changed since training")
        for item in inputs["king_features"]:
            verify_file(Path(item["path"]), item["sha256"])
        if (inputs["mnkf_definition_id"] != dataset.king_features.definition_id
                or inputs["mnkf_column_count"] != dataset.king_features.column_count):
            raise ValueError("MNKF definition ID or column count changed since training")
    return state


def verify_file(path: Path, checksum: str) -> None:
    if digest(path) != checksum:
        raise ValueError(f"checksum changed: {path}")


def generated_path(run: Path, seed: int) -> Path:
    return run / f"generated-{seed}.bin"


def validate_generated(path: Path, state: dict, seed: int, run: Path) -> None:
    Dataset([path])
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
            verify_input(json.loads(receipt.read_text()))
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
        write_json(receipt, {**input_receipt(output), "records": read_header(output).record_count})


def training_data(run: Path, state: dict) -> list[dict]:
    files = []
    for item in state["existing_data"]:
        verify_input(item)
        files.append(item)
    for seed in state["config"]["generate"]["seeds"]:
        path = generated_path(run, seed)
        receipt = json.loads((run / f"generated-{seed}.json").read_text())
        verify_input(receipt)
        validate_generated(path, state, seed, run)
        files.append(receipt)
    return files


def training_dataset(paths: list[str], config: dict) -> Dataset:
    """追加特徴を使う設定では、学習と診断に同じ列の対応を与える。"""
    if "king_features" in config and config["king_features"]:
        from train_pst import parse_ranges
        return Dataset(paths, lambda_override=config.get("lambda_override"),
                       lookahead=config.get("lookahead"), rescore=config["rescore"],
                       king_features=config["king_features"],
                       extra_columns=parse_ranges(config["extra_columns"], None))
    return Dataset(paths, lambda_override=config.get("lambda_override"),
                   lookahead=config.get("lookahead"), rescore=config["rescore"])


def train(run: Path) -> None:
    state = load_prepared(run)
    destination = run / "training"
    if destination.exists():
        raise ValueError(f"training already started; use a new run or isolate {destination} before retrying")
    import torch
    from train_pst import estimate_generation_ks, estimate_mixed_k, float_weights_path, read_mnpt
    config = state["config"]["train"]
    if config["device"] == "cuda" and not torch.cuda.is_available():
        raise ValueError("CUDA unavailable; run training on a GPU host")
    files = training_data(run, state)
    paths = [item["path"] for item in files]
    dataset = training_dataset(paths, config)
    king_inputs = ([] if dataset.king_features is None else
                   [{"path": p, "sha256": digest(Path(p))} for p in config["king_features"]])
    training_count, validation_count = dataset.training_indices.size, dataset.validation_indices.size
    if not training_count or not validation_count:
        raise ValueError("training and validation records must both be nonempty")
    teacher_ks, _ = estimate_generation_ks(dataset, indices=dataset.training_indices)
    mixed_k = estimate_mixed_k(dataset, indices=dataset.training_indices)
    k = config["k"]
    classes = dataset.class_metadata()
    exclusions = dataset.exclusions
    mnkf_definition_id = None if dataset.king_features is None else dataset.king_features.definition_id
    mnkf_column_count = None if dataset.king_features is None else dataset.king_features.column_count
    del dataset
    destination.mkdir()
    steps = (int(training_count) + config["batch"] - 1) // config["batch"]
    write_json(destination / "inputs.json", {
        "data": files, "king_features": king_inputs,
        "lambda_override": config.get("lambda_override"), "lookahead": config.get("lookahead"),
        "mnkf_definition_id": mnkf_definition_id, "mnkf_column_count": mnkf_column_count,
        "teacher_ks": [float(k) if math.isfinite(k) else None for k in teacher_ks],
        "teacher_classes": classes, "rescore_exclusions": exclusions, "rescores": state["rescores"], "k": k, "mixed_k": mixed_k,
        "training_records": int(training_count),
        "validation_records": int(validation_count), "steps_per_epoch": steps,
        "total_steps": steps * config["epochs"], "options": config,
    })
    write_json(destination / "environment.json", {
        "python": sys.version, "platform": platform.platform(), "torch": torch.__version__,
        "cuda": torch.version.cuda, "device": config["device"],
        "device_name": torch.cuda.get_device_name(0) if config["device"] == "cuda" else platform.processor(),
        "packages": sorted(f"{d.metadata['Name']}=={d.version}" for d in distributions()),
    })
    print(f"K={k}, mixed K={mixed_k} (reference), training={training_count}, "
          f"validation={validation_count}, steps/epoch={steps}", flush=True)
    command = [sys.executable, str(SOURCES / "train_pst.py"), "train", "--data", *paths,
               "--init", str(run / "pst-base.bin"), "--output", str(destination / "pst.bin"), "--k", repr(k)]
    for key, option in (("model", "model"), ("learning_rate", "lr"), ("epochs", "epochs"), ("batch", "batch"),
                        ("seed", "seed"),
                        ("removal_penalty", "removal-penalty"), ("device", "device"),
                        ("validation_sample", "validation-sample")):
        command += ["--" + option, str(config[key])]
    command += ["--rescore", *config["rescore"]]
    if config.get("lambda_override") is not None:
        command += ["--lambda-override", str(config["lambda_override"])]
    if config.get("lookahead") is not None:
        command += ["--lookahead-gamma", str(config["lookahead"]["gamma"]),
                    "--lookahead-plies", str(config["lookahead"]["plies"])]
    if king_inputs:
        command += ["--king-features", *config["king_features"],
                    "--extra-columns", config["extra_columns"], "--train-extra", config["train_extra"]]
        if config["freeze_pst"]:
            command.append("--freeze-pst")
    run_command(run, "train", command, ROOT)
    read_mnpt(destination / "pst.bin", feature_count=None)
    if (destination / "pst.bin").read_bytes()[-PIECE_VALUE_BYTES:] != (run / "pst-base.bin").read_bytes()[-PIECE_VALUE_BYTES:]:
        raise ValueError("trained weights changed the fixed piece values")
    write_json(destination / "complete.json", {
        "sha256": digest(destination / "pst.bin"),
        "float_sha256": digest(float_weights_path(destination / "pst.bin")),
    })


def diagnose_probe(binary: Path):
    """診断で使うRustの探査関数を返す。テストではPythonの参照実装へ差し替える。"""
    from pst_diagnostics import rust_probe
    return rust_probe(binary)


def diagnose(run: Path) -> None:
    from pst_diagnostics import diagnose as diagnose_weights
    from train_pst import float_weights_path
    state = load_prepared(run)
    candidate = run / "training/pst.bin"
    completion = json.loads((run / "training/complete.json").read_text())
    verify_file(candidate, completion["sha256"])
    verify_file(float_weights_path(candidate), completion["float_sha256"])
    probe = run / "probe"
    if git(probe, "rev-parse", "HEAD") != state["probe_commit"] or git(probe, "status", "--porcelain"):
        raise ValueError("probe worktree changed since prepare")
    binary = probe / "target/release/pst_probe"
    verify_file(binary, state["probe_sha256"])
    paths = [item["path"] for item in training_data(run, state)]
    destination = run / "diagnostics"
    destination.mkdir()
    config = state["config"]["diagnose"]
    training_config = state["config"]["train"]
    report = diagnose_weights(training_dataset(paths, training_config), run / "pst-base.bin", candidate, float_weights_path(candidate),
                              destination, config["sample_size"], config["seed"],
                              diagnose_probe(binary))
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
