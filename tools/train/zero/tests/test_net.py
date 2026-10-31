"""設計書の価値logitの契約と、onnx.rsの入出力契約を検証する。"""

from pathlib import Path
import tempfile
import unittest

import onnx
import torch

from minase_zero.encoding import INPUT_PLANES
from minase_zero.net import ZeroNet, export_onnx


class NetTests(unittest.TestCase):
    def setUp(self) -> None:
        previous = torch.get_num_threads()
        torch.set_num_threads(1)
        self.addCleanup(torch.set_num_threads, previous)

    def test_ten_blocks_128_channels_count_all_training_heads(self) -> None:
        # 入力183,424、残差328,320×10、方策32,121×4、価値1,185,092。
        model = ZeroNet(10, 128)
        self.assertEqual(sum(parameter.numel() for parameter in model.parameters()), 4_780_200)

    def test_forward_logits_preserves_policy_and_sigmoid_value(self) -> None:
        torch.manual_seed(7)
        model = ZeroNet(1, 16).eval()
        inputs = torch.randn(3, INPUT_PLANES, 12, 12)
        with torch.inference_mode():
            policy, value = model(inputs)
            logits = model.forward_logits(inputs)
        self.assertTrue(torch.equal(policy, logits.policy))
        torch.testing.assert_close(value, logits.value[:, :1].sigmoid())
        self.assertEqual(logits.value.shape, (3, 4))
        for auxiliary in logits[2:]:
            self.assertEqual(auxiliary.shape, (3, 35_856))
        self.assertEqual(model.value_head[0].out_channels, 32)
        self.assertIsNone(model.value_head[0].bias)
        self.assertEqual((model.value_head[4].in_features, model.value_head[4].out_features), (4608, 256))
        self.assertEqual(policy.shape, (3, 35_856))
        self.assertEqual(value.shape, (3, 1))

    def test_onnx_names_shapes_types_and_dynamic_batch(self) -> None:
        with tempfile.TemporaryDirectory(prefix="minase-zero-net-") as directory:
            path = Path(directory) / "model.onnx"
            export_onnx(ZeroNet(1, 16).eval(), path)
            model = onnx.load(path)
        self.assertFalse(any("soft_policy" in entry.name or "opponent_policy" in entry.name
                             for entry in model.graph.initializer))
        value_linear = [node for node in model.graph.node if node.op_type == "Gemm"][-1]
        weights = next(entry for entry in model.graph.initializer if entry.name == value_linear.input[1])
        self.assertEqual(list(weights.dims), [1, 256])
        self.assertEqual([(op.domain, op.version) for op in model.opset_import], [("", 17)])
        self.assertEqual([entry.name for entry in model.graph.input], ["input"])
        self.assertEqual([entry.name for entry in model.graph.output], ["policy", "value"])
        for entry, shape in zip(
            [*model.graph.input, *model.graph.output], [(159, 12, 12), (35_856,), (1,)],
        ):
            tensor = entry.type.tensor_type
            self.assertEqual(tensor.elem_type, onnx.TensorProto.FLOAT)
            self.assertEqual(tensor.shape.dim[0].dim_param, "batch")
            self.assertEqual(tuple(dim.dim_value for dim in tensor.shape.dim[1:]), shape)


if __name__ == "__main__":
    unittest.main()
