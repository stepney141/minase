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

    def test_forward_logits_preserves_policy_and_sigmoid_value(self) -> None:
        torch.manual_seed(7)
        model = ZeroNet(1, 16).eval()
        inputs = torch.randn(3, INPUT_PLANES, 12, 12)
        with torch.inference_mode():
            policy, value = model(inputs)
            policy_logits, value_logits = model.forward_logits(inputs)
        self.assertTrue(torch.equal(policy, policy_logits))
        self.assertTrue(torch.equal(value, value_logits.sigmoid()))
        self.assertIsInstance(model.value_head[-1], torch.nn.Sigmoid)
        self.assertEqual(policy.shape, (3, 35_856))
        self.assertEqual(value.shape, (3, 1))

    def test_onnx_names_shapes_types_and_dynamic_batch(self) -> None:
        with tempfile.TemporaryDirectory(prefix="minase-zero-net-") as directory:
            path = Path(directory) / "model.onnx"
            export_onnx(ZeroNet(1, 16).eval(), path)
            model = onnx.load(path)
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
