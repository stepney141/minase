"""埋め込みネットnnue.binの第1層と出力重みの値域を集計する（読み取りのみ）。"""
import struct
import sys

import numpy as np

path = sys.argv[1]
raw = open(path, "rb").read()
assert raw[:4] == b"MNUE"
version = struct.unpack_from("<I", raw, 4)[0]
arch = raw[8:104].rstrip(b"\0").decode()
width = struct.unpack_from("<I", raw, 104)[0]
k = struct.unpack_from("<f", raw, 108)[0]
print(f"version={version} arch={arch} H={width} K={k:.2f} bytes={len(raw)}")

HEADER = 192
FEATURES = 13_680
body = HEADER
linear = np.frombuffer(raw, "<i2", FEATURES * 2, body)
body += FEATURES * 4 + 47 * 4
ft = np.frombuffer(raw, "<i2", FEATURES * width, body).reshape(FEATURES, width)
body += FEATURES * 2 * width
bias = np.frombuffer(raw, "<i4", width, body)
body += 4 * width
out = np.frombuffer(raw, "<i2", 2 * width, body)
out_bias = struct.unpack_from("<i", raw, body + 4 * width)[0]
assert body + 4 * width + 4 == len(raw)

abs_ft = np.abs(ft.astype(np.int32))
used = abs_ft.any(axis=1)
print(f"features with any nonzero row: {used.sum()} / {FEATURES}")
print(f"FT |w| max={abs_ft.max()} (float {abs_ft.max() / 127:.3f}, clip bound 1016 = 8.0)")
for q in (50, 90, 99, 99.9, 99.99):
    print(f"  FT |w| p{q}: {np.percentile(abs_ft[used], q):.1f}")
print(f"bias range: {bias.min()} .. {bias.max()}")

# 視点ごとの累算値が理論上i16へ収まるかの上界（発火146個が全部同符号の最悪）。
row_max = abs_ft.max(axis=1)
top146 = np.sort(row_max)[-146:].sum()
print(f"worst-case sum over any 146 rows of per-feature max |w|: {top146} (i16 limit 32767)")
print(f"clip needed for i16 with 146 active: |w| <= {32767 // 146} (float {32767 / 146 / 127:.3f})")

abs_out = np.abs(out.astype(np.int32))
print(f"output |w| max={abs_out.max()} (bound 8192), zero={np.mean(out == 0):.3%}")
print(f"output weights sign: pos={np.mean(out > 0):.3f} neg={np.mean(out < 0):.3f}")

# 列の冗長性：鏡映共有の下で独立な行は前半H/2本。列どうしの相関の高さを見る。
half = ft[:, : width // 2].astype(np.float64)
half = half[used]
half -= half.mean(axis=0, keepdims=True)
s = np.linalg.svd(half, compute_uv=False)
energy = np.cumsum(s**2) / np.sum(s**2)
print("spectrum of independent FT columns (H/2 =", width // 2, "):")
for r in (1, 2, 4, 8, 16, 32, 48, 56, 60, 64):
    if r <= len(energy):
        print(f"  top {r:3d} singular values: {energy[r - 1]:.4f} of energy")
print(f"  s_max/s_min = {s[0] / s[-1]:.2f}")
norms = np.linalg.norm(ft.astype(np.float64), axis=0)
dead = np.sum(norms[: width // 2] < 1e-9)
print(f"column norm min/median/max: {norms.min():.0f}/{np.median(norms):.0f}/{norms.max():.0f}, dead={dead}")
contrib = np.abs(out[: width // 2].astype(np.float64))
print(f"us-side |output| sorted top/median/bottom: {np.sort(contrib)[-1]:.0f}/{np.median(contrib):.0f}/{np.sort(contrib)[0]:.0f}")
