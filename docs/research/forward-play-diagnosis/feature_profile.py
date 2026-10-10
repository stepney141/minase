"""教師データでの特徴の出現回数、PSTの初期値からの変化、FM補正の周辺寄与を段ごとに集計する。

使い方（リポジトリのルートで実行する）:
  uv run --project tools/train python docs/research/forward-play-diagnosis/feature_profile.py \
      data/pst-longer-training.toml <集計結果.npz>
集計結果のファイルが既にあれば、集計を省いて表だけを出力する。
"""
import sys, tomllib
from pathlib import Path
import numpy as np
from minase_train.data.features import FEATURE_COUNT, feature_indices
from minase_train.data.mnpt import initial_weights, read_mnpt, read_mnpt_v3
from minase_train.data.mnsd import map_records

config, output = sys.argv[1], Path(sys.argv[2])
CHUNK, FM_STRIDE = 20_000, 4  # FMの周辺寄与は4塊に1塊だけで推定する。


def collect():
    i0, i1, _, _ = read_mnpt("crates/minase/nets/pst-init.bin")
    assert np.array_equal(i0, initial_weights()) and np.array_equal(i1, initial_weights())
    c0, c1, _, _, u, signs, exponent = read_mnpt_v3("crates/minase/nets/pst.bin")
    d0 = np.append(c0.astype(np.float64) - i0, 0.0); d1 = np.append(c1.astype(np.float64) - i1, 0.0)
    up = np.vstack([u.astype(np.float32), np.zeros((1, u.shape[1]), np.float32)])
    sg = signs.astype(np.float32); fm_divisor = float(2 ** (2 * exponent))
    cnt = np.zeros(FEATURE_COUNT + 1, np.int64); dsum = np.zeros(FEATURE_COUNT + 1)
    fsum = np.zeros(FEATURE_COUNT + 1); fcnt = np.zeros(FEATURE_COUNT + 1, np.int64); total = 0
    for path in tomllib.load(open(config, "rb"))["run"]["data"]:
        records = map_records(path); total += len(records)
        for chunk, start in enumerate(range(0, len(records), CHUNK)):
            r = records[start:start + CHUNK]
            idx = feature_indices(r["board"], r["stm"], r["lion"]); board = idx[:, :144]; flat = board.ravel()
            q = np.clip((r["board"] != 0).sum(1) - 2, 0, 90).astype(np.float64)[:, None]
            change = (q * d0[board] + (90 - q) * d1[board]) / 720.0  # 探索と同じ序中盤・終盤の補間（cp）
            cnt += np.bincount(flat, minlength=FEATURE_COUNT + 1)
            dsum += np.bincount(flat, weights=change.ravel(), minlength=FEATURE_COUNT + 1)
            if chunk % FM_STRIDE == 0:
                g = up[idx]; s = g.sum(1, keepdims=True)
                marginal = ((g * (s - g)) * sg).sum(2) / fm_divisor  # 特徴iと他の全特徴の組の寄与（cp）
                fsum += np.bincount(idx.ravel(), weights=marginal.ravel().astype(np.float64), minlength=FEATURE_COUNT + 1)
                fcnt += np.bincount(idx.ravel(), minlength=FEATURE_COUNT + 1)
        print(Path(path).name, len(records), file=sys.stderr, flush=True)
    np.savez(output, cnt=cnt[:FEATURE_COUNT], dsum=dsum[:FEATURE_COUNT], fsum=fsum[:FEATURE_COUNT],
             fcnt=fcnt[:FEATURE_COUNT], total=total, i0=i0, i1=i1, c0=c0, c1=c1)


if not output.exists():
    collect()
z = np.load(output); cnt, dsum, fsum, fcnt = z["cnt"], z["dsum"], z["fsum"], z["fcnt"]; total = int(z["total"])
GROUPS = [("歩兵", [29]), ("銅将・銀将・金将", [42, 43, 44]), ("猛豹・盲虎・醉象", [39, 40, 41]),
          ("横行・竪行・角行・飛車・反車・香車", [31, 32, 33, 34, 35, 36]), ("龍馬・龍王（成る前）", [37, 38]),
          ("と金", [17]), ("獅子・奔王", [10, 20]), ("成駒専用の駒種", list(range(22, 29)))]
print("局面数", total)
for title, states in GROUPS:
    print(f"\n== {title}（自駒）  段  出現/局面  PST変化cp  FM周辺寄与cp")
    for depth in range(12):  # 手番側視点の段番号は、自陣最下段が0。
        ids = [s * 144 + depth * 12 + f for s in states for f in range(12)]
        n = cnt[ids].sum()
        if n:
            fm = fsum[ids].sum() / fcnt[ids].sum() if fcnt[ids].sum() else float("nan")
            print(f"  {depth:2d} {n / total:9.5f} {dsum[ids].sum() / n:8.1f} {fm:8.1f}  n={n}")
own = [s * 144 + q for s in range(47) if s not in (11, 21) for q in range(144)]
own = np.array(own); count = cnt[own]
moved = np.abs((z["c0"][own].astype(float) - z["i0"][own]) + (z["c1"][own].astype(float) - z["i1"][own])) / 16
rare = (count >= 1) & (count < 100)
print(f"\n出現1〜99回の自駒特徴 {rare.sum()}個（うち不成で8〜11段 {((own[rare] // 144 >= 29) & ((own[rare] % 144) // 12 >= 8)).sum()}個）、"
      f"変化量の中央値 {np.median(moved[rare]):.1f}cp")
