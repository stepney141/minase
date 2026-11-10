"""実際のPythonサブプロセスで学習器を起動する試験用の共通処理。"""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


ZERO = Path(__file__).resolve().parents[1]
COMMIT = "0123456789abcdef" * 2 + "01234567"


def environment() -> dict[str, str]:
    """CPUの試験を1スレッドに固定し、共用機の負荷を制限する。"""
    return {**os.environ, "OMP_NUM_THREADS": "1", "MKL_NUM_THREADS": "1"}


def command(arguments: list[str], world_size: int = 1) -> list[str]:
    """trainはtorchrunから、ほかのサブコマンドは通常のPythonから起動する。"""
    launcher = [sys.executable, "-m"]
    if arguments and arguments[0] == "train":
        launcher += ["torch.distributed.run", "--standalone", "--nproc-per-node", str(world_size), "-m"]
    return [*launcher, "minase_zero", *arguments]


def run_cli(arguments: list[str], world_size: int = 1) -> subprocess.CompletedProcess[str]:
    """指定した引数を実際のCLIへ渡し、終了コードと出力を回収する。"""
    return run_command(command(arguments, world_size))


def run_command(arguments: list[str]) -> subprocess.CompletedProcess[str]:
    """ワーカーを含むプロセスグループを、制限時間超過や試験の中断時にも回収する。"""
    with subprocess.Popen(arguments, cwd=ZERO, env=environment(), stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True, start_new_session=True) as process:
        try:
            stdout, stderr = process.communicate(timeout=180)
        except BaseException:
            kill_training(process)
            raise
        return subprocess.CompletedProcess(arguments, process.returncode, stdout, stderr)


def init_arguments(out: Path, seed: int) -> list[str]:
    """固定した小さなネットを作るための必須引数をすべて返す。"""
    return ["init", "--blocks", "1", "--channels", "16", "--seed", str(seed),
            "--out-dir", str(out), "--trainer-commit", COMMIT]


def train_arguments(initial: Path, out: Path) -> list[str]:
    """設計書の再開試験で使う1サイクル分の設定を返す。"""
    data_list = out.with_name(out.name + "-data.json")
    data_list.parent.mkdir(parents=True, exist_ok=True)
    data_list.write_text(json.dumps(["fixtures/zero-selfplay-small.mnsd"]), encoding="utf-8")
    return [
        "train", "--init", str(initial), "--data-list", str(data_list),
        "--latest-data", "fixtures/zero-selfplay-small.mnsd",
        "--out-dir", str(out), "--cycle", "1", "--seed", "42", "--batch-size", "16",
        "--micro-batch-size", "4", "--replay-threads", "1",
        "--lr", "0.01", "--momentum", "0.9", "--weight-decay", "0.0001",
        "--clip-grad-norm", "10", "--value-lambda", "0.333", "--precision", "fp32",
        "--checkpoint-steps", "4", "--device", "cpu", "--trainer-commit", COMMIT,
    ]


def require_success(result: subprocess.CompletedProcess[str]) -> None:
    """試験の準備が失敗した場合も、子プロセスのエラーを隠さず失敗させる。"""
    if result.returncode != 0:
        raise AssertionError(f"exit={result.returncode}\n{result.stdout}\n{result.stderr}")


def set_training_data(arguments: list[str], files: list[str]) -> None:
    """CLIへ渡すJSON配列を指定したファイルの順に書く。"""
    Path(arguments[arguments.index("--data-list") + 1]).write_text(json.dumps(files), encoding="utf-8")


def kill_training(process: subprocess.Popen) -> int:
    """終了前に子孫を集め、別セッションのワーカーも止まったことを確かめる。"""
    processes = {}
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
        except (FileNotFoundError, ProcessLookupError):
            continue
        processes[int(entry.name)] = (int(fields[1]), int(fields[2]))
    descendants = {process.pid}
    pending = [process.pid]
    while pending:
        parent = pending.pop()
        for pid, (ppid, _) in processes.items():
            if ppid == parent:
                descendants.add(pid)
                pending.append(pid)
    groups = {process.pid}
    groups.update(pgid for pid, (_, pgid) in processes.items() if pid in descendants)
    for group in sorted(groups):
        try:
            os.killpg(group, signal.SIGKILL)
        except ProcessLookupError:
            pass
    code = process.wait(timeout=30)
    # 親を失った子孫も、収集済みのPIDで調べる。initの回収前のゾンビは実行を終えている。
    deadline = time.monotonic() + 5
    while True:
        surviving = []
        for pid in sorted(descendants):
            try:
                fields = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
            except (FileNotFoundError, ProcessLookupError):
                continue
            if fields[0] not in ("Z", "X"):
                surviving.append(pid)
        if not surviving:
            break
        if time.monotonic() >= deadline:
            raise AssertionError(f"training processes survived SIGKILL: {surviving}")
        time.sleep(0.002)
    return code
