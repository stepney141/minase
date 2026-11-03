import json
d = json.load(open("../../../../data/piece-placement-census/counts.json")); C = d["counts"]
NAME = dict(l="香車", f="猛豹", c="銅将", s="銀将", g="金将", e="醉象", k="王将", a="反車", b="角行", t="盲虎", x="鳳凰", o="麒麟", m="横行", v="竪行", r="飛車", h="龍馬", d="龍王", q="奔王", n="獅子", p="歩兵", i="仲人")
def dist(g, ph, k):
    a = C.get(g, {}).get(str(ph), {}).get(k)
    if not a: return None, 0
    t = sum(a); return [x / t for x in a], t
def tv(g1, g2, ph, k):
    p, n = dist(g1, ph, k); q, m = dist(g2, ph, k)
    if p is None or q is None: return None
    return 0.5 * sum(abs(x - y) for x, y in zip(p, q))
REF = "human2000"
cols = ["nnue_now", "pst_fm", "pst_gen1", "pst_gen0", "hachu", "c0032", "human1700", "humanlow"]
for ph, lbl in enumerate(["20〜59手", "60〜99手", "100〜159手"]):
    print(f"== {lbl}  人間2000以上との総変動距離TV  (noise=人間2000の折半, nnue_noise=現行の折半)")
    print("駒    " + " ".join(f"{c[:9]:>9}" for c in cols) + "     noise nnue_noise")
    tot = {c: [] for c in cols}
    for k in NAME:
        row = []
        for c in cols:
            v = tv(c, REF, ph, k); row.append(v)
            if v is not None: tot[c].append(v)
        nz = tv(REF + "#A", REF + "#B", ph, k); nn = tv("nnue_now#A", "nnue_now#B", ph, k)
        print(f"{NAME[k]:4} " + " ".join(f"{(v if v is not None else float('nan')):9.3f}" for v in row) + f" {nz:9.3f} {nn:9.3f}")
    print("平均 " + " ".join(f"{sum(tot[c]) / max(len(tot[c]), 1):9.3f}" for c in cols))
print()
# 現行と人間の差が大きい升（100手未満の2局面帯を合算）
def merged(g, k):
    out = [0] * 144
    for ph in (0, 1):
        a = C.get(g, {}).get(str(ph), {}).get(k)
        if a: out = [x + y for x, y in zip(out, a)]
    t = sum(out) or 1
    return [x / t for x in out]
print("== 20〜99手、現行ネットが人間2000以上より多くいる升（筋は持ち主の右端=1、段は自陣最下段=1）")
for k in NAME:
    p = merged("nnue_now", k); q = merged(REF, k); z = merged("c0032", k); h = merged("hachu", k); g0 = merged("pst_gen0", k)
    idx = sorted(range(144), key=lambda i: q[i] - p[i])[:3]
    s = "  ".join(f"{i // 12 + 1}筋{i % 12 + 1}段 {p[i]:.1%}対{q[i]:.1%}(c0032 {z[i]:.1%},HaChu {h[i]:.1%},gen0 {g0[i]:.1%})" for i in idx)
    print(f"{NAME[k]:4} TV {0.5 * sum(abs(a - b) for a, b in zip(p, q)):.3f}  {s}")
print()
print("== 平均段（自陣最下段=1）と平均の|筋−6.5|（小さいほど中央）20〜99手")
hdr = ["nnue_now", "pst_gen0", "hachu", "c0032", "human2000"]
print("駒    " + " ".join(f"{c[:9]:>14}" for c in hdr))
def ms(g, k):
    p = merged(g, k)
    return sum(p[i] * (i % 12 + 1) for i in range(144)), sum(p[i] * abs(i // 12 + 1 - 6.5) for i in range(144))
for k in NAME:
    print(f"{NAME[k]:4} " + " ".join("{:6.2f}/{:6.2f} ".format(*ms(c, k)) for c in hdr))
