"""MNSDからP型NNUEを学習し、浮動小数点重みとMNUEを生成する。"""

from __future__ import annotations

import argparse
import math
from pathlib import Path
from typing import NamedTuple, Sequence

import numpy as np
from numpy.typing import NDArray
import torch
from torch import Tensor, nn
from torch.nn import functional as torch_functional

from features import FEATURE_COUNT, INITIAL_BOARD, PADDING_INDEX, feature_indices, mirror
from mnsd import NO_LION_SQUARE, load_records
from nnue_net import (
    HIDDEN1_WIDTH,
    HIDDEN2_WIDTH,
    HIDDEN3_WIDTH,
    Parameters,
    evaluate,
    write_mnue,
)


# 第2層以降の重みの訓練時範囲。第1層はi16の余裕を使って広く取る。
WEIGHT_LIMIT = 1.98
WEIGHT1_LIMIT = 8.0
INPUT2_WIDTH = HIDDEN1_WIDTH * 2
WEIGHT1_SCALE = 127
WEIGHT_SCALE = 4096
ACTIVATION_SCALE = 127
BIAS_SCALE = ACTIVATION_SCALE * WEIGHT_SCALE
# 量子化誤差の合格条件はロジット単位で0.025（センチポーンではK/40）。第2層以降を
# 8ビットで量子化する設計では、ロジット0.01程度の丸め雑音が避けられないため、
# 学習PSTの2センチポーンではなくKに相対的な基準を使う（docs/plans/evaluation.md）。
QUANTIZATION_ERROR_LIMIT_LOGIT = 0.025


class FeatureTensors(NamedTuple):
    """通常局面と鏡映局面の両視点の特徴番号を保持する。"""

    stm: Tensor
    opp: Tensor
    mirrored_stm: Tensor
    mirrored_opp: Tensor


class NnueModel(nn.Module):
    """共有第1層と3個の全結合層からなるP型NNUEを表す。"""

    def __init__(self, device: torch.device) -> None:
        """PyTorchの既定初期値で各層を指定デバイス上に作る。"""
        super().__init__()
        self.embedding = nn.EmbeddingBag(
            FEATURE_COUNT + 1,
            HIDDEN1_WIDTH,
            mode="sum",
            padding_idx=PADDING_INDEX,
            device=device,
        )
        self.b1 = nn.Parameter(torch.empty(HIDDEN1_WIDTH, device=device))
        first_layer_bound = 1.0 / math.sqrt(FEATURE_COUNT)
        nn.init.uniform_(self.b1, -first_layer_bound, first_layer_bound)
        self.layer2 = nn.Linear(INPUT2_WIDTH, HIDDEN2_WIDTH, device=device)
        self.layer3 = nn.Linear(HIDDEN2_WIDTH, HIDDEN3_WIDTH, device=device)
        self.output = nn.Linear(HIDDEN3_WIDTH, 1, device=device)
        with torch.no_grad():
            self.embedding.weight[PADDING_INDEX].zero_()

    def forward(self, stm: Tensor, opp: Tensor) -> Tensor:
        """両視点の特徴番号から勝率ロジットを計算する。"""
        accumulator_stm = self.embedding(stm) + self.b1
        accumulator_opp = self.embedding(opp) + self.b1
        hidden = torch.clamp(
            torch.cat((accumulator_stm, accumulator_opp), dim=1), 0.0, 1.0
        )
        hidden = torch.clamp(self.layer2(hidden), 0.0, 1.0)
        hidden = torch.clamp(self.layer3(hidden), 0.0, 1.0)
        return self.output(hidden).squeeze(1)

    def clamp_weights(self) -> None:
        """量子化対象の重みを訓練時範囲へ制限し、padding行を0に戻す。"""
        with torch.no_grad():
            self.embedding.weight.clamp_(-WEIGHT1_LIMIT, WEIGHT1_LIMIT)
            self.b1.clamp_(-WEIGHT1_LIMIT, WEIGHT1_LIMIT)
            self.layer2.weight.clamp_(-WEIGHT_LIMIT, WEIGHT_LIMIT)
            self.layer3.weight.clamp_(-WEIGHT_LIMIT, WEIGHT_LIMIT)
            self.output.weight.clamp_(-WEIGHT_LIMIT, WEIGHT_LIMIT)
            self.embedding.weight[PADDING_INDEX].zero_()


def build_targets(
    records: np.ndarray, k: float, lambda_value: float
) -> NDArray[np.float32]:
    """探索値と最終結果を混合した教師勝率を作る。"""
    score_probability = 1.0 / (
        1.0 + np.exp(-records["score"].astype(np.float64) / k)
    )
    result_probability = records["result"].astype(np.float64) / 2.0
    return (
        lambda_value * score_probability
        + (1.0 - lambda_value) * result_probability
    ).astype(np.float32)


def make_model(device: torch.device, seed: int) -> NnueModel:
    """指定シードで初期化したモデルを作る。"""
    torch.manual_seed(seed)
    if device.type == "cuda":
        torch.cuda.manual_seed_all(seed)
    return NnueModel(device)


def validation_loss(
    model: NnueModel,
    features: FeatureTensors,
    targets: Tensor,
    indices: Tensor,
    batch: int,
) -> float:
    """検証集合全体の平均二値交差エントロピーを計算する。"""
    model.eval()
    total = 0.0
    with torch.no_grad():
        for start in range(0, indices.shape[0], batch):
            selected = indices[start : start + batch]
            loss = torch_functional.binary_cross_entropy_with_logits(
                model(features.stm[selected], features.opp[selected]),
                targets[selected],
                reduction="sum",
            )
            total += float(loss.item())
    return total / indices.shape[0]


def train_epoch(
    model: NnueModel,
    optimizer: torch.optim.Optimizer,
    features: FeatureTensors,
    targets: Tensor,
    training_indices: Tensor,
    batch: int,
    generator: torch.Generator,
) -> float:
    """鏡映を標本ごとに選び、訓練集合を1エポック学習する。"""
    model.train()
    permutation = torch.randperm(
        training_indices.shape[0], generator=generator, device=training_indices.device
    )
    order = training_indices[permutation]
    total = 0.0
    for start in range(0, order.shape[0], batch):
        selected = order[start : start + batch]
        choose_mirrored = torch.rand(
            selected.shape[0], generator=generator, device=selected.device
        ) < 0.5
        selected_stm = torch.where(
            choose_mirrored[:, None],
            features.mirrored_stm[selected],
            features.stm[selected],
        )
        selected_opp = torch.where(
            choose_mirrored[:, None],
            features.mirrored_opp[selected],
            features.opp[selected],
        )
        optimizer.zero_grad(set_to_none=True)
        loss = torch_functional.binary_cross_entropy_with_logits(
            model(selected_stm, selected_opp), targets[selected]
        )
        loss.backward()
        optimizer.step()
        model.clamp_weights()
        total += float(loss.item()) * selected.shape[0]
    return total / training_indices.shape[0]


def _quantize_array(
    name: str,
    values: NDArray[np.float32],
    scale: int,
    dtype: np.dtype,
) -> NDArray[np.generic]:
    """浮動小数点配列を有限性と整数範囲を検査して量子化する。"""
    if not np.all(np.isfinite(values)):
        raise ValueError(f"{name} contains a non-finite value")
    rounded = np.rint(values.astype(np.float64) * scale)
    limits = np.iinfo(dtype)
    if np.any((rounded < limits.min) | (rounded > limits.max)):
        raise OverflowError(f"{name} does not fit {dtype}")
    return rounded.astype(dtype)


def float_parameters(model: NnueModel) -> dict[str, NDArray[np.float32]]:
    """モデルから保存および量子化に使うfloat32パラメータを取り出す。"""
    return {
        "w1": model.embedding.weight[:FEATURE_COUNT].detach().cpu().numpy().astype(np.float32),
        "b1": model.b1.detach().cpu().numpy().astype(np.float32),
        "w2": model.layer2.weight.detach().cpu().numpy().astype(np.float32),
        "b2": model.layer2.bias.detach().cpu().numpy().astype(np.float32),
        "w3": model.layer3.weight.detach().cpu().numpy().astype(np.float32),
        "b3": model.layer3.bias.detach().cpu().numpy().astype(np.float32),
        "wo": model.output.weight[0].detach().cpu().numpy().astype(np.float32),
        "bo": model.output.bias.detach().cpu().numpy().astype(np.float32),
    }


def quantize(parameters: dict[str, NDArray[np.float32]]) -> Parameters:
    """浮動小数点モデルをMNUEの整数パラメータへ量子化する。"""
    return Parameters(
        _quantize_array("W1", parameters["w1"], WEIGHT1_SCALE, np.dtype(np.int16)),
        _quantize_array("b1", parameters["b1"], WEIGHT1_SCALE, np.dtype(np.int32)),
        _quantize_array("W2", parameters["w2"], WEIGHT_SCALE, np.dtype(np.int16)),
        _quantize_array("b2", parameters["b2"], BIAS_SCALE, np.dtype(np.int32)),
        _quantize_array("W3", parameters["w3"], WEIGHT_SCALE, np.dtype(np.int16)),
        _quantize_array("b3", parameters["b3"], BIAS_SCALE, np.dtype(np.int32)),
        _quantize_array("Wo", parameters["wo"], WEIGHT_SCALE, np.dtype(np.int16)),
        _quantize_array("bo", parameters["bo"], BIAS_SCALE, np.dtype(np.int32)),
    )


def save_float_parameters(
    path: str | Path,
    parameters: dict[str, NDArray[np.float32]],
    k: float,
) -> None:
    """浮動小数点パラメータと勝率尺度をNPZへ保存する。"""
    with Path(path).open("wb") as stream:
        np.savez(stream, **parameters, k=np.float32(k))


def build_feature_tensors(
    records: np.ndarray, device: torch.device
) -> FeatureTensors:
    """全局面の通常・鏡映特徴を両視点で前計算してデバイスへ置く。"""
    sides = records["stm"]
    opponents = 1 - sides
    mirrored_boards, mirrored_lions = mirror(records["board"], records["lion"])
    arrays = (
        feature_indices(records["board"], sides, records["lion"]),
        feature_indices(records["board"], opponents, records["lion"]),
        feature_indices(mirrored_boards, sides, mirrored_lions),
        feature_indices(mirrored_boards, opponents, mirrored_lions),
    )
    return FeatureTensors(*(torch.as_tensor(value, device=device) for value in arrays))


def floating_scores(
    model: NnueModel,
    features: FeatureTensors,
    indices: NDArray[np.int64],
    k: float,
    batch: int,
) -> NDArray[np.float32]:
    """指定局面の浮動小数点モデル出力をセンチポーンで返す。"""
    model.eval()
    chunks: list[NDArray[np.float32]] = []
    with torch.no_grad():
        for start in range(0, indices.size, batch):
            selected = torch.as_tensor(
                indices[start : start + batch], device=features.stm.device
            )
            logits = model(features.stm[selected], features.opp[selected])
            chunks.append((logits * k).cpu().numpy().astype(np.float32))
    return np.concatenate(chunks)


def check_quantization_error(
    model: NnueModel,
    params: Parameters,
    records: np.ndarray,
    features: FeatureTensors,
    validation_indices: NDArray[np.int64],
    k: float,
    validation_sample: int,
    batch: int,
    seed: int,
) -> tuple[float, float, int]:
    """検証標本で浮動小数点推論と整数推論の誤差を検査する。"""
    sample_count = min(validation_sample, validation_indices.size)
    random = np.random.default_rng(seed)
    sample_indices = random.choice(
        validation_indices, size=sample_count, replace=False
    ).astype(np.int64, copy=False)
    float_scores = floating_scores(model, features, sample_indices, k, batch)
    integer_scores = evaluate(
        params,
        k,
        records["board"][sample_indices],
        records["stm"][sample_indices],
        records["lion"][sample_indices],
    )
    errors = np.abs(float_scores.astype(np.float64) - integer_scores.astype(np.float64))
    mean_absolute_error = float(errors.mean())
    maximum_error = float(errors.max())
    if not math.isfinite(mean_absolute_error) or not math.isfinite(maximum_error):
        raise ValueError("quantization error is non-finite")
    return mean_absolute_error, maximum_error, sample_count


def initial_position_scores(params: Parameters, k: float) -> tuple[int, int]:
    """初期局面の手番先手・手番後手の整数評価値を返す。"""
    scores = evaluate(
        params,
        k,
        np.stack((INITIAL_BOARD, INITIAL_BOARD)),
        np.array([0, 1], dtype=np.uint8),
        np.array([NO_LION_SQUARE, NO_LION_SQUARE], dtype=np.uint8),
    )
    return int(scores[0]), int(scores[1])


def _validate_arguments(arguments: argparse.Namespace) -> torch.device:
    """学習引数を検査し、指定されたPyTorchデバイスを返す。"""
    if not 0.0 <= arguments.lambda_value <= 1.0:
        raise ValueError("--lambda must be in 0..1")
    if arguments.epochs <= 0 or arguments.batch <= 0:
        raise ValueError("--epochs and --batch must be positive")
    if arguments.validation_sample <= 0:
        raise ValueError("--validation-sample must be positive")
    if any(rate <= 0.0 or not math.isfinite(rate) for rate in arguments.lr):
        raise ValueError("every --lr value must be finite and positive")
    if arguments.k <= 0.0 or not math.isfinite(arguments.k):
        raise ValueError("--k must be finite and positive")
    device = torch.device(arguments.device)
    if device.type == "cuda" and not torch.cuda.is_available():
        raise ValueError("CUDA was requested but is not available")
    return device


def command_train(arguments: argparse.Namespace) -> None:
    """trainサブコマンドを実行する。"""
    device = _validate_arguments(arguments)
    records = load_records(arguments.data)
    validation_mask = records["game"] % 20 == 0
    if not np.any(validation_mask) or np.all(validation_mask):
        raise ValueError("game split produced an empty training or validation set")
    training_indices_array = np.flatnonzero(~validation_mask).astype(np.int64)
    validation_indices_array = np.flatnonzero(validation_mask).astype(np.int64)
    print(
        f"records: total={records.size} training={training_indices_array.size} "
        f"validation={validation_indices_array.size}"
    )
    print("building feature indices")
    features = build_feature_tensors(records, device)
    targets = torch.as_tensor(
        build_targets(records, arguments.k, arguments.lambda_value), device=device
    )
    training_indices = torch.as_tensor(training_indices_array, device=device)
    validation_indices = torch.as_tensor(validation_indices_array, device=device)

    selected_rate = arguments.lr[0]
    if len(arguments.lr) > 1:
        candidates: list[tuple[float, float]] = []
        for rate in arguments.lr:
            model = make_model(device, arguments.seed)
            optimizer = torch.optim.Adam(model.parameters(), lr=rate)
            generator = torch.Generator(device=device).manual_seed(arguments.seed)
            training_loss = train_epoch(
                model,
                optimizer,
                features,
                targets,
                training_indices,
                arguments.batch,
                generator,
            )
            loss = validation_loss(
                model,
                features,
                targets,
                validation_indices,
                arguments.batch,
            )
            if not math.isfinite(training_loss) or not math.isfinite(loss):
                raise ValueError(f"learning rate {rate:g} produced a non-finite loss")
            candidates.append((loss, rate))
            print(
                f"learning-rate candidate: lr={rate:g} "
                f"train_loss={training_loss:.9f} validation_loss={loss:.9f}"
            )
        selected_rate = min(candidates)[1]
    print(f"selected learning rate: {selected_rate:g}")

    model = make_model(device, arguments.seed)
    optimizer = torch.optim.Adam(model.parameters(), lr=selected_rate)
    # 一定学習率では検証損失が30エポック前後で雑音の中に埋もれて停滞したので、
    # 余弦減衰で終盤の学習率を下げる。
    scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(optimizer, T_max=arguments.epochs)
    generator = torch.Generator(device=device).manual_seed(arguments.seed)
    best_loss = math.inf
    best_state: dict[str, Tensor] | None = None
    best_epoch = 0
    for epoch in range(1, arguments.epochs + 1):
        training_loss = train_epoch(
            model,
            optimizer,
            features,
            targets,
            training_indices,
            arguments.batch,
            generator,
        )
        loss = validation_loss(
            model,
            features,
            targets,
            validation_indices,
            arguments.batch,
        )
        scheduler.step()
        if not math.isfinite(training_loss) or not math.isfinite(loss):
            raise ValueError(f"epoch {epoch} produced a non-finite loss")
        print(
            f"epoch {epoch}: train_loss={training_loss:.9f} "
            f"validation_loss={loss:.9f}"
        )
        if loss < best_loss:
            best_loss = loss
            best_epoch = epoch
            best_state = {
                name: value.detach().cpu().clone()
                for name, value in model.state_dict().items()
            }
    if best_state is None:
        raise AssertionError("training did not produce a best model")
    model.load_state_dict(best_state)
    print(f"best epoch: {best_epoch} validation_loss={best_loss:.9f}")

    parameters = float_parameters(model)
    # 量子化誤差の検査に失敗しても浮動小数点重みは診断に使えるので、検査前に保存する。
    save_float_parameters(arguments.float_output, parameters, arguments.k)
    params = quantize(parameters)
    mean_error, maximum_error, sample_count = check_quantization_error(
        model,
        params,
        records,
        features,
        validation_indices_array,
        arguments.k,
        arguments.validation_sample,
        arguments.batch,
        arguments.seed,
    )
    print(
        f"quantization error: samples={sample_count} "
        f"mean_absolute={mean_error:.9f} max={maximum_error:.9f} cp"
    )
    error_limit = QUANTIZATION_ERROR_LIMIT_LOGIT * arguments.k
    if mean_error > error_limit:
        raise ValueError(
            f"quantization mean absolute error {mean_error} cp exceeds "
            f"{error_limit} cp ({QUANTIZATION_ERROR_LIMIT_LOGIT} logit)"
        )
    write_mnue(arguments.output, params, arguments.k)
    black, white = initial_position_scores(params, arguments.k)
    print(f"initial black-to-move: {black} cp")
    print(f"initial white-to-move: {white} cp")


def build_parser() -> argparse.ArgumentParser:
    """学習器CLIの引数解析器を作る。"""
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    train_parser = commands.add_parser("train", help="P型NNUEを訓練する")
    train_parser.add_argument("--data", required=True, nargs="+")
    train_parser.add_argument("--output", required=True, type=Path)
    train_parser.add_argument("--float-output", required=True, type=Path)
    train_parser.add_argument("--k", required=True, type=float)
    train_parser.add_argument("--lambda", dest="lambda_value", type=float, default=0.75)
    train_parser.add_argument("--lr", required=True, type=float, nargs="+")
    train_parser.add_argument("--epochs", type=int, default=20)
    train_parser.add_argument("--batch", type=int, default=16384)
    train_parser.add_argument("--seed", type=int, default=1)
    train_parser.add_argument("--validation-sample", type=int, default=10000)
    train_parser.add_argument("--device", default="cuda")
    train_parser.set_defaults(handler=command_train)
    return parser


def main(arguments: Sequence[str] | None = None) -> None:
    """CLI引数を解析し、選択されたサブコマンドを実行する。"""
    parsed = build_parser().parse_args(arguments)
    parsed.handler(parsed)


if __name__ == "__main__":
    main()
