"""チェックポイントからこの機械でONNXを書き出し直し、学習した機械が書いたONNXと比べる。

使い方（tools/train/zero を作業ディレクトリにして）:
uv run --frozen python <本ファイル> <checkpoint.pt> <model.onnx>
計算グラフの節点と初期値の名前の一致、および初期値ごとの差の絶対値の最大を表示する。
"""

import sys
import tempfile
from pathlib import Path

import numpy as np
import onnx
from onnx import numpy_helper

sys.path.insert(0, ".")
from minase_zero.net import export_onnx  # noqa: E402
from minase_zero.train import _load_checkpoint, _model  # noqa: E402

checkpoint, reference = Path(sys.argv[1]), Path(sys.argv[2])
model = _model(_load_checkpoint(checkpoint)).eval()
with tempfile.TemporaryDirectory() as directory:
    path = Path(directory) / "model.onnx"
    export_onnx(model, path)
    print("bytes_equal", path.read_bytes() == reference.read_bytes())
    a, b = onnx.load(str(reference)), onnx.load(str(path))
    initial_a = {item.name: numpy_helper.to_array(item) for item in a.graph.initializer}
    initial_b = {item.name: numpy_helper.to_array(item) for item in b.graph.initializer}
    print("initializer_names_equal", list(initial_a) == list(initial_b), len(initial_a))
    differences = sorted(
        ((float(np.max(np.abs(initial_a[name].astype(np.float64) - initial_b[name].astype(np.float64)))), name)
         for name in initial_a if initial_a[name].size),
        reverse=True,
    )
    nonzero = [item for item in differences if item[0] > 0]
    print("differing_initializers", len(nonzero), "largest", nonzero[:5])
    nodes_a = [(node.op_type, list(node.input), list(node.output)) for node in a.graph.node]
    nodes_b = [(node.op_type, list(node.input), list(node.output)) for node in b.graph.node]
    print("nodes_equal", nodes_a == nodes_b, len(nodes_a))
