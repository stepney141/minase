# strength-stage5-malus-stc

## 目的

棋力向上段階5のmalus（β打ち切りを起こした静かな手に加点し、同じノードでそれより前に探索した静かな手に減点し、更新式`v += bonus − v · |bonus| / HISTORY_LIMIT`で表を有界に保つ変更）を、直前の採用構成と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage5-malus-stc --seed 20490903 \
  --candidate commit:c002357 --baseline commit:7877531 \
  --each time=10000+100 --concurrency 14 gsprt --max-pairs 3000
```

## エンジン

候補はコミットc002357（分岐stage5-malus-p1）、基準は順序付けキーの重複計算の除去を採用したコミット7877531、規則セットは`L0,P0,R1,E0`である。
候補はbutterfly表だけを持つ構成へmalusを適用したものであり、piece-to historyのSTCが判定前だったため、その不採用を想定して先行した。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した14である。
同じ機械でpiece-to historyのSTC（同時対局数4）が並行していた。

## 結果

3,000ペアを実行し、有効ペア2,955、破棄ペア45（手数上限）であった。
ペンタノミアル度数は[625, 55, 1573, 62, 640]、LLRは−1.578で`decision: pending`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は28,785秒である。

## 結論

上限3,000ペアでの判定保留であり、停止時点のLLRが負なのでLTCへ進めず、malusは不採用とする。
benchの総ノード数は17.9%減っていたが、時間制御の対局では強さの差として検出できなかった。
