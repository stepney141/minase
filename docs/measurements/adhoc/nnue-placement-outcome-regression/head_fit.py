"""第1層を固定して出力重みだけを対局結果へ回帰する案と、段と筋の補正を、同じ折りの交差検証で比べる。"""
import pickle, struct, subprocess, tempfile
import numpy as np
NET = "../../../../crates/minase/nets/nnue.bin"
BIN = "../../../../target/match-cache/6d47da2a9663918f8992290834d9c47e695de98f/minase"
rows = pickle.load(open("../../../../data/nnue-placement-outcome-regression/rows2.pkl", "rb"))
b = open(NET, "rb").read()
H = struct.unpack_from("<I", b, 104)[0]; cpl = struct.unpack_from("<f", b, 108)[0]
scale = round(cpl * 65536 / (127 * 4096))
o = 192 + 13680 * 4 + 47 * 4
W = np.frombuffer(b, "<i2", 13680 * H, o).reshape(13680, H).astype(np.int32); o += 13680 * H * 2
BI = np.frombuffer(b, "<i4", H, o).astype(np.int32); o += H * 4
OW = np.frombuffer(b, "<i2", 2 * H, o).astype(np.int64); o += 4 * H
OB = struct.unpack_from("<i", b, o)[0]
KIND = dict(p=0, i=1, l=2, a=3, m=4, v=5, b=6, r=7, h=8, d=9, q=10, k=11, e=12, f=13, t=14, c=15, s=16, g=17, o=18, x=19, n=20)
PROM = {0: 17, 1: 12, 2: 22, 3: 23, 4: 25, 5: 24, 6: 8, 7: 9, 8: 27, 9: 28, 12: 21, 13: 6, 14: 26, 15: 4, 16: 5, 17: 7, 18: 20, 19: 10}
NOPROM = {10, 11, 20}
ORDER = [k for k in range(29) if k in PROM]
UNPROM = {k: 29 + i for i, k in enumerate(ORDER)}
def state(k, prom):
    if prom: return PROM[k]
    return UNPROM.get(k, k)
def parse(sfen):
    board, side, *_ = sfen.split()
    out = []
    for r, row in enumerate(board.split("/")):
        f = 11; i = 0
        while i < len(row):
            ch = row[i]
            if ch.isdigit():
                j = i
                while j < len(row) and row[j].isdigit(): j += 1
                f -= int(row[i:j]); i = j; continue
            prom = ch == "+"
            if prom: i += 1; ch = row[i]
            out.append((f, r, KIND[ch.lower()], prom, 0 if ch.isupper() else 1))
            f -= 1; i += 1
    return out, 0 if side == "b" else 1
def activations(sfen):
    pcs, stm = parse(sfen)
    acc = [BI.copy(), BI.copy()]
    for f, r, k, prom, col in pcs:
        st = state(k, prom)
        for persp in (0, 1):
            rel = 0 if col == persp else 1
            rr = 11 - r if persp == 0 else r  # 盤面の文字列の先頭の段は内部の段番号11に当たる
            acc[persp] += W[(rel * 47 + st) * 144 + rr * 12 + f]
    c0 = np.clip(acc[stm], 0, 127); c1 = np.clip(acc[1 - stm], 0, 127)
    return (c0 * c0) >> 7, (c1 * c1) >> 7
AU = np.zeros((len(rows), H), np.int32); AT = np.zeros((len(rows), H), np.int32)
for i, r in enumerate(rows):
    AU[i], AT[i] = activations(r["sfen"])
# 推論の再現を、エンジンの非線形部分と照合する
out = (AU.astype(np.int64) * OW[:H]).sum(1) + (AT.astype(np.int64) * OW[H:]).sum(1) + OB
cp = (out * scale + 32768) >> 16
with tempfile.TemporaryFile("w+", encoding="utf-8") as c:
    c.write("usi\nisready\nusinewgame\n")
    for r in rows[:4000]: c.write(f"position sfen {r['sfen']}\neval\n")
    c.write("quit\n"); c.seek(0)
    txt = subprocess.run([BIN, "--protocol", "usi", "--rules", "engine-default"], stdin=c, stdout=subprocess.PIPE, text=True, check=True).stdout
nl = np.array([int(l.split()[-1]) for l in txt.splitlines() if l.startswith("info string nonlinear ")])
print("非線形部分の一致", int((nl == cp[:4000]).sum()), "/", len(nl), "最大差", int(np.abs(nl - cp[:4000]).max()), flush=True)
print("共有の確認 w[j]==w[j+H/2]", bool((OW[:H // 2] == OW[H // 2:H]).all()), bool((OW[H:H + H // 2] == OW[H + H // 2:]).all()))
XH = np.c_[AU[:, :H // 2] + AU[:, H // 2:], AT[:, :H // 2] + AT[:, H // 2:]].astype(float)
XH = (XH - XH.mean(0)) / np.maximum(XH.std(0), 1e-9)
z = np.array([r["z"] for r in rows]); ply = np.array([r["ply"] for r in rows]); stm = np.array([r["stm"] for r in rows], float)
_, gid = np.unique([r["game"] for r in rows], return_inverse=True); w = 1.0 / np.bincount(gid)[gid]
_, pid = np.unique([r["pair"] for r in rows], return_inverse=True)
ltc = np.array([r["run"].endswith("ltc") for r in rows], float)
XF = np.stack([r["xf"] for r in rows]).astype(float); XF = XF[:, np.where(np.abs(XF).sum(0) > 0)[0]]
band = np.digitize(ply, [70, 90, 110, 130])
v = np.array([r["static"] for r in rows], float) / 1000
A0 = np.c_[stm - 0.5, ltc - 0.5, (stm - 0.5) * (ltc - 0.5), np.stack([v * (band == t) for t in range(5)], 1)]
n0 = A0.shape[1]
def fit(A, y, wt, P, it=7):
    th = np.zeros(A.shape[1])
    for _ in range(it):
        e = A @ th; p = 1 / (1 + np.exp(-e)); W_ = wt * np.clip(p * (1 - p), 1e-6, None)
        g = A.T @ (wt * (y - p)) - P * th
        H_ = (A * W_[:, None]).T @ A + np.diag(P) + 1e-9 * np.eye(A.shape[1])
        d = np.linalg.solve(H_, g); th += d
        if np.abs(d).max() < 1e-6: break
    return th
def nll(A, th, y):
    e = np.clip(A @ th, -30, 30); return np.logaddexp(0, -e) * y + np.logaddexp(0, e) * (1 - y)
rng = np.random.default_rng(1); wp = np.bincount(pid, w)
def boot(a, b_):
    d = np.bincount(pid, w * (a - b_)) / wp
    D = float((d * wp).sum() / wp.sum())
    bs = [float((d[i] * wp[i]).sum() / wp[i].sum()) for i in (rng.integers(0, len(d), len(d)) for _ in range(1000))]
    return f"{D:+.6f} [{np.percentile(bs, 2.5):+.6f}, {np.percentile(bs, 97.5):+.6f}]"
def cv(blocks, grids):
    """blocksは特徴の行列の列、gridsは各ブロックのτの候補。"""
    import itertools
    A = np.c_[(A0, *blocks)]; L = np.zeros(len(z)); chosen = []
    for fo in range(5):
        tr = pid % 5 != fo; te = ~tr
        th0 = fit(A0[tr], z[tr], w[tr], np.zeros(n0)); K = 1000 / th0[3:].mean()
        inner = (pid % 25 // 5 == (fo + 1) % 5) & tr; fi = tr & ~inner; best = None
        for taus in itertools.product(*grids):
            P = np.r_[(np.zeros(n0), *[np.full(x.shape[1], (K / t) ** 2) for x, t in zip(blocks, taus)])]
            th = fit(A[fi], z[fi], w[fi], P, it=5)
            l = float((w[inner] * nll(A[inner], th, z[inner])).sum() / w[inner].sum())
            if best is None or l < best[0]: best = (l, taus, P)
        chosen.append(best[1])
        L[te] = nll(A[te], fit(A[tr], z[tr], w[tr], best[2]), z[te])
    return L, chosen
L0 = np.zeros(len(z))
for fo in range(5):
    tr = pid % 5 != fo; te = ~tr
    L0[te] = nll(A0[te], fit(A0[tr], z[tr], w[tr], np.zeros(n0)), z[te])
print("基準（静的評価）の損失", round(float((w * L0).sum() / w.sum()), 5), flush=True)
LP, cp_ = cv([XF], [(20, 40, 80)]); print("段と筋の補正", boot(LP, L0), cp_, flush=True)
LH, ch = cv([XH], [(5, 10, 20, 40)]); print("出力重みの補正", boot(LH, L0), ch, flush=True)
print("出力重み−段と筋", boot(LH, LP), flush=True)
LB, cb = cv([XF, XH], [(20, 40, 80), (5, 10, 20, 40)]); print("両方", boot(LB, L0), cb, flush=True)
print("両方−段と筋", boot(LB, LP), flush=True)
