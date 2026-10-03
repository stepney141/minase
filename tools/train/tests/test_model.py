"""PSTモデルの初期化、重み共有、および予測を検証する。"""

from __future__ import annotations

import unittest

import numpy as np
import torch

from minase_train.data.features import (
    FEATURE_COUNT,
    INITIAL_BOARD,
    PADDING_INDEX,
    feature_indices,
    mirror,
)
from minase_train.data.mnpt import quantize
from minase_train.data.taper import phase_numerators, phase_ratios
from minase_train.pst.evaluate import float_evaluate, integer_evaluate
from minase_train.pst.model import expanded_model_weights, make_model, model_logits


class MirroredModelTest(unittest.TestCase):

    """strength-stage7.md「鏡映の重み共有」の全特徴と学習経路の契約を検証する。"""

    def test_initialization_averages_each_endpoint_and_padding_stays_zero_after_update(self) -> None:
        random = np.random.default_rng(731)
        initial = random.integers(-16_000, 16_000, size=(FEATURE_COUNT, 2)).astype(np.float32) / 8.0
        model = make_model(torch.from_numpy(initial), torch.device("cpu"), "mirrored")
        self.assertEqual(tuple(model.weight.shape), (6841, 2))
        self.assertEqual(sum(parameter.numel() for parameter in model.parameters()), 6841 * 2)
        expected = (initial.reshape(95, 12, 12, 2) + initial.reshape(95, 12, 12, 2)[:, :, ::-1]) / 2
        np.testing.assert_array_equal(expanded_model_weights(model).detach().numpy(), expected.reshape(FEATURE_COUNT, 2))
        # 全特徴を個別に入力して、各鏡映対が片側の更新を共有することを確認する。
        features = torch.arange(FEATURE_COUNT + 1).reshape(-1, 1)
        phi = torch.linspace(0, 1, FEATURE_COUNT + 1)
        optimizer = torch.optim.SGD(model.parameters(), lr=0.1)
        model_logits(model, features, phi, 200.0).sum().backward()
        optimizer.step()
        expanded = expanded_model_weights(model).detach().numpy().reshape(95, 12, 12, 2)
        np.testing.assert_array_equal(expanded, expanded[:, :, ::-1])
        np.testing.assert_array_equal(model(torch.tensor([[PADDING_INDEX]])).detach().numpy(), 0)

    def test_expansion_matches_tapered_predictions_and_initial_perspectives(self) -> None:
        random = np.random.default_rng(432)
        initial = torch.from_numpy(random.normal(0.0, 100.0, (FEATURE_COUNT, 2)).astype(np.float32))
        model = make_model(initial, torch.device("cpu"), "mirrored")
        expanded = expanded_model_weights(model).detach()
        tapered = make_model(expanded, torch.device("cpu"), "tapered")
        # 0, 2, 47, 92枚と全盤面の境界、両視点、先獅子対象升を含む。
        boards = np.zeros((6, 144), dtype=np.uint8)
        boards[1, [5, 138]] = [12, 76]
        boards[2, :47] = 1
        boards[3] = INITIAL_BOARD
        boards[4] = INITIAL_BOARD
        boards[5, :] = random.choice([1, 30, 65, 94], 144)
        stm = np.array([0, 1, 0, 0, 1, 1], dtype=np.uint8)
        lion = np.array([255, 255, 17, 255, 255, 143], dtype=np.uint8)
        features = feature_indices(boards, stm, lion)
        phi = torch.from_numpy(phase_ratios(boards).astype(np.float32))
        logits = model_logits(model, torch.from_numpy(features), phi, 1072.6529541015625)
        torch.testing.assert_close(logits, model_logits(tapered, torch.from_numpy(features), phi, 1072.6529541015625))
        reflected_board, reflected_lion = mirror(boards, lion)
        reflected = feature_indices(reflected_board, stm, reflected_lion)
        torch.testing.assert_close(logits, model_logits(model, torch.from_numpy(reflected), phi, 1072.6529541015625))
        floating = float_evaluate(expanded[:, 0].numpy(), expanded[:, 1].numpy(), features, phase_ratios(boards))
        self.assertAlmostEqual(float(floating[3]), float(floating[4]), delta=0.001)
        self.assertAlmostEqual(float(logits[3].detach()), float(logits[4].detach()), delta=0.000001)
        quantized = quantize(expanded.numpy())
        integer = integer_evaluate(quantized[:, 0], quantized[:, 1], features, phase_numerators(boards))
        self.assertEqual(int(integer[3]), int(integer[4]))
        np.testing.assert_array_equal(
            integer,
            integer_evaluate(quantized[:, 0], quantized[:, 1], reflected, phase_numerators(boards)),
        )


if __name__ == "__main__":
    unittest.main()
