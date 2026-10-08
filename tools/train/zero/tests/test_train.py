"""学習ループ、チェックポイントからの再開、およびONNXの出力を、小さなネットとCPUで検査する。"""

from __future__ import annotations

import hashlib
import json
import math

import numpy as np
import onnxruntime as ort
import pytest
import torch

from minase_zero import train
from minase_zero.dataset import BatchDataset, validation_loader
from minase_zero.encoding import POLICY_SIZE
from minase_zero.mnzt import MnztCollection, sha256_file


def arguments(data, out, *extra) -> list[str]:
    """CPUで数ステップだけ走る小さな学習の引数を作る。"""
    return [
        "--data", str(data), "--out", str(out), "--blocks", "1", "--channels", "8",
        "--batch-size", "256", "--epochs", "2", "--lr", "0.01", "--seed", "5",
        "--log-interval", "1", "--eval-interval", "2", "--checkpoint-interval", "2",
        "--workers", "0", "--device", "cpu", *extra,
    ]


def read_log(run_dir) -> list[dict]:
    """`log.jsonl`を読み、計測時間に依存する欄を除く。"""
    entries = [json.loads(line) for line in (run_dir / "log.jsonl").read_text().splitlines()]
    for entry in entries:
        entry.pop("positions_per_second", None)
    return entries


def test_training_writes_finite_losses_checkpoint_and_onnx(sample_mnzt, tmp_path, capsys):
    """学習が有限の損失で走り、エポックのステップ数を表示し、ONNX、検査和、および来歴を書く。"""
    run_dir = tmp_path / "run"
    train.main(arguments(sample_mnzt, run_dir))
    output = capsys.readouterr().out
    assert "steps per epoch: " in output

    entries = read_log(run_dir)
    train_entries = [e for e in entries if e["kind"] == "train"]
    validation_entries = [e for e in entries if e["kind"] == "validation"]
    steps = train_entries[-1]["step"]
    assert steps >= 4
    assert [e["step"] for e in train_entries] == list(range(1, steps + 1))
    assert validation_entries
    for entry in entries:
        for key in ("loss", "policy_loss", "value_loss"):
            assert math.isfinite(entry[key])
    for entry in validation_entries:
        assert 0.0 <= entry["policy_agreement"] <= 1.0
        assert entry["positions"] > 0

    checkpoint = train.load_checkpoint(run_dir / "checkpoint.pt")
    assert checkpoint["step"] == steps
    assert checkpoint["data"] == [{"path": str(sample_mnzt), "sha256": sha256_file(sample_mnzt)}]

    onnx_path = run_dir / "net.onnx"
    digest = hashlib.sha256(onnx_path.read_bytes()).hexdigest()
    assert (run_dir / "net.sha256").read_text() == f"{digest}  net.onnx\n"
    session = ort.InferenceSession(str(onnx_path), providers=["CPUExecutionProvider"])
    policy, value = session.run(None, {"input": np.zeros((3, 95, 12, 12), dtype=np.float32)})
    assert policy.shape == (3, POLICY_SIZE)
    assert value.shape == (3, 1)

    provenance = json.loads((run_dir / "provenance.json").read_text())
    assert provenance["data"] == checkpoint["data"]
    assert provenance["seed"] == 5
    assert provenance["torch_version"] == torch.__version__
    assert provenance["started_at"] <= provenance["finished_at"]


class Interrupted(Exception):
    """学習を途中で止めるための例外。"""


def test_resume_reproduces_uninterrupted_training(sample_mnzt, tmp_path, monkeypatch):
    """保存の直後に止めて再開した学習が、止めなかった学習と同じ重み、ステップ数、および記録になる。"""
    straight = tmp_path / "straight"
    train.main(arguments(sample_mnzt, straight))

    original_save = train.save_checkpoint

    def save_then_stop(path, state, meta):
        original_save(path, state, meta)
        if state.step == 2:
            # 保存より後の記録が再開で除かれることも確かめるため、余分な行を残してから止める。
            train.append_log(path.parent / "log.jsonl", {"kind": "train", "step": 3, "loss": -1.0})
            raise Interrupted

    resumed = tmp_path / "resumed"
    monkeypatch.setattr(train, "save_checkpoint", save_then_stop)
    with pytest.raises(Interrupted):
        train.main(arguments(sample_mnzt, resumed))
    assert train.load_checkpoint(resumed / "checkpoint.pt")["step"] == 2
    monkeypatch.setattr(train, "save_checkpoint", original_save)
    train.main(["--resume", str(resumed), "--workers", "0", "--device", "cpu"])

    expected = train.load_checkpoint(straight / "checkpoint.pt")
    actual = train.load_checkpoint(resumed / "checkpoint.pt")
    assert actual["step"] == expected["step"]
    for key, tensor in expected["model"].items():
        assert torch.equal(actual["model"][key], tensor), key
    assert read_log(resumed) == read_log(straight)


def test_init_loads_weights_only(sample_mnzt, tmp_path):
    """`--init`は前の実行の重みを読み込んで始める。"""
    first = tmp_path / "first"
    train.main(arguments(sample_mnzt, first))
    second = tmp_path / "second"
    train.main(arguments(sample_mnzt, second, "--init", str(first / "checkpoint.pt"), "--epochs", "1"))
    provenance = json.loads((second / "provenance.json").read_text())
    assert provenance["init"]["sha256"] == sha256_file(first / "checkpoint.pt")


def test_resume_rejects_new_settings(tmp_path):
    """`--resume`と学習の設定を同時に指定するとエラーになる。"""
    with pytest.raises(SystemExit):
        train.parse_args(["--resume", str(tmp_path), "--lr", "0.1"])
    with pytest.raises(SystemExit):
        train.parse_args(["--data", "a.mnzt", "--out", str(tmp_path / "x"), "--seed", "1"])


def test_policy_loss_matches_dense_cross_entropy():
    """埋め草つきの組で計算した損失が、密な目標分布との交差エントロピーと一致する。"""
    torch.manual_seed(0)
    logits = torch.randn(2, POLICY_SIZE)
    labels = torch.tensor([[3, 900, 0], [7, 0, 0]])
    probs = torch.tensor([[0.25, 0.75, 0.0], [1.0, 0.0, 0.0]])
    dense = torch.zeros(2, POLICY_SIZE).scatter_add_(1, labels, probs)
    expected = -(dense * torch.log_softmax(logits, dim=1)).sum(dim=1)
    torch.testing.assert_close(train.policy_loss(logits, labels, probs), expected)
    logits[0, 900] = 100.0
    logits[1, 5] = 100.0
    assert train.policy_agreement(logits, labels, probs).tolist() == [True, False]


def test_worker_processes_read_memmapped_files(sample_mnzt):
    """ワーカーのプロセスへ渡したファイルからも同じバッチを作る。"""
    data = MnztCollection.open([sample_mnzt])
    indices = np.arange(64)
    dataset = BatchDataset(data, 0.333)
    serial = [b["value"] for b in validation_loader(dataset, indices, 16, workers=0, pin_memory=False)]
    parallel = [b["value"] for b in validation_loader(dataset, indices, 16, workers=2, pin_memory=False)]
    for expected, actual in zip(serial, parallel, strict=True):
        torch.testing.assert_close(actual, expected)
