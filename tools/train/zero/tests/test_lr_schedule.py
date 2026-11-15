"""学習率の境界と、実際の更新・保存・再開で累計更新数が続くことを検証する。"""

import sys
import unittest

from minase_zero import train
from minase_zero.__main__ import _parse
from tests.cli import require_success, run_command


class LearningRateBoundaryTests(unittest.TestCase):
    def test_initial_update_and_warmup_boundary(self):
        # フェーズ1指示書のη_base×min(1,(s+1)/250)を境界の手計算値で検査する。
        for step, expected in ((0, 0.04 / 250), (248, 0.04 * 249 / 250),
                               (249, 0.04), (250, 0.04), (10_000, 0.04)):
            with self.subTest(optimizer_steps=step):
                self.assertAlmostEqual(train.learning_rate(0.04, step), expected, places=15)

    def test_resume_requires_an_explicit_base_learning_rate(self):
        arguments = ["train", "--resume", "/unused", "--device", "cpu", "--replay-threads", "1"]
        with self.assertRaisesRegex(ValueError, "--lr"):
            _parse(arguments)
        self.assertEqual(_parse([*arguments, "--lr", "0.04"]).lr, 0.04)


class DistributedLearningRateTests(unittest.TestCase):
    def test_learning_rate_contract_in_torchrun(self):
        result = run_command(
            [sys.executable, "-m", "torch.distributed.run", "--standalone", "--nproc-per-node", "1",
             "-m", "tests.lr_worker"],
        )
        require_success(result)
