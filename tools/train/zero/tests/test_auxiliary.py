"""フェーズ4の目標分布、損失の重み、欠けた相手教師の分母を手計算で検証する。"""

import math
import unittest

import torch

from minase_zero.encoding import policy_mask
from minase_zero.net import TrainingLogits
from minase_zero.train import PolicyTargets, compute_training_losses, soft_policy_targets, total_loss


class AuxiliaryLossTests(unittest.TestCase):
    def setUp(self):
        threads = torch.get_num_threads()
        torch.set_num_threads(1)
        self.addCleanup(torch.set_num_threads, threads)
        self.mask = torch.from_numpy(policy_mask())
        self.opponent_mask = torch.from_numpy(policy_mask(opponent=True))
        self.policy = PolicyTargets(
            torch.tensor([0, 1, 1]), torch.tensor([2, 4, 23]), torch.tensor([65535, 21845, 43690]),
            torch.tensor([0, 0, 1, 1, 1]), torch.tensor([2, 4, 2, 4, 23]),
        )

    def test_soft_targets_include_zero_probability_legal_moves(self):
        # 1候補と2合法手では確率1にならない。次の行には確率0、1/3、2/3を置く。
        a, b = (1 + 1e-7) ** 0.25, (1e-7) ** 0.25
        c, d = (1 / 3 + 1e-7) ** 0.25, (2 / 3 + 1e-7) ** 0.25
        expected = [a / (a + b), b / (a + b), b / (b + c + d), c / (b + c + d), d / (b + c + d)]
        actual = soft_policy_targets(self.policy, 2, 35_856)
        torch.testing.assert_close(actual, torch.tensor(expected))
        self.assertLess(actual[0], 1)
        self.assertTrue(torch.all(actual > 0))
        self.assertAlmostEqual(actual[:2].sum().item(), 1)
        self.assertAlmostEqual(actual[2:].sum().item(), 1)

    def test_all_loss_terms_and_weights_match_their_definitions(self):
        policy = torch.full((2, 35_856), -1000.0)
        policy[0, [2, 4, 23]] = torch.tensor([math.log(2), math.log(3), math.log(5)])
        policy[1, [2, 4, 23]] = torch.tensor([math.log(7), math.log(11), math.log(13)])
        # 相手の歩兵が相対段4から3へ成る手。自手のマスクでは無効になる。
        promotion = 132 * 144 + 53
        self.assertTrue(self.opponent_mask[promotion])
        self.assertFalse(self.mask[promotion])
        opponent = torch.full((2, 35_856), -1000.0)
        opponent[0, [promotion, 2]] = torch.tensor([math.log(4), 0.0])
        opponent[1] = 0
        # 不可能なlogitが正規化と勾配に入らないことも同時に確かめる。
        policy[:, ~self.mask] = 1000
        opponent[:, ~self.opponent_mask] = 1000
        values = torch.tensor([[math.log(3), math.log(3), 0, -math.log(3)],
                               [-math.log(3), -math.log(3), math.log(3), 0]])
        logits = TrainingLogits(*(tensor.clone().requires_grad_()
                                  for tensor in (policy, values, policy, opponent, opponent)))
        targets = PolicyTargets(torch.tensor([0]), torch.tensor([promotion]), torch.tensor([65535]),
                                torch.tensor([0, 0]), torch.tensor([promotion, 2]))
        losses = compute_training_losses(
            logits, self.policy, torch.tensor([1.0, 0.0]), torch.tensor([0, 65535]), targets,
            torch.tensor([[1.0, 0.5, 0.0], [0.0, 0.0, 0.5]]), 0.25, self.mask, self.opponent_mask,
        )
        a, b = (1 + 1e-7) ** 0.25, (1e-7) ** 0.25
        c, d = (1 / 3 + 1e-7) ** 0.25, (2 / 3 + 1e-7) ** 0.25
        expected = [
            (math.log(5) + math.log(31) - (math.log(11) + 2 * math.log(13)) / 3) / 2,
            math.log(4) - 0.75 * math.log(3), math.log(4 / 3),
            (math.log(10) - (a * math.log(2) + b * math.log(3)) / (a + b)
             + math.log(31) - (b * math.log(7) + c * math.log(11) + d * math.log(13)) / (b + c + d)) / 2,
            math.log(5 / 4) / 2, (math.log(5) - a * math.log(4) / (a + b)) / 2,
            math.log(4 / 3), (math.log(2) + math.log(4)) / 2, (math.log(4 / 3) + math.log(2)) / 2,
        ]
        for actual, value in zip(losses, expected, strict=True):
            self.assertEqual(actual.dtype, torch.float32)
            self.assertAlmostEqual(actual.item(), value, places=6)
        expected_total = (expected[0] + expected[1] + 8 * expected[3] + 0.15 * expected[4]
                          + 1.2 * expected[5] + sum(expected[6:]))
        total = total_loss(losses)
        self.assertAlmostEqual(total.item(), expected_total, places=5)
        self.assertFalse(losses[2].requires_grad)
        total.backward()
        for output in (logits.policy, logits.soft_policy):
            self.assertTrue(torch.all(output.grad[:, ~self.mask] == 0))
        for output in (logits.opponent_policy, logits.opponent_soft_policy):
            self.assertTrue(torch.all(output.grad[1] == 0))
            self.assertTrue(torch.all(output.grad[:, ~self.opponent_mask] == 0))
            self.assertNotEqual(output.grad[0, promotion], 0)
        self.assertTrue(torch.all(logits.value.grad.abs().sum(dim=0) > 0))

    def test_missing_opponent_targets_give_zero_losses_and_gradients(self):
        empty = torch.empty(0, dtype=torch.int64)
        targets = PolicyTargets(empty, empty, empty, empty, empty)
        logits = TrainingLogits(torch.zeros(2, 35_856, requires_grad=True),
                                torch.zeros(2, 4, requires_grad=True),
                                *(torch.zeros(2, 35_856, requires_grad=True) for _ in range(3)))
        losses = compute_training_losses(logits, self.policy, torch.zeros(2), torch.zeros(2), targets,
                                         torch.zeros(2, 3), 0.25, self.mask, self.opponent_mask)
        self.assertEqual((losses[4].item(), losses[5].item()), (0, 0))
        total_loss(losses).backward()
        self.assertTrue(torch.all(logits.opponent_policy.grad == 0))
        self.assertTrue(torch.all(logits.opponent_soft_policy.grad == 0))


if __name__ == "__main__":
    unittest.main()
