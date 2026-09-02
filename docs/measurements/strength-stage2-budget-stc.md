# strength-stage2-budget-stc

## 目的

棋力向上段階2の予算式（手数に基づく残り手数見積りと`soft ≤ hard`の不変条件）を、段階開始版と標準STCのGSPRTで比較し、LTCへ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage2-budget-stc --seed 20260903 \
  --candidate commit:96aa2ef --baseline commit:59846ec \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット96aa2ef、基準は段階開始版のコミット59846ec、規則セットは`engine-default`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

446ペアを実行し、有効ペア444、破棄ペア2（手数上限）であった。
ペンタノミアル度数は[49, 8, 238, 8, 141]、LLRは+2.948で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は3,048秒である。

## 結論

短時間GSPRTが`H1`かつ異常0件で終了したため、予算式を長時間GSPRTへ進める。
