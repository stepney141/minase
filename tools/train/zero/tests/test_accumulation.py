"""単一CPUで勾配累積の算術、移動統計、および非有限の損失の拒否を検証する。"""

import copy
import unittest
from unittest.mock import patch

from lightning_fabric import Fabric
from lightning_fabric.strategies import SingleDeviceStrategy
import numpy as np
import torch

from minase_zero.data import read_file
from minase_zero.encoding import policy_mask
from minase_zero.net import ZeroNet
from minase_zero.train import (
    LOSS_NAMES, TrainingModule, _batch, _policy_batch, accumulate_gradients,
    compute_training_losses, total_loss,
)

from tests.cli import ZERO


class AccumulationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data, _ = read_file(ZERO / "fixtures/zero-selfplay-small.mnsd")

    def setUp(self):
        threads = torch.get_num_threads()
        torch.set_num_threads(1)
        self.addCleanup(torch.set_num_threads, threads)
        self.fabric = Fabric(accelerator="cpu", devices=1, num_nodes=1, precision="32-true",
                             strategy=SingleDeviceStrategy(device=torch.device("cpu")))
        self.fabric.launch()
        torch.manual_seed(23)
        self.network = ZeroNet(1, 4).train()
        self.reference = copy.deepcopy(self.network)
        optimizer = torch.optim.SGD(self.network.parameters(), lr=0.01)
        compiled = torch.compile(TrainingModule(self.network), fullgraph=True)
        self.model, self.optimizer = self.fabric.setup(compiled, optimizer, _reapply_compile=False)
        self.config = {"batch_size": 16, "micro_batch_size": 4, "precision": "fp32", "value_lambda": 0.333}
        self.mask = torch.from_numpy(policy_mask())
        self.opponent_mask = torch.from_numpy(policy_mask(opponent=True))
        self.batches = np.flatnonzero(~self.data.validation)[:16].reshape(4, 4)

    def accumulate(self):
        return accumulate_gradients(self.fabric, self.model, self.optimizer, self.data, self.batches,
                                    self.config, self.mask, self.opponent_mask, 0)

    def test_gradients_are_the_mean_of_four_microbatches_and_statistics_use_only_the_first(self):
        # 各ミクロバッチを独立したネットで微分し、その勾配の算術平均を参照値とする。
        expected_gradients = {name: torch.zeros_like(value) for name, value in self.reference.named_parameters()}
        expected_losses = torch.zeros(len(LOSS_NAMES))
        for j, batch in enumerate(self.batches):
            reference = copy.deepcopy(self.reference)
            losses = compute_training_losses(
                *_batch(reference.forward_logits, self.data, batch, torch.device("cpu"), self.config),
                _policy_batch(self.data, batch, torch.device("cpu"), "opponent_"),
                torch.from_numpy(self.data.short_term_values[batch]), 0.333, self.mask, self.opponent_mask,
            )
            total_loss(losses).backward()
            for name, parameter in reference.named_parameters():
                expected_gradients[name] += parameter.grad / 4
            expected_losses += torch.stack([loss.detach() for loss in losses]) * 4
            if j == 0:
                expected_buffers = {name: value.clone() for name, value in reference.named_buffers()}
        actual_losses = self.accumulate()
        torch.testing.assert_close(actual_losses, expected_losses)
        for name, parameter in self.network.named_parameters():
            torch.testing.assert_close(parameter.grad, expected_gradients[name])
        for name, value in self.network.named_buffers():
            torch.testing.assert_close(value, expected_buffers[name])
            if name.endswith("num_batches_tracked"):
                self.assertEqual(value.item(), 1)
        self.accumulate()
        for name, parameter in self.network.named_parameters():
            torch.testing.assert_close(parameter.grad, expected_gradients[name])
        for name, value in self.network.named_buffers():
            if name.endswith("num_batches_tracked"):
                self.assertEqual(value.item(), 2)

    def test_every_loss_is_checked_even_when_its_training_weight_is_zero(self):
        for index, name in enumerate(LOSS_NAMES):
            for nonfinite in (float("nan"), float("inf")):
                with self.subTest(loss=name, value=nonfinite):
                    losses = [torch.tensor(1.0, requires_grad=True) for _ in LOSS_NAMES]
                    losses[index] = torch.tensor(nonfinite)
                    with patch("minase_zero.train.compute_training_losses", return_value=tuple(losses)):
                        with self.assertRaisesRegex(ValueError, "non-finite training loss at update 1, rank 0"):
                            self.accumulate()

    def test_each_forward_disables_backward_autocast_and_restores_the_callers_setting(self):
        settings = []

        def batch(*args):
            settings.append(torch._functorch.config.backward_pass_autocast)
            return _batch(*args)

        with torch._functorch.config.patch(backward_pass_autocast="same_as_forward"):
            with patch("minase_zero.train._batch", side_effect=batch):
                self.accumulate()
                self.accumulate()
            self.assertEqual(torch._functorch.config.backward_pass_autocast, "same_as_forward")
        self.assertEqual(settings, ["off"] * 8)
