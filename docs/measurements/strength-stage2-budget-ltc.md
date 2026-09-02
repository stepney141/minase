# strength-stage2-budget-ltc

## 目的

棋力向上段階2の予算式（手数に基づく残り手数見積りと`soft ≤ hard`の不変条件）を、段階開始版と標準LTCのGSPRTで比較し、採否を確定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage2-budget-ltc --seed 20270903 \
  --candidate commit:96aa2ef --baseline commit:59846ec \
  --each time=60000+600 gsprt
```

## エンジン

候補はコミット96aa2ef、基準は段階開始版のコミット59846ec、規則セットは`engine-default`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

526ペアを実行し、有効ペア519、破棄ペア7（手数上限）であった。
ペンタノミアル度数は[64, 10, 275, 18, 152]、LLRは+2.963で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は23,080秒である。

## 結論

長時間GSPRTが`H1`かつ異常0件で終了したため、予算式を採用し、反復継続の判断の基準にする。
