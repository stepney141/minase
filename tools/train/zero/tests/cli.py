"""実際のPythonサブプロセスで学習器を起動する試験用の共通処理。"""

import os
from pathlib import Path
import subprocess
import sys


ZERO = Path(__file__).resolve().parents[1]
COMMIT = "0123456789abcdef" * 2 + "01234567"


def environment() -> dict[str, str]:
    """CPUの試験を1スレッドに固定し、共用機の負荷を制限する。"""
    return {**os.environ, "OMP_NUM_THREADS": "1", "MKL_NUM_THREADS": "1"}


def run_cli(arguments: list[str]) -> subprocess.CompletedProcess[str]:
    """指定した引数を実際のCLIへ渡し、終了コードと出力を回収する。"""
    return subprocess.run(
        [sys.executable, "-m", "minase_zero", *arguments], cwd=ZERO,
        env=environment(), capture_output=True, text=True, timeout=120,
    )


def init_arguments(out: Path, seed: int) -> list[str]:
    """固定した小さなネットを作るための必須引数をすべて返す。"""
    return ["init", "--blocks", "1", "--channels", "16", "--seed", str(seed),
            "--out-dir", str(out), "--trainer-commit", COMMIT]


def train_arguments(initial: Path, out: Path) -> list[str]:
    """設計書の再開試験で使う1サイクル分の設定を返す。"""
    return [
        "train", "--init", str(initial), "--data", "fixtures/zero-selfplay-small.mnsd",
        "--latest-data", "fixtures/zero-selfplay-small.mnsd",
        "--out-dir", str(out), "--cycle", "1", "--seed", "42", "--batch-size", "16",
        "--lr", "0.01", "--momentum", "0.9", "--weight-decay", "0.0001",
        "--clip-grad-norm", "10", "--value-lambda", "0.333", "--precision", "fp32",
        "--checkpoint-steps", "4", "--device", "cpu", "--trainer-commit", COMMIT,
    ]


def require_success(result: subprocess.CompletedProcess[str]) -> None:
    """試験の準備が失敗した場合も、子プロセスのエラーを隠さず失敗させる。"""
    if result.returncode != 0:
        raise AssertionError(f"exit={result.returncode}\n{result.stdout}\n{result.stderr}")
