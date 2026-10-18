# strength-stage6-aspiration-ltc

## 目的

棋力向上段階6のaspiration windows（[STC](strength-stage6-aspiration-stc.md)で`H1`）を、段階開始版と標準LTCのGSPRTで比較し、採否を確定する。

## コマンドライン

```console
match_runner --run-dir data/matches/strength-stage6-aspiration-ltc --seed 20610903 \
  --candidate commit:75bb69d --baseline commit:75a08de \
  --each time=60000+200 --concurrency 17 gsprt
```

## エンジン

候補はコミット75bb69d、基準は段階開始版のコミット75a08de、規則セットは`L0,P0,R1,E0`である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は256MB、同時対局数は明示した17である。
測定の途中で、時間管理の信号の計数（3プロセス）とcodexによる実装作業が並行していた。

## 結果

185ペアを実行し、有効ペア180、破棄ペア5（手数上限）であった。
ペンタノミアル度数は[17, 3, 87, 6, 67]、LLRは+2.961で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は5,598秒である。

## 結論

長時間GSPRTは`H1`かつ異常0件であり、aspiration windowsを採用する。
以後の項目は本コミットを含む構成を基準に測る。
