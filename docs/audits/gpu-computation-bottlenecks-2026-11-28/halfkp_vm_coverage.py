"""HalfKP_vmの第1層の行のうち、学習した局面で1度以上現れた行の割合を、局面数ごとに数える。

使い方: リポジトリの根を作業ディレクトリとし、tools/train/zero/.venv/bin/python <このファイル>
特徴の計算には、初版の測定スクリプトkernel_probe.pyのPython実装を使う。
教師43本から1本あたり15万局面を抽出して並べ替え、先頭から16,384局面ずつ数える。
2026年11月28日の出力はhalfkp-vm-coverage.txtにある。
"""
import sys, time, glob, importlib.util
import numpy as np
sys.path.insert(0, "tools/train/zero")
import minase_mnsd_reader
from minase_zero.encoding import PIECE_CODE_PLANES

spec = importlib.util.spec_from_file_location("probe", "docs/measurements/adhoc/gpu-bottlenecks-20261128/kernel_probe.py")
probe = importlib.util.module_from_spec(spec); spec.loader.exec_module(probe)

files = sorted(glob.glob("data/nnue-generation-iteration/gen-1/teacher/book-*.mnsd"))
rng = np.random.default_rng(1)
table = np.array(PIECE_CODE_PLANES, dtype=np.int32)
seen = np.zeros(probe.SM_ROWS, dtype=bool)
total = 0
checkpoints = {16384 * k for k in (1, 10, 30, 60, 120, 240, 360)}
batches = []
for path in rng.permutation(files)[:43]:
    raw = minase_mnsd_reader.read(str(path))
    n = len(np.frombuffer(raw["sides"], dtype=np.uint8))
    idx = rng.choice(n, size=min(n, 150000), replace=False)
    boards = np.frombuffer(raw["boards"], dtype=np.uint8).reshape(n, 144)[idx] & 0x7F
    sides = np.frombuffer(raw["sides"], dtype=np.uint8)[idx]
    batches.append((boards, sides))
boards = np.concatenate([b for b, _ in batches]); sides = np.concatenate([s for _, s in batches])
perm = rng.permutation(len(sides)); boards, sides = boards[perm], sides[perm]
t = time.time()
for start in range(0, len(sides), 16384):
    _, hk = probe.build_features(boards[start:start + 16384], sides[start:start + 16384], table)
    v = hk.reshape(-1); v = v[v != probe.SENTINEL]
    seen[v] = True
    total = start + 16384
    if total in checkpoints:
        print(f"positions={total:>8} ever_observed={seen.sum():>7} fraction={seen.mean():.4f} ({time.time()-t:.0f}s)", flush=True)
