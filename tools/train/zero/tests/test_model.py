"""ONNXへ出力したネットの形状、方策の並び、価値の範囲を検査する。"""

from __future__ import annotations

import numpy as np
import onnxruntime as ort
import torch
from torch import nn

from minase_zero.encoding import BOARD_SIZE, INPUT_PLANES, POLICY_PLANES, POLICY_SIZE
from minase_zero.export import export_onnx
from minase_zero.model import ZeroNet


def cpu_session(model: ZeroNet, tmp_path) -> ort.InferenceSession:
    """ネットをONNXへ出力し、CPU実行プロバイダのセッションを作る。"""
    path = tmp_path / "net.onnx"
    export_onnx(model, path)
    return ort.InferenceSession(str(path), providers=["CPUExecutionProvider"])


def random_inputs(batch: int, seed: int) -> np.ndarray:
    """0か1の要素からなる入力平面を作る。"""
    rng = np.random.default_rng(seed)
    shape = (batch, INPUT_PLANES, BOARD_SIZE, BOARD_SIZE)
    return (rng.random(shape) < 0.05).astype(np.float32)


def test_onnx_interface_has_fixed_names_and_dynamic_batch(tmp_path):
    """入力名と出力名が固定で、任意のバッチの大きさで方策と価値の形状が正しい。"""
    torch.manual_seed(0)
    session = cpu_session(ZeroNet(2, 16), tmp_path)
    assert [node.name for node in session.get_inputs()] == ["input"]
    assert [node.name for node in session.get_outputs()] == ["policy", "value"]
    for batch in (1, 7, 64):
        policy, value = session.run(None, {"input": random_inputs(batch, batch)})
        assert policy.shape == (batch, POLICY_SIZE)
        assert value.shape == (batch, 1)


def test_policy_order_matches_rust_labels(tmp_path):
    """1つの升と面のlogitだけを大きくした重みで、ONNXの方策の並びが`升 * 249 + 面`であることを確かめる。

    畳み込みと全結合の重みをすべて0にし、BatchNormは初期値（恒等写像）のまま残す。
    入力層は入力の面0をチャネル0へそのまま写し、残差ブロックは0の重みで恒等写像になるので、
    胴体のチャネル0は入力の面0と同じ升だけが正になる。方策の1×1畳み込みはチャネル0を面pへ写す。
    """
    model = ZeroNet(2, 4)
    with torch.no_grad():
        for module in model.modules():
            if isinstance(module, (nn.Conv2d, nn.Linear)):
                module.weight.zero_()
                if module.bias is not None:
                    module.bias.zero_()
        model.input_conv.weight[0, 0, 1, 1] = 1.0
    # 段と筋、升と面を取り違えると別の位置になるように、非対称な升と面を選ぶ。
    targets = ((2, 9, 200), (7, 0, 5))
    inputs = np.zeros((len(targets), INPUT_PLANES, BOARD_SIZE, BOARD_SIZE), dtype=np.float32)
    for index, (rank, file, plane) in enumerate(targets):
        inputs[index, 0, rank, file] = 1.0
        with torch.no_grad():
            model.policy_conv.weight[plane, 0, 0, 0] = 100.0
    session = cpu_session(model, tmp_path)
    policy, _ = session.run(None, {"input": inputs})
    planes = [plane for _, _, plane in targets]
    for index, (rank, file, _) in enumerate(targets):
        square = rank * BOARD_SIZE + file
        expected = sorted(square * POLICY_PLANES + plane for plane in planes)
        assert sorted(np.flatnonzero(policy[index] > 1.0).tolist()) == expected


def test_value_is_a_probability(tmp_path):
    """価値は大きな入力に対しても0〜1に収まる。"""
    torch.manual_seed(1)
    session = cpu_session(ZeroNet(2, 16), tmp_path)
    inputs = random_inputs(32, 2) * 1000.0 - 500.0
    _, value = session.run(None, {"input": inputs})
    assert np.all((value >= 0.0) & (value <= 1.0))


def test_onnx_matches_pytorch(tmp_path):
    """ONNX RuntimeのCPUの出力が、PyTorchの出力とバッチの大きさ1、7、256で一致する。"""
    torch.manual_seed(2)
    model = ZeroNet(2, 16)
    model.eval()
    session = cpu_session(model, tmp_path)
    for batch in (1, 7, 256):
        inputs = random_inputs(batch, batch)
        with torch.no_grad():
            expected_policy, expected_value = model(torch.from_numpy(inputs))
        policy, value = session.run(None, {"input": inputs})
        np.testing.assert_allclose(policy, expected_policy.numpy(), atol=1e-4, rtol=0)
        np.testing.assert_allclose(value, expected_value.numpy(), atol=1e-4, rtol=0)
