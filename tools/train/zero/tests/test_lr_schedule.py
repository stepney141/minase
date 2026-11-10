"""学習率の契約をtorchrunのCPUワーカーで検証する。"""

import sys
import unittest

from tests.cli import require_success, run_command


class LearningRateTests(unittest.TestCase):
    def test_learning_rate_contract_in_torchrun(self):
        result = run_command(
            [sys.executable, "-m", "torch.distributed.run", "--standalone", "--nproc-per-node", "1",
             "-m", "tests.lr_worker"],
        )
        require_success(result)
