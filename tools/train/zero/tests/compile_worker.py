"""独立したCPUプロセスで1サイクルを学習し、コンパイルの回数を記録する。

ソケットを使わず、SingleDeviceStrategyで本番の読み込み、更新、保存、最終診断を実行する。
コンパイルの回数は学習器の外で数え、初期化後からONNXの書き出しまでを含める。
"""

from contextlib import nullcontext
import json
from pathlib import Path
import sys
from unittest.mock import patch

from lightning_fabric import Fabric
from lightning_fabric.strategies import SingleDeviceStrategy
import torch
import torch._inductor.config
from torch._dynamo.utils import counters

from minase_zero import train
from minase_zero.__main__ import _parse

from tests.cli import init_arguments, train_arguments


def main() -> None:
    root = Path(sys.argv[1])
    initial = root / "initial"
    arguments = vars(_parse(init_arguments(initial, 23)))
    del arguments["command"]
    train.initialize(**arguments)
    arguments = vars(_parse(train_arguments(initial / "checkpoint.pt", root / "trained")))
    del arguments["command"]
    out = arguments.pop("out_dir")
    fabric = Fabric(accelerator="cpu", devices=1, precision="32-true",
                    strategy=SingleDeviceStrategy(device=torch.device("cpu")))
    fabric.launch()
    torch._dynamo.reset()
    counters.clear()
    with (patch.object(train, "training_fabric", return_value=nullcontext(fabric)),
          patch.object(fabric, "setup", wraps=fabric.setup) as setup):
        train.train_new(arguments, out)
    (root / "compile-count.json").write_text(json.dumps({
        "unique_graphs": counters["stats"]["unique_graphs"],
        "graph_breaks": sum(counters["graph_break"].values()),
        "reapply_compile": setup.call_args.kwargs["_reapply_compile"],
        "emulate_precision_casts": torch._inductor.config.emulate_precision_casts,
    }), encoding="utf-8")


if __name__ == "__main__":
    main()
