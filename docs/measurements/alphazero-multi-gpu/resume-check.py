"""途中で止めない学習と、強制終了して再開した学習を、空のコンパイルのキャッシュで比べる。

使い方（作業ディレクトリは学習器のtools/train/zero）:
    uv run --frozen python <このファイル> --out <新しいディレクトリ> --commit <40桁> --init <checkpoint.pt> \
        --data-list <JSON配列のファイル> --nproc <プロセス数> --replay-threads <スレッド数> \
        --checkpoint-steps <保存の間隔> --kill-after <この更新回数以上の保存で止める>

3つの学習のプロセス（途中で止めない学習、強制終了する学習、再開）は、それぞれ新しいTORCHINDUCTOR_CACHE_DIRとTRITON_CACHE_DIRを使う。
全体のバッチ1,024、ミクロバッチ256、学習率0.04、bf16で、--data-listの全ファイルをリプレイバッファと最新のサイクルの両方に使う。
強制終了では、torchrunとその子孫のプロセス群のすべてへSIGKILLを送る。torchrunはワーカーを別のセッションで起動するからである。
比べるのは、経過時間と最大常駐メモリを除くチェックポイントの全項目、経過時間と速さと最大常駐メモリを除くsummary.jsonの全項目、およびmodel.onnxのバイト列である。
結果は<out>/result.jsonに書く。
"""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import time

import torch

parser = argparse.ArgumentParser()
parser.add_argument("--out", type=Path, required=True)
parser.add_argument("--commit", required=True)
parser.add_argument("--init", type=Path, required=True)
parser.add_argument("--data-list", type=Path, required=True)
for name in ("nproc", "replay-threads", "checkpoint-steps", "kill-after"):
    parser.add_argument(f"--{name}", type=int, required=True)
args = parser.parse_args()
files = json.loads(args.data_list.read_text())
zero = Path.cwd()


def environment(name: str) -> dict:
    caches = args.out / f"cache-{name}"
    (caches / "inductor").mkdir(parents=True)
    (caches / "triton").mkdir(parents=True)
    return {**os.environ, "TORCHINDUCTOR_CACHE_DIR": str(caches / "inductor"),
            "TRITON_CACHE_DIR": str(caches / "triton")}


def trainer(arguments: list[str]) -> list[str]:
    return ["uv", "run", "--frozen", "torchrun", "--standalone", "--nproc-per-node", str(args.nproc),
            "-m", "minase_zero", "train", *arguments,
            "--device", "cuda", "--replay-threads", str(args.replay_threads)]


def new_cycle(out: Path) -> list[str]:
    return trainer([
        "--init", str(args.init), "--data-list", str(args.data_list), "--latest-data", *files,
        "--out-dir", str(out), "--cycle", "1", "--seed", "20261101", "--batch-size", "1024",
        "--micro-batch-size", "256", "--lr", "0.04", "--momentum", "0.9", "--weight-decay", "0.0001",
        "--clip-grad-norm", "10", "--value-lambda", "0.333", "--precision", "bf16",
        "--checkpoint-steps", str(args.checkpoint_steps), "--trainer-commit", args.commit,
    ])


def descendants(pid: int) -> list[int]:
    children: dict[int, list[int]] = {}
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit():
            try:
                parent = int((entry / "stat").read_text().rsplit(")", 1)[1].split()[1])
            except (FileNotFoundError, ProcessLookupError):
                continue
            children.setdefault(parent, []).append(int(entry.name))
    found, stack = [], [pid]
    while stack:
        for child in children.get(stack.pop(), []):
            found.append(child)
            stack.append(child)
    return found


def alive(pid: int) -> bool:
    try:
        state = (Path("/proc") / str(pid) / "stat").read_text().rsplit(")", 1)[1].split()[0]
    except (FileNotFoundError, ProcessLookupError):
        return False
    return state not in ("Z", "X")


def run(name: str, command: list[str]) -> float:
    started = time.monotonic()
    with (args.out / f"{name}.log").open("w") as log:
        subprocess.run(command, cwd=zero, env=environment(name), stdout=log, stderr=log, check=True)
    return time.monotonic() - started


def same(left, right) -> bool:
    if isinstance(left, torch.Tensor):
        return isinstance(right, torch.Tensor) and left.dtype == right.dtype and torch.equal(left, right)
    if isinstance(left, dict):
        return isinstance(right, dict) and left.keys() == right.keys() and all(same(left[k], right[k]) for k in left)
    if isinstance(left, list):
        return isinstance(right, list) and len(left) == len(right) and all(same(x, y) for x, y in zip(left, right))
    return left == right


args.out.mkdir(parents=True)
a, b = args.out / "uninterrupted", args.out / "interrupted"
seconds = {"uninterrupted": run("uninterrupted", new_cycle(a))}
started = time.monotonic()
with (args.out / "interrupted.log").open("w") as log:
    process = subprocess.Popen(new_cycle(b), cwd=zero, env=environment("interrupted"),
                               stdout=log, stderr=log, start_new_session=True)
    while True:
        if process.poll() is not None:
            raise SystemExit("training finished before the kill point")
        try:
            done = torch.load(b / "checkpoint.pt", weights_only=True, map_location="cpu")["completed_steps"]
        except (FileNotFoundError, RuntimeError, EOFError, KeyError):
            done = 0
        if done >= args.kill_after:
            break
        time.sleep(1)
    tree = descendants(process.pid)
    for pid in [process.pid, *tree]:
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    process.wait()
    deadline = time.monotonic() + 30
    while any(alive(pid) for pid in tree):
        if time.monotonic() > deadline:
            raise SystemExit(f"processes survived: {[pid for pid in tree if alive(pid)]}")
        time.sleep(0.1)
seconds["interrupted"] = time.monotonic() - started
killed = torch.load(b / "checkpoint.pt", weights_only=True, map_location="cpu")["completed_steps"]
seconds["resumed"] = run("resumed", trainer(["--resume", str(b)]))

state_a, state_b = (torch.load(path / "checkpoint.pt", weights_only=True, map_location="cpu") for path in (a, b))
summary_a, summary_b = (json.loads((path / "summary.json").read_text()) for path in (a, b))
skip_state = {"elapsed_seconds", "loading_peak_rss_bytes"}
skip_summary = {"elapsed_seconds", "loading_peak_rss_bytes", "positions_per_second"}
result = {
    "killed_at_completed_steps": killed,
    "total_steps": state_a["total_steps"],
    "nproc": args.nproc,
    "seconds": seconds,
    "uninterrupted_elapsed_seconds": summary_a["elapsed_seconds"],
    "checkpoint": {key: same(state_a[key], state_b[key]) for key in sorted(state_a.keys() - skip_state)},
    "summary": {key: same(summary_a[key], summary_b[key]) for key in sorted(summary_a.keys() - skip_summary)},
    "model_onnx_bytes_equal": (a / "model.onnx").read_bytes() == (b / "model.onnx").read_bytes(),
    "network_sha256": [summary_a["network_sha256"], summary_b["network_sha256"]],
}
result["all_equal"] = (all(result["checkpoint"].values()) and all(result["summary"].values())
                       and result["model_onnx_bytes_equal"])
(args.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2))
