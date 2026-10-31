# /// script
# requires-python = "==3.13.*"
# dependencies = [
#     "mctx @ git+https://github.com/google-deepmind/mctx@428bfb7",
#     "jax==0.12.0.1",
#     "jaxlib==0.12.0.1",
#     "chex==0.1.92",
#     "numpy==2.5.4",
# ]
# ///
"""mctxのGumbel探索を合成環境で実行し、minaseのMCTSの照合用JSONを書く。

`uv run --script gumbel_reference.py --output <パス>`で実行する。
依存は本ファイルのメタデータと`gumbel_reference.py.lock`で固定し、学習器のロックファイルには入れない。
各節点の方策と価値は、ケースの鍵と状態番号だけから決まる。
Rustの試験は、記録された節点の出力をネットの代わりに使って同じ探索を再現する。
"""

import argparse
from functools import partial
import json
from pathlib import Path

import jax
import jax.numpy as jnp
import mctx
from mctx._src import seq_halving
import numpy as np


MCTX_COMMIT = "428bfb7"
MAX_NUM_CONSIDERED_ACTIONS = 16
CASES = ((24, 32, 0), (5, 32, 1), (24, 64, 2))
CONSIDERED_VISITS = (
    (16, 32),
    (5, 32),
    (16, 64),
    (2, 10),
    (1, 5),
    (3, 7),
    (16, 16),
    (4, 3),
)


def float32_list(values) -> list[float]:
    """単精度の値を、Rustの`f32`で読むと元に戻るPythonの浮動小数にする。"""
    return [float(np.float32(value)) for value in np.asarray(values)]


def export_nodes(tree, num_actions: int, num_simulations: int) -> list[dict]:
    """各シミュレーションが1節点だけを展開したことを検査し、展開の順に節点を書く。"""
    num_nodes = num_simulations + 1
    if tree.node_visits.shape != (1, num_nodes):
        raise RuntimeError(
            f"Expected a batch of 1 with {num_nodes} nodes; "
            f"got node_visits.shape={tree.node_visits.shape}"
        )

    visits = np.asarray(tree.node_visits[0])
    # mctxはn + 1個の節点の領域を先に確保するので、使われた領域の数を数える。
    expanded_nodes = int(np.count_nonzero(visits > 0))
    if expanded_nodes != num_nodes:
        raise RuntimeError(
            f"Expected {num_nodes} expanded nodes; got {expanded_nodes}. "
            "Each simulation must expand exactly one new node."
        )
    if int(visits[0]) != num_nodes:
        raise RuntimeError(f"Expected {num_nodes} root visits; got {visits[0]}")
    root_children_visits = np.asarray(tree.children_visits[0, 0])
    if int(root_children_visits.sum()) != num_simulations:
        raise RuntimeError("Root child visits do not sum to num_simulations")

    parents = np.asarray(tree.parents[0])
    actions = np.asarray(tree.action_from_parent[0])
    children = np.asarray(tree.children_index[0])
    if int(parents[0]) != tree.NO_PARENT:
        raise RuntimeError("The root must have no parent")

    paths: list[list[int]] = [[]]
    for node_index in range(1, num_nodes):
        parent = int(parents[node_index])
        action = int(actions[node_index])
        if not 0 <= parent < node_index:
            raise RuntimeError(
                f"Node {node_index} has parent {parent}; "
                "parents must precede their children in simulation order"
            )
        if not 0 <= action < num_actions:
            raise RuntimeError(f"Node {node_index} has invalid action {action}")
        if int(children[parent, action]) != node_index:
            raise RuntimeError(f"Node {node_index} has an inconsistent parent edge")
        paths.append(paths[parent] + [action])

    return [
        {
            "path": path,
            "prior_logits": float32_list(tree.children_prior_logits[0, node_index]),
            "value": float(np.float32(tree.raw_values[0, node_index])),
        }
        for node_index, path in enumerate(paths)
    ]


def generate_case(
    num_actions: int, num_simulations: int, seed: int, value_scale: float
) -> dict:
    case_key = jax.random.PRNGKey(seed)
    qtransform = partial(
        mctx.qtransform_completed_by_mix_value, value_scale=value_scale
    )

    def evaluate_state(state_id):
        # 出力はケースの鍵と状態番号だけで決まり、探索の順序やrecurrent_fnの鍵に依存しない。
        state_key = jax.random.fold_in(case_key, state_id)
        logits_key, value_key = jax.random.split(state_key)
        prior_logits = jax.random.normal(
            logits_key, shape=(num_actions,), dtype=jnp.float32
        ) * jnp.float32(1.5)
        value = jax.random.uniform(
            value_key,
            shape=(),
            dtype=jnp.float32,
            minval=jnp.float32(-0.9),
            maxval=jnp.float32(0.9),
        )
        return prior_logits, value

    evaluate_batch = jax.vmap(evaluate_state)

    def recurrent_fn(params, rng_key, action, embedding):
        del params, rng_key
        # uint32で折り返して混ぜ、下位31ビットを状態番号にする。
        mixed = (
            embedding.astype(jnp.uint32) * jnp.uint32(1_000_003)
            + action.astype(jnp.uint32) * jnp.uint32(7_919)
            + jnp.uint32(1)
        )
        next_embedding = (mixed & jnp.uint32(0x7FFFFFFF)).astype(jnp.int32)
        prior_logits, value = evaluate_batch(next_embedding)
        return mctx.RecurrentFnOutput(
            reward=jnp.zeros_like(value),
            discount=-jnp.ones_like(value),
            prior_logits=prior_logits,
            value=value,
        ), next_embedding

    # バッチの各要素はint32の状態番号を1つ持つ。
    embedding = jnp.zeros((1,), dtype=jnp.int32)
    prior_logits, value = evaluate_batch(embedding)
    result = mctx.gumbel_muzero_policy(
        params=(),
        rng_key=case_key,
        root=mctx.RootFnOutput(
            prior_logits=prior_logits, value=value, embedding=embedding
        ),
        recurrent_fn=recurrent_fn,
        num_simulations=num_simulations,
        max_num_considered_actions=MAX_NUM_CONSIDERED_ACTIONS,
        gumbel_scale=1.0,
        qtransform=qtransform,
    )
    # qtransformはバッチなしの木と節点番号を受け取る。
    root_completed_qvalues = jax.vmap(
        qtransform, in_axes=(0, None)
    )(result.search_tree, 0)[0]
    result = jax.device_get(result)
    tree = result.search_tree

    return {
        "name": f"a{num_actions}-n{num_simulations}-s{value_scale}",
        "num_actions": num_actions,
        "num_simulations": num_simulations,
        "value_scale": value_scale,
        "max_num_considered_actions": MAX_NUM_CONSIDERED_ACTIONS,
        "seed": seed,
        "root_gumbel": float32_list(tree.extra_data.root_gumbel[0]),
        "nodes": export_nodes(tree, num_actions, num_simulations),
        "expected": {
            "action": int(result.action[0]),
            "action_weights": float32_list(result.action_weights[0]),
            "root_children_visits": [
                int(count) for count in tree.children_visits[0, 0]
            ],
            "root_completed_qvalues": float32_list(root_completed_qvalues),
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output", type=Path, required=True, help="書き出すJSONのパス"
    )
    args = parser.parse_args()

    # 配列やバックエンドを作る前にJAXを設定する。
    jax.config.update("jax_platforms", "cpu")
    jax.config.update("jax_enable_x64", False)

    reference = {
        "generator": {
            "script": "tools/train/zero/gumbel_reference.py",
            "mctx_commit": MCTX_COMMIT,
            "jax": jax.__version__,
            "numpy": np.__version__,
        },
        "considered_visits": [
            {
                "max_num_considered_actions": m,
                "num_simulations": n,
                "sequence": list(seq_halving.get_sequence_of_considered_visits(m, n)),
            }
            for m, n in CONSIDERED_VISITS
        ],
        "cases": [
            generate_case(a, n, seed, value_scale)
            for value_scale in (0.1, 1.0)
            for a, n, seed in CASES
        ],
    }
    serialized = json.dumps(reference, indent=2, allow_nan=False) + "\n"
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(serialized, encoding="utf-8")
    print(f"{args.output} {len(reference['cases'])}")


if __name__ == "__main__":
    main()
