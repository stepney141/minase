import pickle, numpy as np
rows = pickle.load(open("../../../../data/nnue-placement-outcome-regression/rows2.pkl", "rb"))
z = np.array([r["z"] for r in rows]); ply = np.array([r["ply"] for r in rows]); stm = np.array([r["stm"] for r in rows], float)
_, gid = np.unique([r["game"] for r in rows], return_inverse=True); w = 1.0 / np.bincount(gid)[gid]
_, pid = np.unique([r["pair"] for r in rows], return_inverse=True)
ltc = np.array([r["run"].endswith("ltc") for r in rows], float)
XF = np.stack([r["xf"] for r in rows]).astype(float); XM = np.stack([r["xm"] for r in rows]).astype(float); X5 = np.stack([r["x5"] for r in rows]).astype(float)
keepm = np.where(np.abs(XM).sum(0) > 0)[0]; XM = XM[:, keepm]
keepf = np.where(np.abs(XF).sum(0) > 0)[0]; XF = XF[:, keepf]
band = np.digitize(ply, [70, 90, 110, 130])
names5 = ["奔王1-2段", "龍馬1-2段", "醉象3-5筋", "金将3段以上", "麒麟右端2筋"]
print("両者の数が異なる局面の割合", {n: round(float((X5[:, i] != 0).mean()), 3) for i, n in enumerate(names5)})
print("局面", len(z), "局", gid.max() + 1, "ペア", pid.max() + 1, "引き分け", (z == 0.5).mean().round(4), "先手勝ち視点の平均", z[stm == 0].mean().round(4))
def fit(A, y, wt, P, it=7):
    th = np.zeros(A.shape[1])
    for _ in range(it):
        e = A @ th; p = 1 / (1 + np.exp(-e)); W = wt * np.clip(p * (1 - p), 1e-6, None)
        g = A.T @ (wt * (y - p)) - P * th
        H = (A * W[:, None]).T @ A + np.diag(P) + 1e-9 * np.eye(A.shape[1])
        d = np.linalg.solve(H, g); th += d
        if np.abs(d).max() < 1e-6: break
    return th
def nll(A, th, y):
    e = np.clip(A @ th, -30, 30); return np.logaddexp(0, -e) * y + np.logaddexp(0, e) * (1 - y)
rng = np.random.default_rng(1)
def boot(a, b):
    wp = np.bincount(pid, w); d = np.bincount(pid, w * (a - b)) / wp
    D = float((d * wp).sum() / wp.sum())
    bs = [float((d[i] * wp[i]).sum() / wp[i].sum()) for i in (rng.integers(0, len(d), len(d)) for _ in range(1000))]
    return f"{D:+.6f} [{np.percentile(bs, 2.5):+.6f}, {np.percentile(bs, 97.5):+.6f}]"
TAUS = (20, 40, 80)
for name in ("static", "search"):
    v = np.array([r[name] for r in rows], float) / 1000
    S = np.stack([v * (band == b) for b in range(5)], 1)
    C = np.c_[stm - 0.5, ltc - 0.5, (stm - 0.5) * (ltc - 0.5)]
    A0 = np.c_[C, S]; A1 = np.c_[A0, XM]; A2 = np.c_[A1, XF]
    n0, nm, nf = A0.shape[1], XM.shape[1], XF.shape[1]
    L0 = np.zeros(len(z)); L1 = np.zeros(len(z)); L2 = np.zeros(len(z)); chosen = []
    for fo in range(5):
        tr = pid % 5 != fo; te = ~tr
        th0 = fit(A0[tr], z[tr], w[tr], np.zeros(n0))
        K = 1000 / th0[3:].mean()
        pm = np.r_[np.zeros(n0), np.full(nm, (K / 160) ** 2)]
        th1 = fit(A1[tr], z[tr], w[tr], pm)
        L0[te] = nll(A0[te], th0, z[te]); L1[te] = nll(A1[te], th1, z[te])
        inner = (pid % 25 // 5 == (fo + 1) % 5) & tr  # 外側の訓練の中の選択用
        fit_i = tr & ~inner
        best = None
        for tp in TAUS:
            P = np.r_[pm, np.full(nf, (K / tp) ** 2)]
            th = fit(A2[fit_i], z[fit_i], w[fit_i], P, it=5)
            l = float((w[inner] * nll(A2[inner], th, z[inner])).sum() / w[inner].sum())
            if best is None or l < best[0]: best = (l, tp)
        chosen.append(best[1])
        P = np.r_[pm, np.full(nf, (K / best[1]) ** 2)]
        th2 = fit(A2[tr], z[tr], w[tr], P)
        L2[te] = nll(A2[te], th2, z[te])
    print(f"--- {name}: 基準損失 {float((w * L0).sum() / w.sum()):.5f}  Kの目安 {K:.0f}  選ばれたτ {chosen}")
    print("  枚数差（成駒を含む）の効果", boot(L1, L0))
    print("  配置の追加の効果（入れ子で選んだτ）", boot(L2, L1), flush=True)
    A5 = np.c_[A1, X5]; P5 = np.r_[np.zeros(n0), np.full(nm, (K / 160) ** 2), np.zeros(5)]
    th = fit(A5, z, w, P5, it=9); sl = th[3:8].mean()
    idx = [np.where(pid == i)[0] for i in range(pid.max() + 1)]; B = []
    for b in range(80):
        I = np.concatenate([idx[i] for i in rng.integers(0, len(idx), len(idx))])
        t = fit(A5[I], z[I], w[I], P5, it=6); B.append(t[-5:] / t[3:8].mean() * 1000)
    B = np.array(B)
    for i, n in enumerate(["奔王1-2段", "龍馬1-2段", "醉象3-5筋", "金将3段以上", "麒麟右端2筋"]):
        print(f"  {n}: {th[-5 + i] / sl * 1000:+.1f} cp [{np.percentile(B[:, i], 2.5):+.1f}, {np.percentile(B[:, i], 97.5):+.1f}]", flush=True)
