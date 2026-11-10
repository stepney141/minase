"""torchrunのCPUワーカーで、正常終了時だけglooのプロセス群を破棄することを確かめる。"""

import unittest
from unittest.mock import patch

import torch.distributed as distributed

from minase_zero.train import training_fabric


class FabricCleanupTests(unittest.TestCase):
    def test_exception_preserves_process_group_and_propagates(self):
        error = ValueError("non-finite loss")
        try:
            with patch.object(distributed, "destroy_process_group") as destroy:
                with self.assertRaises(ValueError) as raised:
                    with training_fabric("cpu", "fp32") as fabric:
                        self.assertEqual(fabric.world_size, 1)
                        self.assertEqual(distributed.get_backend(), "gloo")
                        raise error
                self.assertIs(raised.exception, error)
                destroy.assert_not_called()
                self.assertTrue(distributed.is_initialized())
        finally:
            if distributed.is_initialized():
                distributed.destroy_process_group()

    def test_success_destroys_process_group(self):
        with patch.object(distributed, "destroy_process_group",
                          wraps=distributed.destroy_process_group) as destroy:
            with training_fabric("cpu", "fp32"):
                self.assertTrue(distributed.is_initialized())
                destroy.assert_not_called()
            destroy.assert_called_once_with()
        self.assertFalse(distributed.is_initialized())


if __name__ == "__main__":
    unittest.main()
