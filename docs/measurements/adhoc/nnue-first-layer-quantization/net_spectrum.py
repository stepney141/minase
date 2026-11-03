"""第1層の独立な列の特異値を、初期値（標準偏差0.05、量子化尺度127）の雑音と比べる。"""
import struct
import sys

import numpy as np

raw = open(sys.argv[1], "rb").read()
width = struct.unpack_from("<I", raw, 104)[0]
FEATURES = 13_680
start = 192 + FEATURES * 4 + 47 * 4
ft = np.frombuffer(raw, "<i2", FEATURES * width, start).reshape(FEATURES, width)
used = np.abs(ft).any(axis=1)
half = ft[used][:, : width // 2].astype(np.float64)
m, n = half.shape
sigma = 0.05 * 127
edge = sigma * (np.sqrt(m) + np.sqrt(n))
low = sigma * (np.sqrt(m) - np.sqrt(n))
s = np.linalg.svd(half, compute_uv=False)
print(f"rows={m} cols={n} init noise sigma={sigma:.2f}")
print(f"Marchenko-Pastur bulk for init noise alone: [{low:.0f}, {edge:.0f}]")
print("singular values:", " ".join(f"{v:.0f}" for v in s))
for factor in (1.0, 1.25, 1.5, 2.0, 3.0):
    print(f"  count above {factor:.2f} x bulk edge: {(s > factor * edge).sum()} / {n}")

rng = np.random.default_rng(1)
noise = np.rint(rng.normal(0, sigma, size=(m, n)))
sn = np.linalg.svd(noise, compute_uv=False)
print(f"simulated pure init noise: max={sn[0]:.0f} min={sn[-1]:.0f}")

# 実際の重みのうち、初期値の雑音で説明できる大きさの割合。
abs_w = np.abs(half)
print(f"median |w| = {np.median(abs_w):.1f}; pure-noise median would be {sigma * 0.6745:.1f}")
print(f"share of weights with |w| <= 2 sigma ({2 * sigma:.1f}): {np.mean(abs_w <= 2 * sigma):.3f} (pure noise: 0.954)")
row_rms = np.sqrt(np.mean(half**2, axis=1))
for q in (10, 25, 50, 75, 90, 99):
    print(f"  row RMS p{q}: {np.percentile(row_rms, q):.1f}")
