"""STCの対局の局面で、1手のシミュレーション数と毎秒の回数をバッチごとに測る。

引数はminaseのバイナリ、ネット、実行ディレクトリ、およびバッチの列である。
"""

import statistics as st
import sys

from common import load_pair, opening_moves, pair_files
from engine import Engine

binary, net, run = sys.argv[1:4]
pair = load_pair(pair_files(run)[0])
moves = opening_moves(pair) + [t["response"]["usi"] for t in pair["games"][0]["turns"] if t["response"].get("kind") == "move"]
plies = [x for x in range(20, 201, 20) if x < len(moves)]
for batch in map(int, sys.argv[4:]):
    engine = Engine(binary, net, batch)
    for name, go in (("clock", "go btime 8000 wtime 8000 binc 100 winc 100"), ("nodes800", "go nodes 800")):
        results = [engine.search(moves[:ply], go) for ply in plies]
        nodes = [r[1] for r in results]
        times = [r[2] for r in results]
        rate = st.median(n / max(t, 1) * 1000 for n, t in zip(nodes, times))
        print(f"batch {batch} {name}: nodes median {st.median(nodes)} min {min(nodes)} max {max(nodes)}; "
              f"time median {st.median(times)} ms; simulations per second median {rate:.0f}", flush=True)
    engine.quit()
