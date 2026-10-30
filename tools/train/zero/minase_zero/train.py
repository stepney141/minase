"""自己対局の学習窓から局面を抽出し、更新の境界で保存して再開する。

trainの出力先には、checkpoint.pt、model.onnx、およびsummary.jsonを置く。
どれも同じディレクトリの一時ファイルへ書いて同期し、名前を変えてから親ディレクトリを
同期して確定させる。checkpoint.ptは--checkpoint-steps回の更新ごとと最後の更新の後に
置き換え、最新の1つだけを残す。最後の更新の後は、最終のチェックポイントを確定させてから
窓全体と最新サイクルの訓練標本、および検証集合を診断し、model.onnx、平均を持つ場合の
model-ema.onnx、summary.jsonの順に確定させて、
summary.jsonの存在をサイクルの完了とみなす。
すべての更新を終えたcheckpoint.ptが、次のサイクルの--initに与えるチェックポイントになる。

--latest-dataは最新サイクルの生成ファイルの列で、--dataの末尾とパスの文字列も順も一致させる。
窓のファイルを指定順に並べ、各ファイルの訓練局面だけに0からT−1の通し番号を振る。
最新サイクルの訓練局面数Nを抽出数とし、Nは1以上とする。抽出には、ほかの用途と共有しない
numpy.random.Generator(PCG64(seed))による部分的なFisher–Yates法を使う。
A[j]=jから始め、k=0,...,N−1でj=integers(k,T,dtype=int64)を引き、A[k]とA[j]を
交換してs[k]=A[k]とする。列sが抽出結果と更新順を同時に定める。
未使用のAのうち動かした要素だけを保持し、抽出にもO(N)のメモリを使う。
sample_sha256は列sをリトルエンディアンのint64で並べたバイト列のSHA-256である。
計数はMNSDの項目フラグを読んで再生なしで行い、その後、全ファイルを並列に1回再生して、
抽出した訓練局面と最新サイクルの検証局面だけを保持する。

Bをバッチの大きさとして、k番目の更新はs[kB:min((k+1)B,N)]を使い、全更新回数はceil(N/B)となる。
再開は完了した更新の回数kから先頭のmin(kB,N)個を飛ばす。保存したファイルの列を読み直して
抽出を作り直し、各ファイルの大きさ、訓練局面数、検証局面数、latest_data、N、T、
およびsample_sha256を照合する。ファイルのSHA-256は初回に来歴として計算し、再開時は計算し直さない。
data_order_sha256は、初期値を32バイトの0とし、更新のたびに「直前の値の32バイト」と
「その更新に使った抽出前の通し番号sの部分列をリトルエンディアンのint64で並べたバイト列」をつないだSHA-256で
置き換えた値の16進表記である。中断の有無にかかわらず、同じ学習窓とシードなら同じ値になる。

checkpoint.ptは、torch.load(weights_only=True)で読める辞書であり、重み（BNの移動統計を
含む）、最適化器の状態、完了した更新の回数、全更新回数、サイクル番号、シード、設定、
来歴、抽出条件とsample_sha256、累計の損失、学習曲線、およびdata_order_sha256を持つ。initのチェックポイントは
最適化器の状態を持たず、完了した更新の回数と全更新回数をともに0とする。

重みの指数移動平均（EMA）は--ema-period-samples=P、--ema-scale=Sをともに正にすると有効になる。
両方0なら平均を作らず、保存もしない。平均がない--initからはその重みを初期値として始め、
平均があれば重み、周期の余りの標本数、更新回数の累計を引き継ぐ。
各バッチの最適化後に読んだ標本数を加え、Pに達するたびに平均←平均+(現在の重み−平均)/Sとし、
Pを引いた余りを残す。1バッチで複数の周期を越えた場合も、その回数だけ更新する。
初期値の設定は更新回数に含めない。新しいサイクルでは指定されたPとSを使う。
checkpoint.ptのemaにはmodel、samples_since_update、updatesを保存し、--resumeでも復元する。
KataGoのAveragedModel(use_buffers=False)に合わせ、BatchNormの学習する係数は平均し、
running_mean、running_var、num_batches_trackedは平均の更新時の値を複写する。
診断とONNX出力はこの保存した統計を使い、統計の再計算や最終の通常重みからの複写は行わない。

trainのsummary.jsonの欄は次のとおりである。

| 欄 | 意味 |
|---|---|
| cycle | サイクル番号。 |
| config | ネットの大きさと、新しいサイクルに指定した設定。出力先は含めない。 |
| provenance | 学習器と環境定義のコミット、実行環境、初期重みと教師ファイルの来歴。 |
| latest_data | 最新サイクルの生成ファイルの列。以下の抽出の4欄とともにconfigにも記録する。 |
| sample_count | 抽出数N。最新サイクルの訓練局面数に等しい。 |
| window_training_count | 窓全体の抽出前の訓練局面数T。 |
| file_training_counts | ファイルごとの抽出前の訓練局面数。dataの順に並ぶ。 |
| sample_sha256 | 抽出した通し番号を更新順に並べた列sのSHA-256。 |
| training_count / validation_count | 抽出した訓練局面数Nと、最新サイクルの検証局面数。 |
| completed_steps / total_steps | 完了した更新回数と、このサイクルの全更新回数。 |
| positions_seen | 完了した更新で読んだ訓練局面数。 |
| training_losses | 更新中の重み・訓練モードで局面数を重みとして平均したpolicy、value、value_result。 |
| diagnostics.training_sample | 抽出した訓練局面のうち標本の対局に属するものを、最終の重みの推論モードで診断した値。 |
| diagnostics.latest_training_sample | training_sampleのうちlatest_dataに属する局面を、同じ関数で診断した値。 |
| diagnostics.validation | latest_dataの検証局面を、同じ関数で診断した値。 |
| diagnosticsの各集合のgame_count / position_count | 診断に含めた対局数と採用局面数。 |
| diagnosticsの各集合のhash_modulus / hash_remainder | hash64(seed, game)の法20と剰余。訓練標本は1、検証は0。 |
| diagnosticsの各集合のgames | 採用局面を持つ対局のseed（ヘッダの基本シード）とgame（対局番号）の列。 |
| diagnosticsの各集合のpolicy / value / value_result | 局面単位の算術平均による3つの損失。 |
| diagnosticsの各集合のlegal_probability_mass / log_legal_probability_mass | 全合法手に乗る確率質量の平均と、その自然対数の平均。 |
| diagnosticsの各集合のpolicy_agreement | 合法手の最大logitの手が、教師の最大確率の手の集合に含まれる割合。 |
| learning_curve | 保存間隔ごとのcompleted_stepsと、その区間のtraining_losses。 |
| data_order_sha256 | 更新に使った局面の通し番号を連鎖させたSHA-256。 |
| network_sha256 | model.onnxのSHA-256。 |
| ema_network_sha256 | 平均を持つ場合のmodel-ema.onnxのSHA-256。 |
| ema_period_samples / ema_scale / ema_updates | 平均を持つ場合のP、S、およびサイクルをまたぐ更新回数の累計。 |
| ema_diagnostics | 平均を持つ場合に、diagnosticsと同じ3集合と関数で求めた診断。 |
| elapsed_seconds | loading、training、diagnosticsの秒数。 |
| loading_peak_rss_bytes | 読み込み直後までのプロセスの最大常駐メモリ。resource.getrusageで測り、バイトへ換算する。 |
| positions_per_second | positions_seenをtrainingの秒数で割った値。 |

policyは定数マスクが真となる16,108ラベルで正規化した交差エントロピー、valueは結果と根の価値を混ぜた
教師に対する二値交差エントロピー、value_resultは結果だけに対する同じ損失である。
価値の教師は (1−λ)·結果 + λ·根の価値 であり、λは--value-lambdaで与える。
損失は方策と価値の和で、最適化器はネステロフのモーメンタムつきSGDとする。
--precision bf16では、ネットの順伝播だけをbf16のautocastで計算する。
損失の計算と累計はfloat32で行う。訓練集合、2つの訓練標本、検証集合のいずれかが空なら、平均を定義できない
ためエラーにする。シードはuint64、サイクルは0以上の整数とする。

loadingは教師の読み込みと初回の検査和計算、trainingはバッチの準備と更新、
diagnosticsは3集合のバッチの準備と推論を、平均を持つ場合はその診断も含めて測る。保存とONNX出力は含めない。
loading_peak_rss_bytesは読み込みだけの増分ではなく、プロセス開始後の最大値である。
再開時は保存済みの値と今回の読み込み直後の値の大きい方を記録する。
再開時は保存済みの時間に今回の読み込みと更新の時間を加え、中断で失われた更新や
停止中の時間は含めない。再開に指定した機器は実行に使い、configとprovenanceには
サイクル開始時の設定と環境を保持する。

診断ではeval()とinference_mode()を使い、勾配・バッチ正規化の移動統計・乱数状態を変えない。
合法手の確率質量は、定数マスク内の全ラベルを分母、反復の履歴を考えない全合法手を
分子とする。最大値を引いて単精度で計算し、対数は質量の指数化前に求める。
方策の一致率は、ネット側の同率なら最小ラベルを選び、教師側の同率はすべて正解とする。
最後のチェックポイントで更新済みの重みから再開する場合も、3集合の診断をやり直す。

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
import resource
import sys
import tempfile
import time

import numpy as np
from numpy.typing import NDArray
import torch
from torch.nn import functional as F

from .data import Dataset, FileInfo, read_window
from .encoding import encode_planes, policy_mask
from .fixture import evaluate_fixture
from .net import Ema, ZeroNet, export_onnx, validate_ema


LOSS_NAMES = ("policy", "value", "value_result")
DIAGNOSTIC_NAMES = (*LOSS_NAMES, "legal_probability_mass", "log_legal_probability_mass", "policy_agreement")


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
    validate_ema(config["ema_period_samples"], config["ema_scale"])
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
    results: torch.Tensor, values: torch.Tensor, value_lambda: float, mask: torch.Tensor,
) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
    """疎な方策と量子化した価値から、単精度の平均損失3つを計算する。

    rowsとlabelsは候補の行番号とラベル、probabilitiesとvaluesは0〜65,535、
    resultsは手番側の結果、maskは同じ機器に置いた方策の定数マスクである。
    マスク外の教師ラベルはValueErrorとし、結果だけの監視用損失には勾配を持たせない。
    """
    policy_logits, value_logits = policy_logits.float(), value_logits.float().flatten()
    probabilities, results, values = probabilities.float(), results.float(), values.float()
    if torch.any((labels < 0) | (labels >= len(mask))) or not torch.all(mask[labels]):
        raise ValueError("policy target label is outside the constant mask")
    policy_logits = policy_logits.masked_fill(~mask, -torch.inf)
    candidate_losses = -F.log_softmax(policy_logits, dim=1)[rows, labels] * (probabilities / 65_535)
    per_position = torch.zeros(len(results), dtype=torch.float32, device=policy_logits.device)
    policy_loss = per_position.scatter_add_(0, rows, candidate_losses).mean()
    target = (1 - value_lambda) * results + value_lambda * (values / 65_535)
    value_loss = F.binary_cross_entropy_with_logits(value_logits, target)
    with torch.no_grad():
        result_loss = F.binary_cross_entropy_with_logits(value_logits, results)
    return policy_loss, value_loss, result_loss


def _sparse_indices(
    offsets: NDArray[np.uint64], indices: NDArray[np.int64],
) -> tuple[NDArray[np.int64], NDArray[np.int64]]:
    starts, ends = offsets[indices], offsets[indices + 1]
    entries = np.concatenate([np.arange(start, end, dtype=np.int64) for start, end in zip(starts, ends)])
    rows = np.repeat(np.arange(len(indices), dtype=np.int64), (ends - starts).astype(np.int64))
    return rows, entries


def _batch(
    model: ZeroNet, data: Dataset, indices: NDArray[np.int64],
    device: torch.device, config: dict,
) -> tuple[torch.Tensor, ...]:
    """順伝播の出力と疎な教師を、compute_lossesの先頭7引数の順に返す。"""
    boards, sides, lions = (
        torch.from_numpy(array[indices]).to(device) for array in (data.boards, data.sides, data.lions)
    )
    inputs = encode_planes(boards, sides, lions)
    row_indices, candidates = _sparse_indices(data.policy_offsets, indices)
    rows = torch.from_numpy(row_indices).to(device)
    labels = torch.from_numpy(data.policy_labels[candidates].astype(np.int64)).to(device)
    probabilities = torch.from_numpy(data.policy_probabilities[candidates].astype(np.float32)).to(device)
    results = torch.from_numpy(data.results[indices]).to(device)
    values = torch.from_numpy(data.values[indices].astype(np.float32)).to(device)
    with torch.autocast("cuda", dtype=torch.bfloat16) if config["precision"] == "bf16" else nullcontext():
        policy_logits, value_logits = model.forward_logits(inputs)
    return policy_logits, value_logits, rows, labels, probabilities, results, values


def compute_policy_diagnostics(
    policy_logits: torch.Tensor, rows: torch.Tensor, labels: torch.Tensor, probabilities: torch.Tensor,
    legal_rows: torch.Tensor, legal_labels: torch.Tensor, mask: torch.Tensor,
) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
    """合法手の質量・その自然対数・教師との一致率を、局面単位で平均する。

    legal_rowsとlegal_labelsは各局面の全合法手を重複なく表す。
    ネットの最大logitが同率なら最小ラベルを選び、教師の同率はすべて正解とする。
    マスク外の合法手、合法手がない局面、有限でない合法手の最大logitはValueErrorとする。
    """
    if torch.any((legal_labels < 0) | (legal_labels >= len(mask))) or not torch.all(mask[legal_labels]):
        raise ValueError("legal label is outside the constant mask")
    logits = policy_logits.float().masked_fill(~mask, -torch.inf)
    maximum = logits.amax(dim=1)
    shifted = logits - maximum[:, None]
    log_denominator = shifted.exp().sum(dim=1).log()
    legal_logits = logits[legal_rows, legal_labels]
    count, size = logits.shape
    legal_max = torch.full((count,), -torch.inf, dtype=torch.float32, device=logits.device).scatter_reduce_(
        0, legal_rows, legal_logits, reduce="amax",
    )
    if not torch.all(torch.isfinite(legal_max)):
        raise ValueError("each position must have a finite maximum legal logit")
    legal_sum = torch.zeros(count, dtype=torch.float32, device=logits.device).scatter_add_(
        0, legal_rows, (legal_logits - legal_max[legal_rows]).exp(),
    )
    log_mass = (legal_max - maximum) + legal_sum.log() - log_denominator
    best_labels = torch.full((count,), size, dtype=torch.int64, device=logits.device).scatter_reduce_(
        0, legal_rows, torch.where(legal_logits == legal_max[legal_rows], legal_labels, size), reduce="amin",
    )
    probabilities = probabilities.float()
    teacher_max = torch.zeros(count, dtype=torch.float32, device=logits.device).scatter_reduce_(
        0, rows, probabilities, reduce="amax",
    )
    matches = (labels == best_labels[rows]) & (probabilities == teacher_max[rows])
    agreement = torch.zeros(count, dtype=torch.float32, device=logits.device).scatter_add_(0, rows, matches.float()) > 0
    return log_mass.exp().mean(), log_mass.mean(), agreement.float().mean()


@torch.inference_mode()
def diagnose(
    model: ZeroNet, data: Dataset, indices: NDArray[np.int64],
    device: torch.device, config: dict, mask: torch.Tensor,
) -> dict[str, float]:
    """最終の重みのevalモードで、指定局面を入力順に診断する。乱数は使わない。"""
    if len(indices) == 0:
        raise ValueError("diagnostic set must contain positions")
    model.eval()
    sums = dict.fromkeys(DIAGNOSTIC_NAMES, 0.0)
    for start in range(0, len(indices), config["batch_size"]):
        batch = indices[start:start + config["batch_size"]]
        tensors = _batch(model, data, batch, device, config)
        losses = compute_losses(*tensors, config["value_lambda"], mask)
        legal_rows, legal_entries = _sparse_indices(data.legal_offsets, batch)
        metrics = compute_policy_diagnostics(
            tensors[0], tensors[2], tensors[3], tensors[4],
            torch.from_numpy(legal_rows).to(device),
            torch.from_numpy(data.legal_labels[legal_entries].astype(np.int64)).to(device), mask,
        )
        if not all(torch.isfinite(value).item() for value in (*losses, *metrics)):
            raise ValueError("non-finite diagnostic value")
        _accumulate(sums, (*losses, *metrics), len(batch))
    return _mean(sums, len(indices))


def _zero_losses() -> dict[str, float]:
    return dict.fromkeys(LOSS_NAMES, 0.0)


def _accumulate(sums: dict[str, float], losses: tuple[torch.Tensor, ...], count: int) -> None:
    for name, loss in zip(sums, losses, strict=True):
        sums[name] = float(np.float32(sums[name]) + np.float32(loss.detach().item()) * np.float32(count))


def _mean(sums: dict[str, float], count: int) -> dict[str, float]:
    return {name: float(np.float32(value) / np.float32(count)) for name, value in sums.items()}


def _counts(data: Dataset, latest_start: int) -> tuple[int, int]:
    validation = int(np.count_nonzero(data.validation))
    training = len(data) - validation
    if training == 0 or validation == 0:
        raise ValueError("training and validation sets must both contain positions")
    if not np.any(data.training_sample):
        raise ValueError("training diagnostic sample must contain positions")
    if not np.any(data.training_sample & (data.file_indices >= latest_start)):
        raise ValueError("latest training diagnostic sample must contain positions")
    return training, validation


def _sampling_metadata(config: dict, infos: list[FileInfo], selected: NDArray[np.int64]) -> dict:
    return {
        "latest_data": config["latest_data"],
        "sample_count": len(selected),
        "window_training_count": sum(info.training_count for info in infos),
        "file_training_counts": [info.training_count for info in infos],
        "sample_sha256": hashlib.sha256(selected.astype("<i8").tobytes()).hexdigest(),
    }


def _peak_rss_bytes() -> int:
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return peak if sys.platform == "darwin" else peak * 1024


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
    data, infos, selected = read_window(config["data"], config["latest_data"], config["seed"])
    loading_peak_rss = _peak_rss_bytes()
    training_count, validation_count = _counts(data, len(infos) - len(config["latest_data"]))
    sampling = _sampling_metadata(config, infos, selected)
    config.update(sampling)
    provenance = _environment(config["trainer_commit"], device)
    provenance["init_sha256"] = _sha256(Path(config["init"]))
    provenance["files"] = [_file_provenance(path, info) for path, info in zip(config["data"], infos)]
    state = {
        "model": initial["model"], "optimizer": initial["optimizer"],
        "completed_steps": 0, "total_steps": (training_count + config["batch_size"] - 1) // config["batch_size"],
        "cycle": config["cycle"], "seed": config["seed"], "config": config, "provenance": provenance,
        "training_count": training_count, "validation_count": validation_count,
        **sampling,
        "positions_seen": 0, "loss_sums": _zero_losses(), "learning_curve": [],
        "data_order_sha256": bytes(32).hex(),
        "elapsed_seconds": {"loading": time.perf_counter() - started, "training": 0.0},
        "loading_peak_rss_bytes": loading_peak_rss,
    }
    if config["ema_period_samples"] > 0:
        state["ema"] = (initial["ema"] if "ema" in initial else
                        {"model": initial["model"], "samples_since_update": 0, "updates": 0})
    _run(state, data, selected, out_dir, device)


def resume(out_dir: Path, device_name: str) -> None:
    """保存したファイル情報と抽出条件を照合し、最後に確定した更新から再開する。"""
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
    data, infos, selected = read_window(paths, config["latest_data"], config["seed"])
    state["loading_peak_rss_bytes"] = max(state["loading_peak_rss_bytes"], _peak_rss_bytes())
    for record, info in zip(records, infos):
        if (record["size"], record["training_count"], record["validation_count"]) != (
            info.size, info.training_count, info.validation_count,
        ):
            raise ValueError(f"training window size or position counts differ: {record['path']}")
    for key, value in _sampling_metadata(config, infos, selected).items():
        if state[key] != value or config[key] != value:
            raise ValueError(f"training window sampling condition differs: {key}")
    if _counts(data, len(infos) - len(config["latest_data"])) != (state["training_count"], state["validation_count"]):
        raise ValueError("training window total position counts differ")
    batch_size = config["batch_size"]
    if state["total_steps"] != (state["training_count"] + batch_size - 1) // batch_size:
        raise ValueError("checkpoint total_steps disagrees with training_count and batch_size")
    if state["positions_seen"] != min(state["completed_steps"] * batch_size, state["training_count"]):
        raise ValueError("checkpoint positions_seen disagrees with completed_steps")
    state["elapsed_seconds"]["loading"] += time.perf_counter() - started
    print(f"resume: completed_steps={state['completed_steps']}/{state['total_steps']}", file=sys.stderr, flush=True)
    _run(state, data, selected, out_dir, device)


def _run(
    state: dict, data: Dataset, selected: NDArray[np.int64], out_dir: Path, device: torch.device,
) -> None:
    config = state["config"]
    model = _model(state).to(device).train()
    ema = None
    if config["ema_period_samples"] > 0:
        ema = Ema(model, config["ema_period_samples"], config["ema_scale"])
        ema.load_state_dict(state["ema"])
    mask = torch.from_numpy(policy_mask()).to(device)
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
    # 保持した訓練局面は通し番号の昇順なので、抽出した順から配列の行番号へ戻す。
    order = indices[np.searchsorted(np.sort(selected), selected)]
    batch_size = config["batch_size"]
    interval_sums, interval_count = _zero_losses(), 0
    interval_started = time.perf_counter()
    for step in range(state["completed_steps"], state["total_steps"]):
        batch = order[step * batch_size:(step + 1) * batch_size]
        optimizer.zero_grad(set_to_none=True)
        losses = compute_losses(*_batch(model, data, batch, device, config), config["value_lambda"], mask)
        if not all(torch.isfinite(loss).item() for loss in losses):
            raise ValueError(f"non-finite training loss at update {step + 1}")
        (losses[0] + losses[1]).backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), config["clip_grad_norm"], error_if_nonfinite=True)
        optimizer.step()
        if ema is not None:
            ema.update(model, len(batch))
        _accumulate(state["loss_sums"], losses, len(batch))
        _accumulate(interval_sums, losses, len(batch))
        state["positions_seen"] += len(batch)
        interval_count += len(batch)
        state["completed_steps"] = step + 1
        state["data_order_sha256"] = hashlib.sha256(
            bytes.fromhex(state["data_order_sha256"])
            + selected[step * batch_size:(step + 1) * batch_size].astype("<i8").tobytes()
        ).hexdigest()
        if (step + 1) % config["checkpoint_steps"] == 0 or step + 1 == state["total_steps"]:
            elapsed = time.perf_counter() - interval_started
            state["elapsed_seconds"]["training"] += elapsed
            averages = _mean(interval_sums, interval_count)
            state["learning_curve"].append({"completed_steps": step + 1, "training_losses": averages})
            state["model"], state["optimizer"] = model.state_dict(), optimizer.state_dict()
            if ema is not None:
                state["ema"] = ema.state_dict()
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
    diagnostics = {}
    ema_diagnostics = {}
    latest = data.file_indices >= len(config["data"]) - len(config["latest_data"])
    for name, membership, remainder in (("training_sample", data.training_sample, 1),
                                        ("latest_training_sample", data.training_sample & latest, 1),
                                        ("validation", data.validation, 0)):
        indices = np.flatnonzero(membership)
        games = [
            {"seed": state["provenance"]["files"][int(file)]["header"]["seed"], "game": int(game)}
            for file in np.unique(data.file_indices[indices])
            for game in np.unique(data.games[indices][data.file_indices[indices] == file])
        ]
        metadata = {
            "game_count": len(games), "position_count": len(indices),
            "hash_modulus": 20, "hash_remainder": remainder, "games": games,
        }
        diagnostics[name] = {**metadata, **diagnose(model, data, indices, device, config, mask)}
        if ema is not None:
            ema_diagnostics[name] = {**metadata, **diagnose(ema.model, data, indices, device, config, mask)}
    diagnostic_seconds = time.perf_counter() - started
    with _atomic_path(out_dir / "model.onnx") as temporary:
        export_onnx(copy.deepcopy(model).cpu().float().eval(), temporary)
    ema_summary = {}
    if ema is not None:
        with _atomic_path(out_dir / "model-ema.onnx") as temporary:
            export_onnx(copy.deepcopy(ema.model).cpu().float().eval(), temporary)
        ema_summary = {
            "ema_network_sha256": _sha256(out_dir / "model-ema.onnx"),
            "ema_period_samples": config["ema_period_samples"], "ema_scale": config["ema_scale"],
            "ema_updates": ema.updates, "ema_diagnostics": ema_diagnostics,
        }
    _write_json(out_dir / "summary.json", {
        **ema_summary,
        "cycle": state["cycle"], "config": config, "provenance": state["provenance"],
        "training_count": state["training_count"], "validation_count": state["validation_count"],
        **{key: state[key] for key in ("latest_data", "sample_count", "window_training_count",
                                      "file_training_counts", "sample_sha256")},
        "completed_steps": state["completed_steps"], "total_steps": state["total_steps"],
        "positions_seen": state["positions_seen"],
        "training_losses": _mean(state["loss_sums"], state["positions_seen"]),
        "diagnostics": diagnostics, "learning_curve": state["learning_curve"],
        "data_order_sha256": state["data_order_sha256"], "network_sha256": _sha256(out_dir / "model.onnx"),
        "elapsed_seconds": {**state["elapsed_seconds"], "diagnostics": diagnostic_seconds},
        "loading_peak_rss_bytes": state["loading_peak_rss_bytes"],
        "positions_per_second": state["positions_seen"] / state["elapsed_seconds"]["training"],
    })


def fixture_eval(checkpoint: Path, onnx: Path, fixture: Path, out: Path, weights: str) -> None:
    """指定した通常または平均の重みとONNXの一致を確かめ、固定局面集をCPUで推論する。"""
    state = _load_checkpoint(checkpoint)
    if weights == "ema":
        if "ema" not in state:
            raise ValueError("checkpoint does not contain EMA weights")
        state["model"] = state["ema"]["model"]
    elif weights != "raw":
        raise ValueError("weights must be raw or ema")
    model = _model(state).eval()
    onnx_bytes = onnx.read_bytes()
    with tempfile.TemporaryDirectory(prefix="minase-zero-fixture-") as directory:
        path = Path(directory) / "model.onnx"
        export_onnx(model, path)
        if path.read_bytes() != onnx_bytes:
            raise ValueError("checkpoint ONNX bytes differ from --onnx; weights are not identical")
    _write_json(out, evaluate_fixture(model, fixture, onnx_bytes))
