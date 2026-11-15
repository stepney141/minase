"""設計書の価値logitの契約と、onnx.rsの入出力契約を検証する。"""

import importlib.util
from pathlib import Path
import tempfile
import unittest

import numpy as np
import onnx
import torch
from torch import nn

from minase_zero.encoding import INPUT_PLANES, policy_mask
from minase_zero.net import ResidualBlock, ZeroNet, export_onnx


class NetTests(unittest.TestCase):
    def setUp(self) -> None:
        previous = torch.get_num_threads()
        torch.set_num_threads(1)
        self.addCleanup(torch.set_num_threads, previous)

    def test_twenty_blocks_256_channels_have_the_specified_parameter_count(self) -> None:
        # 設計書とフェーズ1指示書の内訳。バッチ正規化の移動統計は学習パラメータに含めない。
        model = ZeroNet(20, 256)
        count = lambda module: sum(parameter.numel() for parameter in module.parameters())
        self.assertEqual(count(model.trunk[:3]), 366_848)
        self.assertEqual(count(model.trunk[3:]), 23_613_440)
        self.assertEqual(count(model.policy_head), 1_164_281)
        self.assertEqual(count(model.value_head), 37_635)
        self.assertEqual(count(model), 25_182_204)

    def test_trunk_and_heads_follow_the_alphazero_architecture(self) -> None:
        model = ZeroNet(2, 4)
        self.assertEqual([type(layer) for layer in model.trunk],
                         [nn.Conv2d, nn.BatchNorm2d, nn.ReLU, ResidualBlock, ResidualBlock])
        stem = model.trunk[0]
        self.assertEqual((stem.in_channels, stem.out_channels, stem.kernel_size, stem.padding),
                         (159, 4, (3, 3), (1, 1)))
        self.assertIsNone(stem.bias)
        for block in model.trunk[3:]:
            self.assertFalse(hasattr(block, "pool_bias"))
            self.assertEqual([type(layer) for layer in block.children()],
                             [nn.Conv2d, nn.BatchNorm2d, nn.Conv2d, nn.BatchNorm2d])
            for conv in (block.conv1, block.conv2):
                self.assertEqual((conv.in_channels, conv.out_channels, conv.kernel_size, conv.padding),
                                 (4, 4, (3, 3), (1, 1)))
                self.assertIsNone(conv.bias)
            # 2層目が0なら、残差接続と最後のReLUだけが残る。
            with torch.no_grad():
                block.conv2.weight.zero_()
                inputs = torch.randn(2, 4, 12, 12)
                torch.testing.assert_close(block(inputs), inputs.relu())
        self.assertEqual([type(layer) for layer in model.policy_head],
                         [nn.Conv2d, nn.BatchNorm2d, nn.ReLU, nn.Conv2d])
        first, bn, _, final = model.policy_head
        self.assertEqual((first.in_channels, first.out_channels, first.kernel_size, first.padding),
                         (4, 4, (3, 3), (1, 1)))
        self.assertIsNone(first.bias)
        self.assertEqual(bn.num_features, 4)
        self.assertEqual((final.in_channels, final.out_channels, final.kernel_size, final.padding),
                         (4, 249, (3, 3), (1, 1)))
        self.assertIsNotNone(final.bias)
        self.assertEqual([type(layer) for layer in model.value_head],
                         [nn.Conv2d, nn.BatchNorm2d, nn.ReLU, nn.Flatten, nn.Linear, nn.ReLU, nn.Linear])
        conv, bn, _, flatten, hidden, _, final = model.value_head
        self.assertEqual((conv.in_channels, conv.out_channels, conv.kernel_size), (4, 1, (1, 1)))
        self.assertIsNone(conv.bias)
        self.assertEqual(bn.num_features, 1)
        self.assertEqual(flatten.start_dim, 1)
        self.assertEqual((hidden.in_features, hidden.out_features), (144, 256))
        self.assertEqual((final.in_features, final.out_features), (256, 1))

    def test_forward_logits_preserves_policy_and_sigmoid_value(self) -> None:
        torch.manual_seed(7)
        model = ZeroNet(1, 16).eval()
        inputs = torch.randn(3, INPUT_PLANES, 12, 12)
        with torch.inference_mode():
            policy, value = model(inputs)
            logits = model.forward_logits(inputs)
        self.assertTrue(torch.equal(policy, logits.policy))
        torch.testing.assert_close(value, logits.value.sigmoid())
        self.assertEqual(len(logits), 2)
        self.assertEqual(logits.value.shape, (3, 1))
        self.assertTrue(torch.all((value >= 0) & (value <= 1)))
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
