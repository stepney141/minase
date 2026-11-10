"""本番の勾配累積をtorchrunで呼び、クリップ前の勾配とBatchNormの統計を保存する。"""

import copy
from pathlib import Path
import sys

import numpy as np
import torch

from minase_zero.data import read_replay_buffer
from minase_zero.encoding import encode_planes, policy_mask
from minase_zero.net import ZeroNet
from minase_zero.train import TrainingModule, accumulate_gradients, training_fabric


def main():
    out = Path(sys.argv[1])
    config = {"batch_size": 16, "micro_batch_size": 4, "precision": "fp32", "value_lambda": 0.333}
    with training_fabric("cpu", "fp32") as fabric:
        torch.manual_seed(23)
        network = ZeroNet(1, 16).train()
        reference = copy.deepcopy(network)
        initial_buffers = {name: value.clone() for name, value in network.named_buffers()}
        optimizer = torch.optim.SGD(network.parameters(), lr=0.01, momentum=0.9, nesterov=True)
        compiled = torch.compile(TrainingModule(network), fullgraph=True)
        model, optimizer = fabric.setup(compiled, optimizer, _reapply_compile=False)
        data, _, selected, retained = read_replay_buffer(
            ["fixtures/zero-selfplay-small.mnsd"], ["fixtures/zero-selfplay-small.mnsd"], 42,
            rank=fabric.global_rank, world_size=fabric.world_size,
            batch_size=16, micro_batch_size=4, replay_threads=1,
        )
        training_rows = np.flatnonzero(~data.validation)
        assigned = selected[:16].reshape(4, 4)[fabric.global_rank::fabric.world_size]
        batches = training_rows[np.searchsorted(retained, assigned)]
        sums = accumulate_gradients(
            fabric, model, optimizer, data, batches, config,
            torch.from_numpy(policy_mask()), torch.from_numpy(policy_mask(opponent=True)), 0,
        )
        if fabric.global_rank == 0:
            # 仕様のj=0だけで、独立したネットの移動統計を1回更新する。
            rows = training_rows[np.searchsorted(retained, selected[:4])]
            inputs = encode_planes(*(torch.from_numpy(column[rows]) for column in
                                     (data.boards, data.sides, data.lions)))
            with torch.no_grad():
                reference.forward_logits(inputs)
            expected = dict(reference.named_buffers())
        else:
            expected = initial_buffers
        torch.save({"gradients": {name: parameter.grad for name, parameter in network.named_parameters()},
                    "buffers": dict(network.named_buffers()), "expected_buffers": expected,
                    "loss_sums": sums}, out / f"rank-{fabric.global_rank}.pt")


if __name__ == "__main__":
    main()
