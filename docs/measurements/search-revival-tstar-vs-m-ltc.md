# search-revival-tstar-vs-m-ltc

## 目的

[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)の段階4として、候補T\*とMを標準LTCのGSPRTで比較し、T\*の採否を決める。

## コマンドライン

```console
match_runner --run-dir data/matches/search-revival-tstar-vs-m-ltc --seed 2027100000 \
  --candidate commit:539bc27 --baseline commit:b96a931 \
  --each time=60000+200 gsprt
```

基本シードは、段階2のSTC（2026900000）と段階3のSTC（2027000000）から、それぞれの上限3,000ペア以上離した。

## エンジン

候補はコミット539bc27（T\*、[調整セッション](search-revival-t.md)の最終値を書き込んだ処置群）、基準はコミットb96a931（M、評価関数のPSTに重みG23を採用したmaster）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は既定の256MB、同時対局数は自動計算の19である。

## 結果

170ペアを実行し、有効ペア162、手数上限による破棄ペア8であった。
ペンタノミアル度数は[14, 4, 67, 9, 68]であり、候補の得点率は67.4%（218.5／324）である。
LLRは2.980で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は4,202秒である。

## 結論

`H1`かつ異常0件なので、T\*を採用する。
段階3の[T\*対T\*′のSTC](search-revival-tstar-vs-tprime-stc.md)も`H1`であり、向上には戻した8項目が寄与している。
