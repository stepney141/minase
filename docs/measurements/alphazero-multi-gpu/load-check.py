"""大きなリプレイバッファの読み込みの時間と最大常駐メモリを、最初のチェックポイントから読む。

使い方（作業ディレクトリは学習器のtools/train/zero）:
    uv run --frozen python <このファイル> --out <新しいディレクトリ> --commit <40桁> --init <checkpoint.pt> \
        --data-list <JSON配列のファイル> --latest-list <JSON配列のファイル> --nproc <プロセス数> --replay-threads <スレッド数>

学習器を全体のバッチ1,024、ミクロバッチ256、--checkpoint-steps 1で起動し、最初のcheckpoint.ptが現れたらtorchrunと子孫のすべてへSIGKILLを送る。
チェックポイントのelapsed_seconds.loading（全プロセスの最大）とloading_peak_rss_bytes（全プロセスの和）を、作業機の時計で測った起動から最初の保存までの秒数とともに<out>/result.jsonに書く。
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
for name in ("out", "init", "data-list", "latest-list"):
    parser.add_argument(f"--{name}", type=Path, required=True)
parser.add_argument("--commit", required=True)
for name in ("nproc", "replay-threads"):
    parser.add_argument(f"--{name}", type=int, required=True)
args = parser.parse_args()
latest = json.loads(args.latest_list.read_text())
args.out.mkdir(parents=True)
run = args.out / "train"
command = ["uv", "run", "--frozen", "torchrun", "--standalone", "--nproc-per-node", str(args.nproc),
           "-m", "minase_zero", "train", "--init", str(args.init), "--data-list", str(args.data_list),
           "--latest-data", *latest, "--out-dir", str(run), "--cycle", "1", "--seed", "20261101",
           "--batch-size", "1024", "--micro-batch-size", "256", "--lr", "0.04", "--momentum", "0.9",
           "--weight-decay", "0.0001", "--clip-grad-norm", "10", "--value-lambda", "0.333",
           "--precision", "bf16", "--checkpoint-steps", "1", "--device", "cuda",
           "--replay-threads", str(args.replay_threads), "--trainer-commit", args.commit]
caches = args.out / "cache"
environment = {**os.environ, "TORCHINDUCTOR_CACHE_DIR": str(caches / "inductor"),
               "TRITON_CACHE_DIR": str(caches / "triton")}


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


started = time.monotonic()
with (args.out / "train.log").open("w") as log:
    process = subprocess.Popen(command, cwd=Path.cwd(), env=environment, stdout=log, stderr=log,
                               start_new_session=True)
    while not (run / "checkpoint.pt").exists():
        if process.poll() is not None:
            raise SystemExit(f"trainer exited {process.returncode} before the first checkpoint")
        time.sleep(1)
    first_checkpoint_seconds = time.monotonic() - started
    time.sleep(5)
    for pid in [process.pid, *descendants(process.pid)]:
        try:
            os.killpg(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    process.wait()
state = torch.load(run / "checkpoint.pt", weights_only=True, map_location="cpu")
result = {
    "nproc": args.nproc, "replay_threads": args.replay_threads,
    "files": len(state["config"]["data"]),
    "replay_buffer_training_count": state["replay_buffer_training_count"],
    "sample_count": state["sample_count"], "training_count": state["training_count"],
    "loading_seconds": state["elapsed_seconds"]["loading"],
    "loading_peak_rss_bytes": state["loading_peak_rss_bytes"],
    "first_checkpoint_seconds": first_checkpoint_seconds,
}
(args.out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
print(json.dumps(result, indent=2))
