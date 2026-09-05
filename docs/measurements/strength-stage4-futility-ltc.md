# strength-stage4-futility-ltc

## 目的

棋力向上段階4のfutility pruning（残り深さ3以下の非PVノードで、静的評価に深さ別の余裕値を加えてもαに届かないとき、置換表の記録手、捕獲手、成る手を除く静かな手を展開しない）を、段階開始版と標準LTCのGSPRTで比較し、採否を確定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage4-futility-ltc --seed 20370903 \
  --candidate commit:a641083 --baseline commit:021fbb3 \
  --each time=60000+200 gsprt
```

## エンジン

候補はコミットa641083、基準は段階開始版のコミット021fbb3（探索コードは段階3完了時の20a7f48と同一）、規則セットは`L0,P0,R1,E0`である。
余裕値は残り深さ1が歩兵価値の半分、深さ2と3が歩兵価値の1.5倍である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は自動計算の19である。

## 結果

218ペアを実行し、有効ペア216、破棄ペア2（手数上限）であった。
ペンタノミアル度数は[25, 5, 101, 12, 73]、LLRは+2.949で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は5,082秒である。

## 結論

長時間GSPRTは`H1`かつ異常0件であり、futility pruningを採用する。
