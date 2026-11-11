"""設計書の価値logitの契約と、onnx.rsの入出力契約を検証する。"""

import importlib.util
from pathlib import Path
import tempfile
import unittest

import numpy as np
import onnx
import torch

from minase_zero.encoding import INPUT_PLANES, policy_mask
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
        self.assertEqual(policy.dtype, torch.float32)
        self.assertEqual(value.dtype, torch.float32)

    def test_onnx_names_shapes_types_and_dynamic_batch(self) -> None:
        with tempfile.TemporaryDirectory(prefix="minase-zero-net-") as directory:
            path = Path(directory) / "model.onnx"
            network = ZeroNet(1, 16).eval()
            export_onnx(network, path)
            self.assertFalse(network.training)
            model = onnx.load(path)
        self.assertFalse(any("soft_policy" in entry.name or "opponent_policy" in entry.name
                             for entry in model.graph.initializer))
        value_linear = [node for node in model.graph.node if node.op_type == "Gemm"][-1]
        weights = next(entry for entry in model.graph.initializer if entry.name == value_linear.input[1])
        self.assertEqual(list(weights.dims), [1, 256])
        self.assertEqual([(op.domain, op.version) for op in model.opset_import], [("", 17)])
        self.assertEqual([entry.name for entry in model.graph.input], ["input"])
        self.assertEqual([entry.name for entry in model.graph.output], ["policy", "value"])
        for entry, shape, dtype in zip(
            [*model.graph.input, *model.graph.output],
            [(159, 12, 12), (16_108,), (1,)],
            [onnx.TensorProto.UINT8, onnx.TensorProto.FLOAT16, onnx.TensorProto.FLOAT],
        ):
            tensor = entry.type.tensor_type
            self.assertEqual(tensor.elem_type, dtype)
            self.assertEqual(tensor.shape.dim[0].dim_param, "batch")
            self.assertEqual(tuple(dim.dim_value for dim in tensor.shape.dim[1:]), shape)
        casts = [node for node in model.graph.node
                 if node.op_type == "Cast" and list(node.input) == ["input"]]
        self.assertEqual(len(casts), 1)
        self.assertEqual(next(attr.i for attr in casts[0].attribute if attr.name == "to"),
                         onnx.TensorProto.FLOAT)
        policy_cast = next(node for node in model.graph.node if list(node.output) == ["policy"])
        self.assertEqual(policy_cast.op_type, "Cast")
        self.assertEqual(next(attr.i for attr in policy_cast.attribute if attr.name == "to"),
                         onnx.TensorProto.FLOAT16)
        # 書き出し契約: 固定の表で軸1を詰め、その後にFP16へ変換する。
        gather = next(node for node in model.graph.node if node.output[0] == policy_cast.input[0])
        self.assertEqual(gather.op_type, "Gather")
        self.assertEqual(next(attr.i for attr in gather.attribute if attr.name == "axis"), 1)
        labels = np.flatnonzero(policy_mask())
        table = next(entry for entry in model.graph.initializer if entry.name == gather.input[1])
        np.testing.assert_array_equal(onnx.numpy_helper.to_array(table), labels)
        self.assertEqual(
            [entry.value for entry in model.metadata_props if entry.key == "policy_labels"],
            [",".join(str(label) for label in labels)],
        )

    def test_repeated_export_has_identical_bytes(self) -> None:
        torch.manual_seed(19)
        network = ZeroNet(1, 16).eval()
        with tempfile.TemporaryDirectory(prefix="minase-zero-net-") as directory:
            first = Path(directory) / "first.onnx"
            second = Path(directory) / "second.onnx"
            export_onnx(network, first)
            export_onnx(network, second)
            self.assertEqual(first.read_bytes(), second.read_bytes())

    @unittest.skipUnless(importlib.util.find_spec("onnxruntime"), "onnxruntimeが未導入のためCPU推論を検証できない")
    def test_onnx_policy_matches_rounded_forward_logits(self) -> None:
        import onnxruntime as ort

        torch.manual_seed(11)
        network = ZeroNet(1, 16).eval()
        with tempfile.TemporaryDirectory(prefix="minase-zero-net-") as directory:
            path = Path(directory) / "model.onnx"
            export_onnx(network, path)
            options = ort.SessionOptions()
            options.intra_op_num_threads = 1
            session = ort.InferenceSession(str(path), sess_options=options, providers=["CPUExecutionProvider"])
            for batch in (1, 3):
                with self.subTest(batch=batch):
                    inputs = torch.randint(0, 2, (batch, INPUT_PLANES, 12, 12), dtype=torch.uint8)
                    with torch.inference_mode():
                        policy, value = network(inputs.float())
                    actual_policy, actual_value = session.run(["policy", "value"], {"input": inputs.numpy()})
                    self.assertEqual(actual_policy.dtype, np.dtype("float16"))
                    self.assertEqual(actual_value.dtype, np.dtype("float32"))
                    rounded = policy.numpy()[:, np.flatnonzero(policy_mask())].astype(np.float16)
                    self.assertEqual(actual_policy.shape, (batch, 16_108))
                    # 正の側の隣接値との差が、丸めた参照におけるFP16の1目盛りになる。
                    magnitude = np.abs(rounded)
                    step = (np.nextafter(magnitude, np.float16(np.inf)).astype(np.float64)
                            - magnitude.astype(np.float64))
                    difference = np.abs(actual_policy.astype(np.float64) - rounded.astype(np.float64))
                    self.assertTrue(np.all(difference <= 1e-4 + step), float(difference.max()))
                    np.testing.assert_allclose(actual_value, value.numpy(), atol=1e-4, rtol=0)


if __name__ == "__main__":
    unittest.main()
