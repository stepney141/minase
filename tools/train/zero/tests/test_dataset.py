"""対局単位の分割、バッチの内容、およびエポック内の順序を検査する。"""

from __future__ import annotations

import numpy as np
import pytest
import torch
from conftest import INITIAL_SFEN, game_records, write_mnzt

from minase_zero.dataset import (
    BatchDataset,
    EpochBatchSampler,
    split_by_game,
    validation_loader,
)
from minase_zero.encoding import INPUT_PLANES, encode, parse_sfen
from minase_zero.mnzt import MnztCollection


def collection(tmp_path, games_per_file=(6, 5)) -> MnztCollection:
    """対局番号が1から始まる合成ファイルを複数作って連結する。ファイルをまたいで対局番号が重なる。"""
    paths = []
    for file_index, games in enumerate(games_per_file):
        records = [r for game in range(1, games + 1) for r in game_records(game, 3 + game)]
        paths.append(write_mnzt(tmp_path / f"{file_index}.mnzt", records))
    return MnztCollection.open(paths)


def test_split_keeps_each_game_on_one_side(tmp_path):
    """同じ（ファイル、対局）の局面は学習と検証の片方だけに入り、検証の対局数は切り上げになる。"""
    data = collection(tmp_path)
    split = split_by_game(data, 0.25, seed=3)
    keys = data.game_keys()
    assert split.games == 11
    assert split.validation_games == 3
    assert set(keys[split.train]).isdisjoint(keys[split.validation])
    assert len(np.unique(keys[split.validation])) == 3
    assert np.array_equal(np.sort(np.concatenate([split.train, split.validation])), np.arange(len(data)))
    again = split_by_game(data, 0.25, seed=3)
    assert np.array_equal(again.validation, split.validation)


def test_sample_split_is_by_game(sample_mnzt):
    """見本の既定の割合でも検証に1局以上が回り、対局が両側に分かれない。"""
    data = MnztCollection.open([sample_mnzt])
    split = split_by_game(data, 0.05, seed=0)
    keys = data.game_keys()
    assert split.validation_games >= 1
    assert len(split.validation) > 0
    assert set(keys[split.train]).isdisjoint(keys[split.validation])


def test_split_rejects_fraction_without_training_games(tmp_path):
    """学習に回る対局がなくなる割合を拒否する。"""
    data = collection(tmp_path, (1,))
    with pytest.raises(ValueError):
        split_by_game(data, 0.5, seed=0)


def test_batch_contents(tmp_path):
    """入力平面、埋め草つきの方策、および価値の教師`(1 − λ)·結果 + λ·根の価値`を返す。"""
    records = [
        {"game": 1, "ply": 0, "outcome": 2, "value": 0.8, "policy": [(10, 0.25), (20, 0.75)]},
        {"game": 1, "ply": 1, "outcome": 1, "value": 0.4, "policy": [(30, 1.0)]},
        {"game": 1, "ply": 2, "outcome": 0, "value": 0.1, "policy": [(40, 0.5), (50, 0.25), (60, 0.25)]},
    ]
    data = MnztCollection.open([write_mnzt(tmp_path / "a.mnzt", records)])
    batch = BatchDataset(data, value_lambda=0.25)[np.array([0, 1, 2])]
    assert batch["input"].shape == (3, INPUT_PLANES, 12, 12)
    assert batch["input"].dtype == torch.float32
    np.testing.assert_array_equal(batch["input"][0].numpy(), encode(parse_sfen(INITIAL_SFEN)))
    assert batch["policy_counts"].tolist() == [2, 1, 3]
    assert batch["policy_labels"].tolist() == [[10, 20, 0], [30, 0, 0], [40, 50, 60]]
    np.testing.assert_allclose(
        batch["policy_probs"].numpy(),
        [[0.25, 0.75, 0.0], [1.0, 0.0, 0.0], [0.5, 0.25, 0.25]],
        atol=1 / 65535,
    )
    np.testing.assert_allclose(
        batch["value"].numpy(),
        [0.75 * 1.0 + 0.25 * 0.8, 0.75 * 0.5 + 0.25 * 0.4, 0.75 * 0.0 + 0.25 * 0.1],
        rtol=1e-6,
    )


def test_epoch_sampler_is_deterministic_and_resumable():
    """同じシードとエポックで同じ順序になり、途中のバッチから始めると残りが一致し、端数は捨てる。"""
    indices = np.arange(100, 123)
    full = list(EpochBatchSampler(indices, 5, seed=1, epoch=2, start_batch=0))
    assert len(full) == 4
    assert all(len(batch) == 5 for batch in full)
    assert len(np.unique(np.concatenate(full))) == 20
    resumed = list(EpochBatchSampler(indices, 5, seed=1, epoch=2, start_batch=3))
    assert [b.tolist() for b in resumed] == [b.tolist() for b in full[3:]]
    other_epoch = list(EpochBatchSampler(indices, 5, seed=1, epoch=3, start_batch=0))
    assert [b.tolist() for b in other_epoch] != [b.tolist() for b in full]


def test_validation_loader_covers_all_records(tmp_path):
    """検証の`DataLoader`は端数のバッチも含めて全局面を1回ずつ返す。"""
    data = collection(tmp_path)
    indices = np.arange(len(data))
    loader = validation_loader(BatchDataset(data, 0.333), indices, 4, workers=0, pin_memory=False)
    assert sum(len(batch["value"]) for batch in loader) == len(data)
