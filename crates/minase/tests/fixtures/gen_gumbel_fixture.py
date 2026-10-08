"""Gumbel探索の参照値を、DeepMindのmctxで小さな表の木について生成する。

docs/plans/alphazero.md「検証」の節の「固定の小さな探索木について、Gumbel探索の訪問の順序と
方策の目標がmctxの出力と一致することを確かめる」試験の参照値を、
`crates/minase/tests/fixtures/gumbel_mctx.json`へ書く。
Rust側の試験は`crates/minase/src/search/mcts/tests.rs`の`gumbel_search_matches_mctx`である。

木の各節点は根からの手順（行動番号の列）で表し、節点の方策logitと手番側の勝率は
手順から決まる乱数で作る。勝率pは手番側から見た[0, 1]の値であり、mctxへは
報酬1、割引−1として渡すので、mctxの行動価値は 1 − (子の価値) となり、
Rust側の「親の手番側から見た勝率」と同じ尺度になる。
logitと勝率はfloat32で表せる値に丸めてから64ビットでmctxへ渡し、JSONにも同じ値を書く。

方策の変換σの係数は設計書のとおり c_visit = 50、c_scale = 1.0 とする。
mctxの`qtransform_completed_by_mix_value`の既定の`value_scale`は0.1なので、1.0を明示して渡す。

実行方法（mctx 0.0.71を導入した環境で）:

    python crates/minase/tests/fixtures/gen_gumbel_fixture.py
"""

import json
from functools import partial
from pathlib import Path

import jax

jax.config.update("jax_enable_x64", True)

import jax.numpy as jnp  # noqa: E402
import mctx  # noqa: E402
import numpy as np  # noqa: E402

OUTPUT = Path(__file__).with_name("gumbel_mctx.json")
# 埋め込みに入れる手順の最大長。探索回数以下の深さしか生じない。
MAX_PATH = 128
C_VISIT = 50.0
C_SCALE = 1.0

# (名前, 行動数, 探索回数n, 候補数の上限m, 乱数のシード)
SCENARIOS = [
    ("wide", 20, 32, 16, 1),
    ("narrow", 5, 16, 16, 2),
    ("deep", 3, 64, 16, 3),
]


def node_row(seed: int, path: tuple[int, ...], num_actions: int) -> tuple[list[float], float]:
    """手順から決まる方策logitと手番側の勝率を返す。"""
    rng = np.random.default_rng([seed, len(path), *path])
    logits = rng.normal(0.0, 1.0, num_actions).astype(np.float32)
    value = np.float32(rng.uniform(0.05, 0.95))
    return [float(x) for x in logits], float(value)


def pad(path: tuple[int, ...]) -> np.ndarray:
    row = np.full(MAX_PATH, -1, dtype=np.int64)
    row[: len(path)] = path
    return row


def unpad(row) -> tuple[int, ...]:
    return tuple(int(x) for x in np.asarray(row) if x >= 0)


def python_loop(lower, upper, body, state):
    """recurrent_fnが具体的な値を受けるよう、シミュレーションをPythonのループで回す。"""
    for index in range(lower, upper):
        state = body(index, state)
    return state


def run(name: str, num_actions: int, num_simulations: int, max_considered: int, seed: int) -> dict:
    table: dict[tuple[int, ...], tuple[list[float], float]] = {}

    def lookup(path: tuple[int, ...]) -> tuple[list[float], float]:
        if path not in table:
            table[path] = node_row(seed, path, num_actions)
        return table[path]

    root_logits, root_value = lookup(())
    root = mctx.RootFnOutput(
        prior_logits=jnp.asarray([root_logits], dtype=jnp.float64),
        value=jnp.asarray([root_value], dtype=jnp.float64),
        embedding=jnp.asarray([pad(())]),
    )

    def recurrent_fn(params, rng_key, action, embedding):
        del params, rng_key
        path = unpad(embedding[0]) + (int(action[0]),)
        logits, value = lookup(path)
        output = mctx.RecurrentFnOutput(
            reward=jnp.asarray([1.0], dtype=jnp.float64),
            discount=jnp.asarray([-1.0], dtype=jnp.float64),
            prior_logits=jnp.asarray([logits], dtype=jnp.float64),
            value=jnp.asarray([value], dtype=jnp.float64),
        )
        return output, jnp.asarray([pad(path)])

    qtransform = partial(
        mctx.qtransform_completed_by_mix_value,
        value_scale=C_SCALE,
        maxvisit_init=C_VISIT,
    )
    output = mctx.gumbel_muzero_policy(
        params=(),
        rng_key=jax.random.PRNGKey(seed),
        root=root,
        recurrent_fn=recurrent_fn,
        num_simulations=num_simulations,
        loop_fn=python_loop,
        qtransform=qtransform,
        max_num_considered_actions=max_considered,
    )
    tree = output.search_tree
    embeddings = np.asarray(tree.embeddings[0])
    # 節点番号k（1以上）は、k−1番目のシミュレーションが展開した節点である。
    visit_order = [list(unpad(embeddings[index])) for index in range(1, num_simulations + 1)]
    return {
        "name": name,
        "num_actions": num_actions,
        "num_simulations": num_simulations,
        "max_considered": max_considered,
        "c_visit": C_VISIT,
        "c_scale": C_SCALE,
        "gumbel": [float(x) for x in np.asarray(tree.extra_data.root_gumbel[0])],
        "table": [
            {"path": list(path), "logits": logits, "value": value}
            for path, (logits, value) in sorted(table.items(), key=lambda item: (len(item[0]), item[0]))
        ],
        "visit_order": visit_order,
        "root_visits": [int(x) for x in np.asarray(tree.children_visits[0, 0])],
        "action": int(output.action[0]),
        "action_weights": [float(x) for x in np.asarray(output.action_weights[0])],
    }


def main() -> None:
    scenarios = [run(*scenario) for scenario in SCENARIOS]
    document = {"generator": "mctx " + mctx.__version__, "scenarios": scenarios}
    OUTPUT.write_text(json.dumps(document, indent=1) + "\n")


if __name__ == "__main__":
    main()
