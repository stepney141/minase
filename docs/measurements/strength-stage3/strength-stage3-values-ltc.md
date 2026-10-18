# strength-stage3-values-ltc

## 目的

棋力向上段階3の駒価値の再調整（学習PSTから導出した駒価値による指し手順序付けと静止探索の余裕値）を、段階開始版と標準LTCのGSPRTで比較し、採否を確定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage3-values-ltc --seed 20290903 \
  --candidate commit:6b4c460 --baseline commit:3a6b038 \
  --each time=60000+200 gsprt
```

## エンジン

候補はコミット6b4c460、基準は段階開始版のコミット3a6b038（探索部のコードは段階2の完了コミット2d1831eと同一）、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。
測定中に、静止探索の深さ分布の診断（`nice -n 19`のbench1回）を並行して実行した。

## 結果

285ペアを実行し、有効ペア280、破棄ペア5（手数上限）であった。
ペンタノミアル度数は[33, 5, 158, 5, 79]、LLRは+2.984で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は7,282秒である。

## 結論

長時間GSPRTは`H1`かつ異常0件であり、駒価値の再調整を採用する。
