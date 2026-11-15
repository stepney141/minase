"""自己対局のリプレイバッファから局面を抽出し、更新の境界で保存して再開する。

trainの出力先には、checkpoint.pt、model.onnx、およびsummary.jsonを置く。
どれも同じディレクトリの一時ファイルへ書いて同期し、名前を変えてから親ディレクトリを同期して確定させる。
checkpoint.ptは--checkpoint-steps回の更新ごとと最後の更新の後に置き換え、最新の1つだけを残す。
最後の更新の後は、最終のチェックポイントを確定させてからリプレイバッファ全体と最新サイクルの訓練標本、および検証集合を診断し、model.onnx、summary.jsonの順に確定させて、summary.jsonの存在をサイクルの完了とみなす。
すべての更新を終えたcheckpoint.ptが、次のサイクルの--initに与えるチェックポイントになる。

--data-listは、空でないパスの文字列を要素とする、空でないUTF-8のJSON配列のファイルを指定する。
読み込んだ配列をconfig["data"]に保存し、リストのファイル名は保存しない。
再開ではリストのファイルを読まない。
--latest-dataは最新サイクルの生成ファイルの列で、読み込んだ配列の末尾とパスの文字列も順も一致させる。
リプレイバッファのファイルを指定順に並べ、各ファイルの訓練局面だけに0からT−1の通し番号を振る。
最新サイクルの訓練局面数Lを抽出数Sとし、Lは1以上とする。
抽出はdata.sample_training_indicesの部分的なFisher–Yates法に従い、Generator(PCG64(seed))でT個の局面からL個を重複なく選ぶ。
列sが抽出結果と更新順を同時に定める。
sample_sha256は列sをリトルエンディアンのint64で並べたバイト列のSHA-256である。
計数はMNSDの項目フラグを読んで再生なしで行い、その後、全ファイルを並列に1回再生して、各ランクが担当するミクロバッチに含まれる局面を、通し番号の重複を除いて保持する。
ランク0は、端数を除いた抽出列の訓練診断標本と最新サイクルの検証局面も保持する。
再生のスレッド数は各プロセスの--replay-threadsで指定し、設定や再開の照合には含めない。

trainは、新しいサイクルも再開も単一機械のtorchrunから起動する。
WORLD_SIZE、LOCAL_WORLD_SIZE、RANK、LOCAL_RANKは必須で、2つのプロセス数は一致させる。
プロセス数をN、--batch-sizeで与える全体のバッチをB、--micro-batch-sizeをm、K=B/mとする。
Bとmは正の整数、Bはmの倍数、KはNの倍数とし、Fabricの作成後、再生前に検査する。
抽出列の先頭M=B×floor(S/B)個を使い、端数を捨て、S<Bなら再生前にエラーにする。
k番目の更新はs[kB:(k+1)B]を使い、全更新回数はfloor(S/B)となる。
ミクロバッチjはs[kB+jm:kB+(j+1)m]で、ランクrはj mod N=rのものを番号順に処理する。
Lightning Fabricは精度32-trueの分散データ並列（DDP）を使い、自分の最後のミクロバッチだけで勾配を同期する。
DDPのfind_unused_parameters=Trueで勾配のバケットの組み替えを止め、再開前後の加算順をそろえる。
学習用の全出力を返すTrainingModuleの順伝播を、torch.compile(..., fullgraph=True)の既定モードで常にコンパイルする。
torch._inductor.config.emulate_precision_casts=Trueをプロセス全体に設定し、コンパイルしない実行と同じ丸めを残す。
逆伝播のグラフは最初の逆伝播時にコンパイルされるため、この設定を順伝播だけの文脈に限定しない。
fabric.setup(..., _reapply_compile=False)に渡し、コンパイルしたラッパーをDDPの内側に置く。
各順伝播はtorch._functorch.config.patch(backward_pass_autocast="off")の中で呼び、逆伝播をautocastの外で行う計算としてコンパイルする。
すべての学習入力はm局面で形を固定し、端数のミクロバッチは作らない。
入力面の符号化、損失の計算、および勾配のクリップはコンパイルしない。
最終診断とONNXの書き出しもコンパイルせず、DDPとラッパーを通さない元のZeroNetを使う。
各ミクロバッチの平均損失にN/Kを掛けて逆伝播し、同期後に勾配のクリップと更新を1回行う。
損失の記録は各ミクロバッチの平均損失×mをfloat32で累計し、all_reduceのsumで全ランクの和を取る。
バッチ正規化（BatchNorm）の移動統計は、PyTorchの既定どおり各ミクロバッチの順伝播で更新する。
DDPのbroadcast_buffersは既定のTrueとし、ランク0の統計をほかのランクへ配る。
保存する統計はGPUの枚数で変わり、ミクロバッチ4つの1更新につき、GPU 1枚なら4回、4枚ならランク0の1回だけ更新される。
保存と最終診断はランク0だけが行い、元のZeroNetの重みと最適化器の状態を保存する。
全ランクが分散プロセス群を解放してから、ランク0だけが最終診断とONNX出力を行う。
診断はDDPを通さないZeroNetで行い、バッチの大きさにはmを使う。
再開は完了した更新の回数kから先頭のkB個を飛ばす。
保存したファイルの列を読み直して抽出を作り直し、各ファイルの大きさ、訓練局面数、検証局面数、latest_data、L、T、およびsample_sha256を照合する。
ファイルのSHA-256は初回に来歴として計算し、再開時は計算し直さない。
data_order_sha256は、初期値を32バイトの0とし、更新のたびに「直前の値の32バイト」と「その更新に使った抽出前の通し番号sの部分列をリトルエンディアンのint64で並べたバイト列」をつないだSHA-256で置き換えた値の16進表記である。
同じリプレイバッファ・シード・Bなら、ランク数と中断の有無にかかわらず同じ値になる。

学習率はlr * min(1, (optimizer_steps+1)/250)とし、最初の250更新で線形に上げる。
optimizer_stepsはランダム初期化からの累計更新数であり、更新の前の値を使う。
initでは0とし、各更新で1を足し、--initと--resumeで引き継ぐ。
サイクル内のcompleted_stepsとは別に数え、サイクルが変わっても0へ戻さない。
基準の学習率lrはサイクルの中では一定とし、configとsummary.jsonに記録する。
再開時も--lrを必須とし、チェックポイントのconfigの値と一致しなければエラーにする。
mはconfigから読み、Nと再生スレッド数は再開の照合に含めない。

checkpoint.ptは、torch.load(weights_only=True)で読める辞書であり、重み（BatchNormの移動統計を含む）、最適化器の状態、完了した更新の回数、全更新回数、サイクル番号、シード、設定、来歴、抽出条件とsample_sha256、累計の損失、学習曲線、optimizer_steps、optimizer_steps_start、およびdata_order_sha256を持つ。
initのチェックポイントは最適化器の状態を持たず、完了した更新の回数と全更新回数をともに0とする。

trainのsummary.jsonの欄は次のとおりである。

| 欄 | 意味 |
|---|---|
| cycle | サイクル番号。 |
| config | ネットの大きさと、新しいサイクルに指定した設定。出力先と再生スレッド数は含めない。 |
| lr | 基準の学習率。configにも記録する。 |
| provenance | 学習器と環境定義のコミット、実行環境、初期重みと教師ファイルの来歴、および開始時のworld_size。 |
| latest_data | 最新サイクルの生成ファイルの列。以下の抽出の4欄とともにconfigにも記録する。 |
| sample_count | 端数を含む抽出列の長さS=L。 |
| replay_buffer_training_count | リプレイバッファ全体の抽出前の訓練局面数T。 |
| file_training_counts | ファイルごとの抽出前の訓練局面数。dataの順に並ぶ。 |
| sample_sha256 | 抽出した通し番号を更新順に並べた列sのSHA-256。 |
| training_count / validation_count | sの先頭M個の通し番号から重複を除いた数と、最新サイクルのファイルの検証局面数の和。 |
| completed_steps / total_steps | 完了した更新回数と、このサイクルの全更新回数floor(S/B)。 |
| positions_seen | B×completed_steps。 |
| optimizer_steps_start / optimizer_steps | サイクル開始時と終了時の、初期化からの累計更新数。 |
| last_lr | 最後の更新に使った学習率。 |
| training_losses | 更新中の重み・訓練モードで局面数を重みとして平均したLOSS_NAMESの各項。 |
| diagnostics.training_sample | 端数を除いた抽出列の訓練局面のうち標本の対局に属するものを、最終の重みの推論モードで診断した値。 |
| diagnostics.latest_training_sample | training_sampleのうちlatest_dataに属する局面を、同じ関数で診断した値。 |
| diagnostics.validation | latest_dataの検証局面を、同じ関数で診断した値。 |
| diagnosticsの各集合のgame_count / position_count | 診断に含めた対局数と採用局面数。 |
| diagnosticsの各集合のhash_modulus / hash_remainder | hash64(seed, game)の法20と剰余。訓練標本は1、検証は0。 |
| diagnosticsの各集合のgames | 採用局面を持つ対局のseed（ヘッダの基本シード）とgame（対局番号）の列。 |
| diagnosticsの各集合の損失 | LOSS_NAMESの各項の局面単位の算術平均。 |
| diagnosticsの各集合のlegal_probability_mass / log_legal_probability_mass | 全合法手に乗る確率質量の平均と、その自然対数の平均。 |
| diagnosticsの各集合のpolicy_agreement | 合法手の最大logitの手が、教師の最大確率の手の集合に含まれる割合。 |
| learning_curve | 保存間隔ごとのcompleted_steps、training_losses、および区間の最後の更新で使ったlrとoptimizer_steps、サイクル開始時のoptimizer_steps_start。 |
| data_order_sha256 | 更新に使った局面の通し番号を連鎖させたSHA-256。 |
| network_sha256 | model.onnxのSHA-256。 |
| elapsed_seconds | loadingは全ランクの最大秒数、trainingとdiagnosticsはランク0の秒数。 |
| loading_peak_rss_bytes | 各ランクの読み込み直後までの最大常駐メモリの和。resource.getrusageで測り、バイトへ換算する。 |
| positions_per_second | positions_seenをtrainingの秒数で割った値。 |

policyは定数マスクが真となる16,108ラベルで正規化した交差エントロピー、valueは結果と根の価値を混ぜた教師に対する二値交差エントロピー、value_resultは結果だけに対する同じ損失である。
価値の教師は (1−λ)·結果 + λ·根の価値 であり、λは--value-lambdaで与える。
更新する損失は方策と価値の和とし、どちらの重みも1とする。
最適化器はネステロフのモーメンタムつきSGDとする。
--precision bf16では、ネットの順伝播だけをbf16のautocastで計算する。
損失の計算と累計はfloat32で行う。
訓練集合、2つの訓練標本、検証集合のいずれかが空なら、平均を定義できないためエラーにする。
シードはuint64、サイクルは0以上の整数とする。

loadingは教師の読み込みと初回の検査和計算、trainingはバッチの準備と更新、diagnosticsは3集合のバッチの準備と推論を測る。
保存とONNX出力は含めない。
loading_peak_rss_bytesは読み込みだけの増分ではなく、各プロセスの開始後の最大値を全ランクで足した値である。
再開時は保存済みの値と今回の全ランクの和の大きい方を記録する。
再開時は保存済みの時間に今回の読み込みと更新の時間を加え、中断で失われた更新や停止中の時間は含めない。
再開に指定した機器は実行に使い、configとprovenanceにはサイクル開始時の設定と環境を保持する。

診断ではeval()とinference_mode()を使い、勾配・バッチ正規化の移動統計・乱数状態を変えない。
合法手の確率質量は、定数マスク内の全ラベルを分母、反復の履歴を考えない全合法手を分子とする。
最大値を引いて単精度で計算し、対数は質量の指数化前に求める。
方策の一致率は、ネット側の同率なら最小ラベルを選び、教師側の同率はすべて正解とする。
最後のチェックポイントで更新済みの重みから再開する場合も、3集合の診断をやり直す。

initのsummary.jsonはblocks、channels、seed、parameters、network_sha256、provenanceだけを持つ。
チェックポイントの内部形式は本モジュールだけで扱う。
"""

from collections.abc import Callable, Iterator
from contextlib import contextmanager, nullcontext
import copy
from dataclasses import asdict
from datetime import timedelta
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
from typing import NamedTuple

from lightning_fabric import Fabric
from lightning_fabric.strategies import DDPStrategy
import numpy as np
from numpy.typing import NDArray
import torch
import torch._functorch.config
import torch._inductor.config
from torch.nn import functional as F

from .data import Dataset, FileInfo, read_replay_buffer
from .encoding import encode_planes, policy_mask
from .fixture import evaluate_fixture
from .net import TrainingLogits, ZeroNet, export_onnx


LOSS_NAMES = ("policy", "value", "value_result")
POLICY_DIAGNOSTIC_NAMES = ("legal_probability_mass", "log_legal_probability_mass", "policy_agreement")
DIAGNOSTIC_NAMES = (*LOSS_NAMES, *POLICY_DIAGNOSTIC_NAMES)
# lczero-training ee626a9のtf/tfprocess.pyの線形ウォームアップの式と、
# tf/configs/example.yamlのwarmup_stepsに倣う。
WARMUP_STEPS = 250


class PolicyTargets(NamedTuple):
    """バッチ内の候補の行番号、ラベル、および0〜65,535の確率。"""

    rows: torch.Tensor
    labels: torch.Tensor
    probabilities: torch.Tensor


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
    for name in ("batch_size", "micro_batch_size"):
        if type(config[name]) is not int or config[name] <= 0:
            raise ValueError(f"{name} must be a positive integer")
    if config["checkpoint_steps"] <= 0:
        raise ValueError("checkpoint_steps must be positive")
    if config["cycle"] < 0:
        raise ValueError("cycle must be nonnegative")
    for name in ("lr", "momentum", "clip_grad_norm"):
        if not math.isfinite(config[name]) or config[name] <= 0:
            raise ValueError(f"{name} must be finite and positive")
    if not math.isfinite(config["weight_decay"]) or config["weight_decay"] < 0:
        raise ValueError("weight_decay must be finite and nonnegative")
    if not math.isfinite(config["value_lambda"]) or not 0 <= config["value_lambda"] <= 1:
        raise ValueError("value_lambda must be between 0 and 1")


@contextmanager
def training_fabric(device_name: str, precision: str) -> Iterator[Fabric]:
    """torchrunの単一機械の環境を検査し、Fabricの分散処理を開始して回収する。"""
    names = ("WORLD_SIZE", "LOCAL_WORLD_SIZE", "RANK", "LOCAL_RANK")
    missing = [name for name in names if name not in os.environ]
    if missing:
        raise ValueError("train must run under torchrun; missing " + ", ".join(missing))
    try:
        world_size, local_world_size, rank, local_rank = (int(os.environ[name]) for name in names)
    except ValueError as error:
        raise ValueError("torchrun rank and world size variables must be integers") from error
    if world_size != local_world_size:
        raise ValueError("torchrun WORLD_SIZE must equal LOCAL_WORLD_SIZE; only one node is supported")
    if world_size < 1 or not 0 <= rank < world_size or local_rank != rank:
        raise ValueError("invalid single-node torchrun ranks or world size")
    _validate_device(device_name, precision)
    fabric = Fabric(
        accelerator=device_name, devices=world_size, num_nodes=1,
        strategy=DDPStrategy(
            process_group_backend="nccl" if device_name == "cuda" else "gloo",
            # バケットの組み替えで再開直後の更新の加算順が変わり、GPU 4枚での再開後の重みが食い違ったと考えられる。
            # find_unused_parameters=Trueで組み替えを止める。
            # RTX 4090 4枚で189回の更新を学ぶ再開の確認では、この設定で一致し、学習時間の差は約2%だった。
            find_unused_parameters=True, timeout=timedelta(hours=1),
        ),
        precision="32-true",
    )
    fabric.launch()
    # 例外時は集団通信の破棄で待たずにランクを終了させ、torchrunにほかのランクを止めさせる。
    yield fabric
    torch.distributed.destroy_process_group()


def _validate_batches(config: dict, world_size: int, replay_threads: int) -> None:
    _validate_training(config)
    if type(replay_threads) is not int or replay_threads < 1:
        raise ValueError("replay-threads must be a positive integer")
    batch_size, micro_batch_size = config["batch_size"], config["micro_batch_size"]
    if batch_size % micro_batch_size:
        raise ValueError("batch_size must be a multiple of micro_batch_size")
    if (batch_size // micro_batch_size) % world_size:
        raise ValueError("microbatch count K must be a multiple of WORLD_SIZE")


class TrainingModule(torch.nn.Module):
    """学習用の全logitを、分散データ並列のforwardから計算する。"""

    def __init__(self, network: ZeroNet) -> None:
        super().__init__()
        self.network = network

    def forward(self, inputs: torch.Tensor) -> TrainingLogits:
        return self.network.forward_logits(inputs)


def compile_training(network: ZeroNet) -> torch.nn.Module:
    """学習用の全logitを返す順伝播を、既定モードの単一グラフにコンパイルする。

    実行時の計測で選ぶカーネル構成がプロセスごとに変わり再開前後の学習が食い違ったため、deterministicをプロセス全体に設定する。
    RTX 3060 Tiでの再開試験では、この設定で出力が一致し、速度は毎秒990局面のままだった。
    """
    # bf16でもコンパイルしない実行と同じ丸めを残す。
    torch._inductor.config.emulate_precision_casts = True
    # 実行時の計測によるカーネル構成の変動を避け、再開前後の計算をそろえる。
    torch._inductor.config.deterministic = True
    # Dynamoの状態復元はdeterministicを書き換えるため、両設定をオプションにも固定する。
    # 順伝播と遅延コンパイルされる逆伝播で、プロセス全体に指定した同じ設定を使う。
    return torch.compile(
        TrainingModule(network), fullgraph=True,
        options={"deterministic": True, "emulate_precision_casts": True},
    )


@contextmanager
def compiled_forward_context() -> Iterator[None]:
    """逆伝播をautocastの外で行う前提を、順伝播の呼び出し中に保つ。"""
    with torch._functorch.config.patch(backward_pass_autocast="off"):
        yield


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
        "optimizer_steps": 0,
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
    方策の損失は全候補の損失を合計して局面数で割り、GPUの原子的な加算を使わない。
    マスク外の教師ラベルはValueErrorとし、結果だけの監視用損失には勾配を持たせない。
    """
    policy_logits, value_logits = policy_logits.float(), value_logits.float().flatten()
    probabilities, results, values = probabilities.float(), results.float(), values.float()
    if torch.any((labels < 0) | (labels >= len(mask))) or not torch.all(mask[labels]):
        raise ValueError("policy target label is outside the constant mask")
    policy_logits = policy_logits.masked_fill(~mask, -torch.inf)
    candidate_losses = -F.log_softmax(policy_logits, dim=1)[rows, labels] * (probabilities / 65_535)
    policy_loss = candidate_losses.sum() / len(results)
    target = (1 - value_lambda) * results + value_lambda * (values / 65_535)
    value_loss = F.binary_cross_entropy_with_logits(value_logits, target)
    with torch.no_grad():
        result_loss = F.binary_cross_entropy_with_logits(value_logits, results)
    return policy_loss, value_loss, result_loss


def compute_training_losses(
    logits: TrainingLogits, policy: PolicyTargets, results: torch.Tensor, values: torch.Tensor,
    value_lambda: float, mask: torch.Tensor,
) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
    """方策、価値、および監視用の結果損失をLOSS_NAMESの順で返す。"""
    return compute_losses(logits.policy, logits.value, policy.rows, policy.labels,
                          policy.probabilities, results, values, value_lambda, mask)


def total_loss(losses: tuple[torch.Tensor, torch.Tensor, torch.Tensor]) -> torch.Tensor:
    """方策と価値をそれぞれ重み1で足し、更新用の損失を返す。"""
    policy, value, _ = losses
    return policy + value


def learning_rate(base: float, optimizer_steps: int) -> float:
    """ランダム初期化から250更新で基準の学習率まで線形に上げ、その後は保つ。"""
    return base * min(1.0, (optimizer_steps + 1) / WARMUP_STEPS)


def _sparse_indices(
    offsets: NDArray[np.uint64], indices: NDArray[np.int64],
) -> tuple[NDArray[np.int64], NDArray[np.int64]]:
    starts, ends = offsets[indices], offsets[indices + 1]
    entries = np.concatenate([np.arange(start, end, dtype=np.int64) for start, end in zip(starts, ends)])
    rows = np.repeat(np.arange(len(indices), dtype=np.int64), (ends - starts).astype(np.int64))
    return rows, entries


def _policy_batch(
    data: Dataset, indices: NDArray[np.int64], device: torch.device,
) -> PolicyTargets:
    rows, candidates = _sparse_indices(data.policy_offsets, indices)
    labels = data.policy_labels[candidates].astype(np.int64)
    probabilities = data.policy_probabilities[candidates].astype(np.float32)
    return PolicyTargets(*(torch.from_numpy(column).to(device) for column in (rows, labels, probabilities)))


def _legal_batch(
    data: Dataset, indices: NDArray[np.int64], device: torch.device,
) -> tuple[torch.Tensor, torch.Tensor]:
    """診断に使う全合法手の行番号とラベルを、バッチの順に組み立てる。"""
    rows, legal = _sparse_indices(data.legal_offsets, indices)
    labels = data.legal_labels[legal].astype(np.int64)
    return torch.from_numpy(rows).to(device), torch.from_numpy(labels).to(device)


def _batch(
    forward: Callable[[torch.Tensor], TrainingLogits], data: Dataset, indices: NDArray[np.int64],
    device: torch.device, config: dict,
) -> tuple[TrainingLogits, PolicyTargets, torch.Tensor, torch.Tensor]:
    """順伝播の出力、疎な方策、対局結果、および根の価値を返す。"""
    boards, sides, lions = (
        torch.from_numpy(array[indices]).to(device) for array in (data.boards, data.sides, data.lions)
    )
    inputs = encode_planes(boards, sides, lions)
    policy = _policy_batch(data, indices, device)
    results = torch.from_numpy(data.results[indices]).to(device)
    values = torch.from_numpy(data.values[indices].astype(np.float32)).to(device)
    with torch.autocast("cuda", dtype=torch.bfloat16) if config["precision"] == "bf16" else nullcontext():
        logits = forward(inputs)
    return logits, policy, results, values


def compute_policy_diagnostics(
    policy_logits: torch.Tensor, rows: torch.Tensor, labels: torch.Tensor, probabilities: torch.Tensor,
    legal_rows: torch.Tensor, legal_labels: torch.Tensor, mask: torch.Tensor,
) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
    """合法手の質量・その自然対数・教師との一致率を、局面単位で平均する。

    legal_rowsとlegal_labelsは各局面の全合法手を重複なく表す。
    ネットの最大logitが同率なら最小ラベルを選び、教師の同率はすべて正解とする。
    マスク外の合法手、合法手がない局面、有限でない合法手の最大logitはValueErrorとする。
    """
    log_mass, agreement = _policy_diagnostics_per_position(
        policy_logits, rows, labels, probabilities, legal_rows, legal_labels, mask,
    )
    return log_mass.exp().mean(), log_mass.mean(), agreement.mean()


def _policy_diagnostics_per_position(
    policy_logits: torch.Tensor, rows: torch.Tensor, labels: torch.Tensor, probabilities: torch.Tensor,
    legal_rows: torch.Tensor, legal_labels: torch.Tensor, mask: torch.Tensor,
) -> tuple[torch.Tensor, torch.Tensor]:
    """合法手の質量の自然対数と教師との一致を、局面ごとのfloat32で返す。

    一致は合法手の最大logitのうち最小のラベルと、最大確率の教師候補のいずれかが
    等しければ1、そうでなければ0とする。検査の条件はcompute_policy_diagnosticsと同じ。
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
    # GPUの原子的な加算を避け、CPUで入力順に加算して診断値を再現する。
    legal_sum = torch.zeros(count, dtype=torch.float32, device="cpu").index_add_(
        0, legal_rows.cpu(), (legal_logits - legal_max[legal_rows]).exp().cpu(),
    ).to(logits.device)
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
    return log_mass, agreement.float()


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
        tensors = _batch(model.forward_logits, data, batch, device, config)
        logits, policy, _, _ = tensors
        losses = compute_training_losses(*tensors, config["value_lambda"], mask)
        legal_rows, legal_labels = _legal_batch(data, batch, device)
        metrics = compute_policy_diagnostics(logits.policy, policy.rows, policy.labels, policy.probabilities,
                                             legal_rows, legal_labels, mask)
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


def _counts(
    data: Dataset, infos: list[FileInfo], selected: NDArray[np.int64], config: dict, rank: int,
) -> tuple[int, int]:
    latest_start = len(infos) - len(config["latest_data"])
    validation = sum(info.validation_count for info in infos[latest_start:])
    used = len(selected) // config["batch_size"] * config["batch_size"]
    training = len(np.unique(selected[:used]))
    if training == 0 or validation == 0:
        raise ValueError("training and validation sets must both contain positions")
    if rank == 0 and not np.any(data.training_sample):
        raise ValueError("training diagnostic sample must contain positions")
    if rank == 0 and not np.any(data.training_sample & (data.file_indices >= latest_start)):
        raise ValueError("latest training diagnostic sample must contain positions")
    return training, validation


def _sampling_metadata(config: dict, infos: list[FileInfo], selected: NDArray[np.int64]) -> dict:
    return {
        "latest_data": config["latest_data"],
        "sample_count": len(selected),
        "replay_buffer_training_count": sum(info.training_count for info in infos),
        "file_training_counts": [info.training_count for info in infos],
        "sample_sha256": hashlib.sha256(selected.astype("<i8").tobytes()).hexdigest(),
    }


def _peak_rss_bytes() -> int:
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return peak if sys.platform == "darwin" else peak * 1024


def _read_data_list(path: Path) -> list[str]:
    try:
        files = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{path}: cannot read data list: {error}") from error
    if not isinstance(files, list) or not files or any(not isinstance(name, str) or not name for name in files):
        raise ValueError(f"{path}: data list must be a nonempty JSON array of nonempty path strings")
    return files


def _read_training_data(
    config: dict, fabric: Fabric, replay_threads: int,
) -> tuple[Dataset, list[FileInfo], NDArray[np.int64], NDArray[np.int64]]:
    return read_replay_buffer(
        config["data"], config["latest_data"], config["seed"],
        rank=fabric.global_rank, world_size=fabric.world_size,
        batch_size=config["batch_size"], micro_batch_size=config["micro_batch_size"],
        replay_threads=replay_threads,
    )


def _loading_measurements(fabric: Fabric, started: float, peak_rss: int) -> tuple[float, int]:
    seconds = torch.tensor(time.perf_counter() - started, dtype=torch.float64, device=fabric.device)
    memory = torch.tensor(peak_rss, dtype=torch.int64, device=fabric.device)
    return (fabric.all_reduce(seconds, reduce_op="max").item(),
            fabric.all_reduce(memory, reduce_op="sum").item())


def train_new(config: dict, out_dir: Path) -> None:
    """完了したチェックポイントから、指定したリプレイバッファで新しいサイクルを始める。"""
    config = dict(config)
    replay_threads = config.pop("replay_threads")
    with training_fabric(config["device"], config["precision"]) as fabric:
        _validate_batches(config, fabric.world_size, replay_threads)
        config["data"] = _read_data_list(Path(config.pop("data_list")))
        initial = _load_checkpoint(Path(config["init"]))
        if initial["completed_steps"] != initial["total_steps"]:
            raise ValueError("init checkpoint has unfinished updates")
        if fabric.global_rank == 0:
            _prepare_output(out_dir)
        config = {**config, "blocks": initial["config"]["blocks"], "channels": initial["config"]["channels"]}
        started = time.perf_counter()
        data, infos, selected, retained = _read_training_data(config, fabric, replay_threads)
        loading_peak_rss = _peak_rss_bytes()
        training_count, validation_count = _counts(data, infos, selected, config, fabric.global_rank)
        sampling = _sampling_metadata(config, infos, selected)
        config.update(sampling)
        if fabric.global_rank == 0:
            provenance = _environment(config["trainer_commit"], fabric.device)
            provenance["world_size"] = fabric.world_size
            provenance["init_sha256"] = _sha256(Path(config["init"]))
            provenance["files"] = [_file_provenance(path, info) for path, info in zip(config["data"], infos)]
        loading_seconds, loading_peak_rss = _loading_measurements(fabric, started, loading_peak_rss)
        state = {
            "model": initial["model"], "optimizer": initial["optimizer"],
            "completed_steps": 0, "total_steps": len(selected) // config["batch_size"],
            "cycle": config["cycle"], "seed": config["seed"], "config": config,
            "training_count": training_count, "validation_count": validation_count,
            **sampling,
            "positions_seen": 0, "loss_sums": _zero_losses(), "learning_curve": [],
            "optimizer_steps": initial["optimizer_steps"], "optimizer_steps_start": initial["optimizer_steps"],
            "data_order_sha256": bytes(32).hex(),
            "elapsed_seconds": {"loading": loading_seconds, "training": 0.0},
            "loading_peak_rss_bytes": loading_peak_rss,
        }
        if fabric.global_rank == 0:
            state["provenance"] = provenance
        network = _run(state, data, selected, retained, out_dir, fabric)
    if fabric.global_rank == 0:
        _finish(state, data, network, out_dir, fabric.device)


def resume(out_dir: Path, device_name: str, replay_threads: int, lr: float) -> None:
    """基準学習率、ファイル情報、抽出条件を照合し、最後に確定した更新から再開する。"""
    if (out_dir / "summary.json").exists():
        raise ValueError(f"training is already complete: {out_dir}")
    state = _load_checkpoint(out_dir / "checkpoint.pt")
    if state["optimizer"] is None:
        raise ValueError("resume requires a training checkpoint")
    config = state["config"]
    if lr != config["lr"]:
        raise ValueError(f"resume lr {lr} differs from checkpoint lr {config['lr']}")
    with training_fabric(device_name, config["precision"]) as fabric:
        _validate_batches(config, fabric.world_size, replay_threads)
        data, network = _resume(state, out_dir, fabric, replay_threads)
    if fabric.global_rank == 0:
        _finish(state, data, network, out_dir, fabric.device)


def _resume(state: dict, out_dir: Path, fabric: Fabric, replay_threads: int) -> tuple[Dataset, ZeroNet]:
    config = state["config"]
    records = state["provenance"]["files"]
    paths = [record["path"] for record in records]
    if config["data"] != paths:
        raise ValueError("replay buffer path sequence differs from provenance")
    started = time.perf_counter()
    data, infos, selected, retained = _read_training_data(config, fabric, replay_threads)
    peak_rss = _peak_rss_bytes()
    for record, info in zip(records, infos):
        if (record["size"], record["training_count"], record["validation_count"]) != (
            info.size, info.training_count, info.validation_count,
        ):
            raise ValueError(f"replay buffer size or position counts differ: {record['path']}")
    for key, value in _sampling_metadata(config, infos, selected).items():
        if state[key] != value or config[key] != value:
            raise ValueError(f"replay buffer sampling condition differs: {key}")
    if _counts(data, infos, selected, config, fabric.global_rank) != (state["training_count"], state["validation_count"]):
        raise ValueError("replay buffer total position counts differ")
    batch_size = config["batch_size"]
    if state["total_steps"] != state["sample_count"] // batch_size:
        raise ValueError("checkpoint total_steps disagrees with sample_count and batch_size")
    if state["positions_seen"] != state["completed_steps"] * batch_size:
        raise ValueError("checkpoint positions_seen disagrees with completed_steps")
    loading_seconds, peak_rss = _loading_measurements(fabric, started, peak_rss)
    state["loading_peak_rss_bytes"] = max(state["loading_peak_rss_bytes"], peak_rss)
    state["elapsed_seconds"]["loading"] += loading_seconds
    if fabric.global_rank == 0:
        print(f"resume: completed_steps={state['completed_steps']}/{state['total_steps']}", file=sys.stderr, flush=True)
    return data, _run(state, data, selected, retained, out_dir, fabric)


def accumulate_gradients(
    fabric: Fabric, model: torch.nn.Module, optimizer: torch.optim.Optimizer,
    data: Dataset, batches: NDArray[np.int64], config: dict,
    mask: torch.Tensor, step: int,
) -> torch.Tensor:
    """1更新分の勾配を同期し、クリップ前の勾配と全局面の損失の和を得る。

    batchesは、このランクに割り当てられたミクロバッチを番号順に並べた2次元配列である。
    バッチ正規化の移動統計は、各ミクロバッチの順伝播で更新する。
    """
    optimizer.zero_grad(set_to_none=True)
    sums = torch.zeros(len(LOSS_NAMES), dtype=torch.float32, device=fabric.device)
    micro_batch_size = config["micro_batch_size"]
    scale = fabric.world_size / (config["batch_size"] // micro_batch_size)
    for index, batch in enumerate(batches):
        with fabric.no_backward_sync(model, enabled=index + 1 < len(batches)):
            with compiled_forward_context():
                tensors = _batch(model, data, batch, fabric.device, config)
            losses = compute_training_losses(*tensors, config["value_lambda"], mask)
            if not all(torch.isfinite(loss).item() for loss in losses):
                raise ValueError(f"non-finite training loss at update {step + 1}, rank {fabric.global_rank}")
            fabric.backward(total_loss(losses) * scale)
        with torch.no_grad():
            sums.add_(torch.stack([loss.detach() for loss in losses]) * micro_batch_size)
    return fabric.all_reduce(sums, reduce_op="sum")


def _run(
    state: dict, data: Dataset, selected: NDArray[np.int64], retained: NDArray[np.int64],
    out_dir: Path, fabric: Fabric,
) -> ZeroNet:
    config = state["config"]
    device = fabric.device
    network = _model(state).train()
    mask = torch.from_numpy(policy_mask()).to(device)
    optimizer = torch.optim.SGD(
        network.parameters(), lr=config["lr"], momentum=config["momentum"],
        weight_decay=config["weight_decay"], nesterov=True,
    )
    compiled = compile_training(network)
    # _reapply_compileは下線で始まる引数なので、Fabric 2.6.6から版を上げる際は
    # コンパイルしたラッパーがDDPの内側に置かれることを確かめ直す。
    model, optimizer = fabric.setup(compiled, optimizer, _reapply_compile=False)
    # load_state_dictは現在のパラメータの機器へモーメンタムも移す。
    if state["optimizer"] is not None:
        optimizer.load_state_dict(state["optimizer"])
        for group in optimizer.param_groups:
            group.update(lr=config["lr"], momentum=config["momentum"], weight_decay=config["weight_decay"],
                         dampening=0, nesterov=True)
    indices = np.flatnonzero(~data.validation)
    batch_size = config["batch_size"]
    micro_batch_size = config["micro_batch_size"]
    assigned = selected[:state["total_steps"] * batch_size].reshape(
        state["total_steps"], batch_size // micro_batch_size, micro_batch_size,
    )[:, fabric.global_rank::fabric.world_size]
    order = indices[np.searchsorted(retained, assigned)]
    interval_sums, interval_count = _zero_losses(), 0
    interval_started = time.perf_counter()
    for step in range(state["completed_steps"], state["total_steps"]):
        losses = accumulate_gradients(
            fabric, model, optimizer, data, order[step], config, mask, step,
        )
        torch.nn.utils.clip_grad_norm_(model.parameters(), config["clip_grad_norm"], error_if_nonfinite=True)
        lr = learning_rate(config["lr"], state["optimizer_steps"])
        for group in optimizer.param_groups:
            group["lr"] = lr
        optimizer.step()
        _accumulate(state["loss_sums"], losses, 1)
        _accumulate(interval_sums, losses, 1)
        state["positions_seen"] += batch_size
        state["optimizer_steps"] += 1
        interval_count += batch_size
        state["completed_steps"] = step + 1
        state["data_order_sha256"] = hashlib.sha256(
            bytes.fromhex(state["data_order_sha256"])
            + selected[step * batch_size:(step + 1) * batch_size].astype("<i8").tobytes()
        ).hexdigest()
        if (step + 1) % config["checkpoint_steps"] == 0 or step + 1 == state["total_steps"]:
            elapsed = time.perf_counter() - interval_started
            state["elapsed_seconds"]["training"] += elapsed
            averages = _mean(interval_sums, interval_count)
            state["learning_curve"].append({"completed_steps": step + 1, "training_losses": averages, "lr": lr,
                                            "optimizer_steps_start": state["optimizer_steps_start"],
                                            "optimizer_steps": state["optimizer_steps"]})
            if fabric.global_rank == 0:
                state["model"], state["optimizer"] = network.state_dict(), optimizer.state_dict()
                _save_checkpoint(out_dir / "checkpoint.pt", state)
                print(
                    f"steps={step + 1}/{state['total_steps']} "
                    + " ".join(f"{name}={averages[name]:.6f}" for name in LOSS_NAMES)
                    + f" positions_per_second={interval_count / elapsed:.2f}",
                    file=sys.stderr, flush=True,
                )
            interval_sums, interval_count = _zero_losses(), 0
            interval_started = time.perf_counter()

    return network


def _finish(state: dict, data: Dataset, network: ZeroNet, out_dir: Path, device: torch.device) -> None:
    """分散処理の終了後に、ランク0の元のネットで最終診断と出力を行う。"""
    config = state["config"]
    mask = torch.from_numpy(policy_mask()).to(device)
    network.eval()
    started = time.perf_counter()
    diagnostics = {}
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
        diagnostics[name] = {**metadata, **diagnose(
            network, data, indices, device, {**config, "batch_size": config["micro_batch_size"]}, mask,
        )}
    diagnostic_seconds = time.perf_counter() - started
    with _atomic_path(out_dir / "model.onnx") as temporary:
        export_onnx(copy.deepcopy(network).cpu().float().eval(), temporary)
    _write_json(out_dir / "summary.json", {
        "cycle": state["cycle"], "config": config, "provenance": state["provenance"],
        "lr": config["lr"],
        "training_count": state["training_count"], "validation_count": state["validation_count"],
        **{key: state[key] for key in ("latest_data", "sample_count", "replay_buffer_training_count",
                                      "file_training_counts", "sample_sha256")},
        "completed_steps": state["completed_steps"], "total_steps": state["total_steps"],
        "positions_seen": state["positions_seen"],
        "optimizer_steps_start": state["optimizer_steps_start"], "optimizer_steps": state["optimizer_steps"],
        "last_lr": state["learning_curve"][-1]["lr"],
        "training_losses": _mean(state["loss_sums"], state["positions_seen"]),
        "diagnostics": diagnostics, "learning_curve": state["learning_curve"],
        "data_order_sha256": state["data_order_sha256"], "network_sha256": _sha256(out_dir / "model.onnx"),
        "elapsed_seconds": {**state["elapsed_seconds"], "diagnostics": diagnostic_seconds},
        "loading_peak_rss_bytes": state["loading_peak_rss_bytes"],
        "positions_per_second": state["positions_seen"] / state["elapsed_seconds"]["training"],
    })


def export_checkpoint(checkpoint: Path, out: Path) -> dict[str, str]:
    """チェックポイントからONNXだけを書き出し、確定したファイルの検査和を返す。"""
    if out.exists() or out.is_symlink():
        raise ValueError(f"output already exists: {out}")
    state = _load_checkpoint(checkpoint)
    model = _model(state).eval()
    with _atomic_path(out) as temporary:
        export_onnx(model, temporary)
    return {"network_sha256": _sha256(out)}


def fixture_eval(checkpoint: Path, onnx: Path, fixture: Path, out: Path) -> None:
    """チェックポイントの重みとONNXの一致を確かめ、固定局面集をCPUで推論する。"""
    state = _load_checkpoint(checkpoint)
    model = _model(state).eval()
    onnx_bytes = onnx.read_bytes()
    with tempfile.TemporaryDirectory(prefix="minase-zero-fixture-") as directory:
        path = Path(directory) / "model.onnx"
        export_onnx(model, path)
        if path.read_bytes() != onnx_bytes:
            raise ValueError("checkpoint ONNX bytes differ from --onnx; weights are not identical")
    _write_json(out, evaluate_fixture(model, fixture, onnx_bytes))
