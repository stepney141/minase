"""手計算できる3局面で診断の定義を検査し、実ネットで状態の保存を確かめる。

3局面の合法手質量は1/2、1/2、1/4、一致は真、偽、真とする。
2局面を同じ対局へ置き、候補数・対局数・バッチ数による重み付けの誤りを検出する。
"""

import math
import random
import unittest

import numpy as np
import torch

from minase_zero.data import Dataset
from minase_zero.encoding import encode_planes, policy_mask
from minase_zero.net import TrainingLogits, ZeroNet
from minase_zero.train import compute_policy_diagnostics, diagnose


class KnownLogits(torch.nn.Module):
    """升0〜2の歩兵で識別した局面ごとに、指定したlogitを返す参照ネット。"""

    def __init__(self, policy: torch.Tensor, value: torch.Tensor) -> None:
        super().__init__()
        self.register_buffer("policy", policy)
        self.register_buffer("value", value)

    def forward_logits(self, inputs: torch.Tensor) -> TrainingLogits:
        # 面0〜93の駒の配置から、歩兵を置いた升を求める。
        indices = inputs[:, :94].sum(dim=1).flatten(1).argmax(1)
        return TrainingLogits(self.policy[indices], self.value[indices].expand(-1, 4),
                              self.policy[indices], self.policy[indices], self.policy[indices])


def example() -> tuple[Dataset, KnownLogits]:
    """最大確率の同率と最大logitの同率を別の局面に持たせる。"""
    boards = np.zeros((3, 144), dtype=np.uint8)
    boards[np.arange(3), np.arange(3)] = 1
    data = Dataset(
        boards=boards, sides=np.zeros(3, dtype=np.uint8), lions=np.full(3, 255, dtype=np.uint8),
        values=np.array([0, 65_535, 32_768], dtype=np.int32), results=np.array([1, 0, 0.5], dtype=np.float32),
        validation=np.array([False, False, True]), training_sample=np.array([True, True, False]),
        games=np.array([1, 1, 2], dtype=np.uint32), plies=np.array([0, 1, 0], dtype=np.uint16),
        file_indices=np.zeros(3, dtype=np.uint32), policy_offsets=np.array([0, 3, 4, 5], dtype=np.uint64),
        policy_labels=np.array([2, 4, 23, 4, 23], dtype=np.uint16),
        policy_probabilities=np.array([30_000, 30_000, 5535, 65_535, 65_535], dtype=np.uint16),
        legal_offsets=np.array([0, 3, 5, 6], dtype=np.uint64),
        legal_labels=np.array([23, 4, 2, 4, 2, 23], dtype=np.uint16),
        short_term_values=np.full((3, 3), 0.5, dtype=np.float32),
        opponent_policy_offsets=np.zeros(4, dtype=np.uint64),
        opponent_policy_labels=np.empty(0, dtype=np.uint16),
        opponent_policy_probabilities=np.empty(0, dtype=np.uint16),
        opponent_legal_offsets=np.zeros(4, dtype=np.uint64),
        opponent_legal_labels=np.empty(0, dtype=np.uint16),
    )
    logits = torch.full((3, 35_856), -1000.0)
    logits[0, [2, 4, 23, 42]] = torch.tensor([0, math.log(4), math.log(2), math.log(7)])
    logits[1, [2, 4, 23]] = torch.tensor([math.log(3), math.log(3), math.log(6)])
    logits[2, [2, 23]] = torch.tensor([math.log(6), math.log(2)])
    logits[:, ~torch.from_numpy(policy_mask())] = 1000
    model = KnownLogits(logits, torch.tensor([[math.log(3)], [-math.log(3)], [0.0]]))
    return data, model


class DiagnosticTests(unittest.TestCase):
    def setUp(self) -> None:
        previous = torch.get_num_threads()
        torch.set_num_threads(1)
        self.addCleanup(torch.set_num_threads, previous)
        self.mask = torch.from_numpy(policy_mask())

    def test_position_means_teacher_ties_and_smallest_legal_label(self) -> None:
        data, model = example()
        expected = {
            "policy": (math.log(14) - (30_000 * math.log(4) + 5535 * math.log(2)) / 65_535
                       + 2 * math.log(4)) / 3,
            "value": (2 * (math.log(4) - 0.75 * math.log(3)) + math.log(2)) / 3,
            "value_result": (2 * math.log(4 / 3) + math.log(2)) / 3,
            "legal_probability_mass": 5 / 12,
            "log_legal_probability_mass": -4 * math.log(2) / 3,
            "policy_agreement": 2 / 3,
        }
        for batch_size in (1, 2, 3):
            with self.subTest(batch_size=batch_size):
                actual = diagnose(model, data, np.arange(3), torch.device("cpu"),
                                  {"precision": "fp32", "value_lambda": 0.25, "batch_size": batch_size}, self.mask, auxiliary=False)
                self.assertEqual(actual.keys(), expected.keys())
                for name, value in expected.items():
                    self.assertAlmostEqual(actual[name], value, places=6, msg=name)

    def test_auxiliary_diagnostics_average_positions_across_partial_batches(self) -> None:
        data, model = example()
        large = (30_000 / 65_535 + 1e-7) ** 0.25
        small = (5535 / 65_535 + 1e-7) ** 0.25
        first_soft = math.log(14) - (large * math.log(4) + small * math.log(2)) / (2 * large + small)
        short_value = (2 * (math.log(4) - 0.5 * math.log(3)) + math.log(2)) / 3
        expected = {"soft_policy": (first_soft + 2 * math.log(4)) / 3,
                    "opponent_policy": 0, "opponent_soft_policy": 0,
                    **{f"short_term_value_{index}": short_value for index in (1, 2, 3)}}
        for batch_size in (1, 2, 3):
            with self.subTest(batch_size=batch_size):
                actual = diagnose(model, data, np.arange(3), torch.device("cpu"),
                                  {"precision": "fp32", "value_lambda": 0.25, "batch_size": batch_size},
                                  self.mask, auxiliary=True)
                for name, value in expected.items():
                    self.assertAlmostEqual(actual[name], value, places=6, msg=name)

    def test_log_mass_remains_finite_when_mass_underflows(self) -> None:
        logits = torch.full((1, 35_856), -1000.0)
        logits[0, 4] = 1000
        mass, log_mass, agreement = compute_policy_diagnostics(
            logits, torch.tensor([0]), torch.tensor([2]), torch.tensor([65_535]),
            torch.tensor([0]), torch.tensor([2]), self.mask,
        )
        self.assertEqual(mass.item(), 0)
        self.assertEqual(log_mass.item(), -2000)
        self.assertEqual(agreement.item(), 1)
        for value in (mass, log_mass, agreement):
            self.assertEqual(value.dtype, torch.float32)

    def test_missing_legal_moves_and_empty_diagnostic_set_raise(self) -> None:
        with self.assertRaises(ValueError):
            compute_policy_diagnostics(
                torch.zeros(2, 35_856), torch.tensor([0, 1]), torch.tensor([2, 4]),
                torch.tensor([65_535, 65_535]), torch.tensor([0]), torch.tensor([2]), self.mask,
            )
        data, model = example()
        with self.assertRaises(ValueError):
            diagnose(model, data, np.empty(0, dtype=np.int64), torch.device("cpu"),
                     {"precision": "fp32", "value_lambda": 0.25, "batch_size": 2}, self.mask, auxiliary=False)

    def test_large_nonlegal_logit_does_not_create_a_false_legal_tie(self) -> None:
        logits = torch.zeros(1, 35_856)
        logits[0, 4], logits[0, 23] = 1, 1e10
        _, _, agreement = compute_policy_diagnostics(
            logits, torch.tensor([0]), torch.tensor([4]), torch.tensor([65_535]),
            torch.tensor([0, 0]), torch.tensor([2, 4]), self.mask,
        )
        self.assertEqual(agreement.item(), 1)

    def test_diagnostics_preserve_weights_batch_norm_gradients_and_random_states(self) -> None:
        data, _ = example()
        model = ZeroNet(1, 16).train()
        inputs = encode_planes(*(torch.from_numpy(column) for column in (data.boards, data.sides, data.lions)))
        logits = model.forward_logits(inputs)
        sum(output.mean() for output in logits).backward()
        weights = {name: tensor.clone() for name, tensor in model.state_dict().items()}
        gradients = [parameter.grad.clone() for parameter in model.parameters()]
        python_rng, numpy_rng, torch_rng = random.getstate(), np.random.get_state(), torch.get_rng_state()
        diagnose(model, data, np.arange(3), torch.device("cpu"),
                 {"precision": "fp32", "value_lambda": 0.25, "batch_size": 2}, self.mask, auxiliary=True)
        self.assertFalse(model.training)
        for name, before in weights.items():
            self.assertTrue(torch.equal(model.state_dict()[name], before), name)
        for parameter, before in zip(model.parameters(), gradients):
            self.assertTrue(torch.equal(parameter.grad, before))
        self.assertEqual(random.getstate(), python_rng)
        after = np.random.get_state()
        self.assertEqual(after[0], numpy_rng[0])
        np.testing.assert_array_equal(after[1], numpy_rng[1])
        self.assertEqual(after[2:], numpy_rng[2:])
        self.assertTrue(torch.equal(torch.get_rng_state(), torch_rng))


if __name__ == "__main__":
    unittest.main()
