"""MNZTの局面を学習のバッチへ変換する。

学習と検証の分割は対局単位で行い、同じ対局の局面を両方に入れない
（docs/plans/alphazero.md の「学習データの形式」）。
バッチは、入力平面`float32 [N, 96, 12, 12]`、方策の教師を長さの揃った（ラベル、確率）の配列と個数、
および価値の教師`(1 − λ)·結果 + λ·根の価値`からなる。
MNZTの局面は成り権保留の升を持たないので、入力平面の面95（成り権保留の升）は常に0になる。
方策の教師を`[N, 35856]`の密な配列へ展開しないのは、損失の計算側がラベルの位置だけを読めば済むからである。
"""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np
import torch
from torch.utils.data import DataLoader, Dataset, Sampler

from minase_zero.encoding import BOARD_SIZE, INPUT_PLANES, encode
from minase_zero.mnzt import MnztCollection, record_position

OUTCOME_SCORES = np.array([0.0, 0.5, 1.0], dtype=np.float32)
"""結果コード（0負け、1引き分け、2勝ち）から手番側の得点への対応。"""


@dataclass(frozen=True)
class Split:
    """対局単位の学習と検証の分割。どちらも大域的なレコード番号の昇順の配列である。"""

    train: np.ndarray
    """学習に使うレコードの番号。"""
    validation: np.ndarray
    """検証に使うレコードの番号。"""
    validation_games: int
    """検証に回した対局の数。"""
    games: int
    """対局の総数。"""


def split_by_game(data: MnztCollection, validation_fraction: float, seed: int) -> Split:
    """対局（ファイル番号、対局番号）を乱択して、検証に`ceil(対局数 × 割合)`局を回す。

    同じ`seed`と同じファイルの並びからは同じ分割を返す。

    Raises:
        ValueError: 割合が0以上1未満でない場合、または学習に回る対局がなくなる場合。
    """
    if not 0.0 <= validation_fraction < 1.0:
        raise ValueError(f"validation fraction must be in [0, 1), got {validation_fraction}")
    keys = data.game_keys()
    games = np.unique(keys)
    validation_games = math.ceil(len(games) * validation_fraction)
    if validation_games >= len(games):
        raise ValueError(
            f"{len(games)} games leave no training game at validation fraction {validation_fraction}"
        )
    rng = np.random.default_rng(seed)
    chosen = rng.permutation(games)[:validation_games]
    in_validation = np.isin(keys, chosen)
    return Split(
        train=np.flatnonzero(~in_validation),
        validation=np.flatnonzero(in_validation),
        validation_games=validation_games,
        games=len(games),
    )


class BatchDataset(Dataset):
    """レコード番号の配列を受けて1バッチを作る`Dataset`。

    `DataLoader`には`batch_size=None`と、番号の配列を返す標本器を組み合わせて渡す。
    """

    def __init__(self, data: MnztCollection, value_lambda: float) -> None:
        if not 0.0 <= value_lambda <= 1.0:
            raise ValueError(f"lambda must be in [0, 1], got {value_lambda}")
        self.data = data
        self.value_lambda = value_lambda

    def __getitem__(self, indices: np.ndarray) -> dict[str, torch.Tensor]:
        """番号`indices`の局面を符号化したバッチを返す。

        キーは`input`（`[N, 96, 12, 12]`）、`policy_labels`（`int64 [N, L]`）、
        `policy_probs`（`float32 [N, L]`）、`policy_counts`（`int64 [N]`）、`value`（`float32 [N]`）である。
        `L`はバッチ内の方策の組の個数の最大値であり、足りない分はラベル0、確率0で埋める。
        """
        file_indices, local_indices = self.data.locate(indices)
        size = len(indices)
        inputs = np.empty((size, INPUT_PLANES, BOARD_SIZE, BOARD_SIZE), dtype=np.float32)
        values = np.empty(size, dtype=np.float32)
        labels: list[np.ndarray] = []
        probs: list[np.ndarray] = []
        for row, (file_index, local) in enumerate(zip(file_indices, local_indices)):
            file = self.data.files[file_index]
            record = file.records[local]
            inputs[row] = encode(record_position(record))
            outcome = OUTCOME_SCORES[record["outcome"]]
            values[row] = (1.0 - self.value_lambda) * outcome + self.value_lambda * record["value"]
            label, prob = file.policy_of(local)
            labels.append(label)
            probs.append(prob)
        counts = np.array([len(label) for label in labels], dtype=np.int64)
        width = max(1, int(counts.max(initial=0)))
        padded_labels = np.zeros((size, width), dtype=np.int64)
        padded_probs = np.zeros((size, width), dtype=np.float32)
        for row, (label, prob) in enumerate(zip(labels, probs)):
            padded_labels[row, : len(label)] = label
            padded_probs[row, : len(prob)] = prob
        return {
            "input": torch.from_numpy(inputs),
            "policy_labels": torch.from_numpy(padded_labels),
            "policy_probs": torch.from_numpy(padded_probs),
            "policy_counts": torch.from_numpy(counts),
            "value": torch.from_numpy(values),
        }


class EpochBatchSampler(Sampler):
    """1エポック分の番号を`seed`とエポック番号から決まる順に並べ、バッチごとに返す標本器。

    端数のバッチは捨てる。学習中のBatchNormは大きさ1のバッチを受け付けないからである。
    `start_batch`を与えると、そのバッチから返し始める。中断した学習を同じ順序の続きから再開するために使う。
    """

    def __init__(
        self, indices: np.ndarray, batch_size: int, seed: int, epoch: int, start_batch: int
    ) -> None:
        self.batches_per_epoch = len(indices) // batch_size
        if not 0 <= start_batch <= self.batches_per_epoch:
            raise ValueError(f"start batch {start_batch} outside the epoch")
        order = np.random.default_rng([seed, epoch]).permutation(indices)
        self.batches = order[: self.batches_per_epoch * batch_size].reshape(-1, batch_size)
        self.start_batch = start_batch

    def __iter__(self):
        """残りのバッチを、局面の番号を昇順に並べ替えて返す（ファイルを読む位置を近づけるため）。"""
        for batch in self.batches[self.start_batch:]:
            yield np.sort(batch)

    def __len__(self) -> int:
        """残りのバッチの数を返す。"""
        return self.batches_per_epoch - self.start_batch


def train_loader(
    dataset: BatchDataset,
    indices: np.ndarray,
    batch_size: int,
    seed: int,
    epoch: int,
    start_batch: int,
    workers: int,
    pin_memory: bool,
) -> DataLoader:
    """1エポック分の学習のバッチを`start_batch`から返す`DataLoader`を作る。"""
    sampler = EpochBatchSampler(indices, batch_size, seed, epoch, start_batch)
    return DataLoader(
        dataset, batch_size=None, sampler=sampler, num_workers=workers, pin_memory=pin_memory
    )


def validation_loader(
    dataset: BatchDataset, indices: np.ndarray, batch_size: int, workers: int, pin_memory: bool
) -> DataLoader:
    """検証のレコードを番号順に、端数のバッチも含めて返す`DataLoader`を作る。"""
    batches = [indices[begin:begin + batch_size] for begin in range(0, len(indices), batch_size)]
    return DataLoader(
        dataset, batch_size=None, sampler=batches, num_workers=workers, pin_memory=pin_memory
    )
