# 教師データの近傍の駒の数と2駒の組の出現

## 目的

[相対位置の局所2駒関係による評価の補正](../plans/relative-pair-eval.md)の半径、費用の見積もり、および出現閾値の判断材料として、基本の教師データの標本で、駒ごとの近傍の駒の数と、近傍の2駒の組が参照する表の要素の出現局面数を数える。
棋力測定ではなく、エンジンも対局も使わない。

## コマンドライン

2026年9月23日に、次のスクリプトを `relative_pair_neighbor_stats.py` として保存し、リポジトリの根を引数に渡して実行した（スクリプトのSHA-256は `d22d8970f2718026ba22d9d1bc247c5a831dcc5bf9482fe61bb11a3860ce77f8`）。

```console
tools/train/.venv/bin/python relative_pair_neighbor_stats.py /home/stepney141/board-games/minase
```

```python
"""教師データの標本で、近傍の駒の数と近傍の2駒の組の出現を数える。"""
import sys
from pathlib import Path

import numpy as np

REPO = Path(sys.argv[1])
sys.path.insert(0, str(REPO / "tools/train/pst"))
from features import COLOR_BY_BYTE, INITIAL_BOARD, PIECE_STATE_BY_BYTE  # noqa: E402
from mnsd import HEADER_LENGTH, RECORD_DTYPE, read_header  # noqa: E402

DATA = REPO / "data"
FILES = [DATA / "gen0.bin"]
FILES += [DATA / f"gen1-s{i}00000.bin" for i in range(1, 6)]
FILES += [DATA / f"strength-stage7/gen2/generated-{i}00000.bin" for i in range(6, 11)]
SAMPLE, SEED, PAD = 200_000, 1, 3

rng = np.random.default_rng(SEED)
counts = [read_header(f).record_count for f in FILES]
total = sum(counts)
boards, stms = [], []
for path, count in zip(FILES, counts):
    records = np.memmap(path, mode="r", dtype=RECORD_DTYPE, offset=HEADER_LENGTH, shape=(count,))
    picked = records[np.sort(rng.choice(count, size=round(SAMPLE * count / total), replace=False))]
    boards.append(np.array(picked["board"]))
    stms.append(np.array(picked["stm"]))
board = np.concatenate(boards)
stm = np.concatenate(stms)
n = len(board)
occupied = (board != 0).reshape(n, 12, 12)
color = COLOR_BY_BYTE[board].reshape(n, 12, 12)
home = ((board == INITIAL_BOARD[None, :]) & (board != 0)).reshape(n, 12, 12)
state = PIECE_STATE_BY_BYTE[board].reshape(n, 12, 12)
code = np.where(occupied, np.where(color == stm[:, None, None], 0, 1) * 47 + state, -1)
print(f"total records {total}, sample {n}, pieces per position {occupied.sum((1, 2)).mean():.1f}")


def shifted(array, fill, dr, df):
    padded = np.full((n, 12 + 2 * PAD, 12 + 2 * PAD), fill, array.dtype)
    padded[:, PAD:-PAD, PAD:-PAD] = array
    return padded[:, PAD + dr : PAD + dr + 12, PAD + df : PAD + df + 12]


for radius in (1, 2, 3):
    same = np.zeros((n, 12, 12), np.int32)
    opposite = np.zeros((n, 12, 12), np.int32)
    same_home_pairs = 0
    keys = []
    for dr in range(-radius, radius + 1):
        for df in range(-radius, radius + 1):
            if (dr, df) == (0, 0):
                continue
            other = shifted(occupied, False, dr, df) & occupied
            other_color = shifted(color, -1, dr, df)
            same += other & (other_color == color)
            opposite += other & (other_color != color)
            same_home_pairs += (other & (other_color == color) & home & shifted(home, False, dr, df)).sum()
            a = code[other].astype(np.int64)
            b = shifted(code, -1, dr, df)[other].astype(np.int64)
            position = np.nonzero(other)[0]
            # 後手番の局面では、特徴と同じく段の向きを手番側から見た向きへ反転する。
            rank = np.where(stm[position] == 1, -dr, dr)
            orbit = np.stack([
                ((a * 94 + b) * 7 + rank + 3) * 7 + df + 3,
                ((b * 94 + a) * 7 - rank + 3) * 7 - df + 3,
                ((a * 94 + b) * 7 + rank + 3) * 7 - df + 3,
                ((b * 94 + a) * 7 - rank + 3) * 7 + df + 3,
            ])
            keys.append(np.stack([orbit.min(0), position, (a // 47 != b // 47)]))
    neighbors = (same + opposite)[occupied]
    pairs = (same + opposite).sum((1, 2)) / 2
    print(
        f"R<={radius}: neighbors per piece mean {neighbors.mean():.2f} (same {same[occupied].mean():.2f}, "
        f"opposite {opposite[occupied].mean():.2f}) p90 {np.percentile(neighbors, 90):.0f} max {neighbors.max()}; "
        f"pairs per position mean {pairs.mean():.0f}; opposite share {opposite.sum() / (same + opposite).sum():.3f}; "
        f"same-colour pairs with both on initial squares {same_home_pairs / same.sum():.3f}"
    )
    key, pos, opp = np.concatenate(keys, axis=1)
    unique = np.unique(np.stack([key, pos]), axis=1)
    elements, per_element = np.unique(unique[0], return_counts=True)
    opp_elements = np.unique(key[opp == 1])
    opp_counts = per_element[np.isin(elements, opp_elements)]
    print(
        f"        canonical elements observed {len(elements)}; positions per element median {np.median(per_element):.0f}; "
        f"opposite-colour elements {len(opp_elements)}, positions per element median {np.median(opp_counts):.0f}; "
        f"elements with >= {200 * n / total:.2f} sample positions (= 200 in full data) {np.sum(per_element >= 200 * n / total)}"
    )
```

## エンジン

エンジンは使わない。
入力は、現行の `nets/pst.bin` の学習に使った基本の教師データ11ファイル（`data/gen0.bin`、`data/gen1-s{1..5}00000.bin`、`data/strength-stage7/gen2/generated-{6..10}00000.bin`、計25,120,683局面）である。
駒状態と手番側視点の陣営は、master（7a5a0e3）の `tools/train/pst/features.py` の定義による。
各ファイルから局面数に比例した数を乱数シード1で非復元抽出し、計20万局面を標本とした。

## 環境

Intel Core Ultra 7 265KF、Python 3.12.14、NumPy 2.5.2で、CPUの1スレッドで実行した。
所要時間は約4分である。

## 結果

標本の1局面あたりの駒数は平均55.1枚だった。
近傍は、駒を中心とする \((2R+1)\times(2R+1)\) の正方形から中心を除いた升（筋の差と段の差の大きい方が \(R\) 以下）である。

| 半径 | 1駒あたりの近傍の駒の数（平均） | 同じ陣営／異なる陣営 | 90パーセンタイル | 最大 | 1局面あたりの組の数（平均） | 組のうち異なる陣営の割合 | 同じ陣営の組のうち2駒とも初期升にある割合 |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 1 | 3.37 | 3.33／0.04 | 6 | 8 | 93 | 1.3% | 32.3% |
| 2以下 | 8.62 | 8.33／0.29 | 14 | 22 | 238 | 3.4% | 30.7% |
| 3以下 | 14.89 | 13.92／0.97 | 23 | 33 | 410 | 6.5% | 31.6% |

表の要素は、後手番の局面で段の向きを反転した手番側視点の相対変位について、組の順序の入れ替えと左右の鏡映で移り合う3つ組を1つにまとめた番号であり、出現局面数は同じ局面で複数回現れても1回と数えた。
全データの200局面は、標本では約1.59局面に当たる。

| 半径 | 表の要素の数 | 標本で観測した要素 | 出現局面数の中央値（全要素） | 異なる陣営の組の要素（観測数、出現局面数の中央値） | 全データ換算で200局面以上の要素 |
|---:|---:|---:|---:|---:|---:|
| 1 | 22,137 | 9,392 | 13 | 3,856、5 | 8,029 |
| 2以下 | 61,946 | 28,285 | 16 | 12,968、9 | 24,541 |
| 3以下 | 119,427 | 55,119 | 17 | 26,014、10 | 47,884 |

標本の出現局面数を全データへ換算する倍率は約125.6であり、半径2以下の異なる陣営の組の要素の中央値9局面は、全データで約1,100局面に当たる。

## 結論

近傍の駒の大部分は同じ陣営の駒であり、異なる陣営の組は半径2以下で組全体の3.4%にとどまる。
同じ陣営の組の約3割は2駒とも初期升にあるので、関係項の学習では対局結果の記憶を避ける教師の選択が必要である。
観測された要素の大部分は全データで200局面以上に現れ、出現閾値200で0に固定される要素は観測された要素の1割強である。
