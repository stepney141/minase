# strength-stage2-iteration-ltc

## 目的

棋力向上段階2の反復継続の判断（固定比2.5による次反復の完了予測）を、採用済みの予算式と標準LTCのGSPRTで比較し、採否を確定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage2-iteration-ltc --seed 20270913 \
  --candidate commit:62b5f04 --baseline commit:96aa2ef \
  --each time=60000+600 gsprt
```

## エンジン

候補はコミット62b5f04、基準は予算式を採用したコミット96aa2ef、規則セットは`engine-default`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

1,576ペアを実行し、有効ペア1,551、破棄ペア25（手数上限）であった。
ペンタノミアル度数は[284, 38, 804, 40, 385]、LLRは+2.949で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は70,632秒である。

## 結論

長時間GSPRTが`H1`かつ異常0件で終了したため、反復継続の判断を採用する。
予算式のLTCが段階開始版を基準に`H1`であることと合わせ、項目ごとのLTCの連鎖により最終構成の採用が確定する。
