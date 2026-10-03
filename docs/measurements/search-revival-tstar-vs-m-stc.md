# search-revival-tstar-vs-m-stc

## 目的

[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)の段階2として、調整値を書き込んだ候補T\*とMを標準STCのGSPRTで比較し、次の段階へ進めるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/search-revival-tstar-vs-m-stc --seed 2026900000 \
  --candidate commit:539bc27 --baseline commit:b96a931 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット539bc27（T\*、[調整セッション](search-revival-t.md)の最終値を書き込んだ処置群）、基準はコミットb96a931（M、評価関数のPSTに重みG23を採用したmaster）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は既定の256MB、同時対局数は自動計算の19である。

## 結果

157ペアを実行し、有効ペア147、手数上限による破棄ペア10であった。
ペンタノミアル度数は[9, 6, 65, 7, 60]であり、候補の得点率は67.5%（198.5／294）である。
LLRは2.969で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は1,661秒である。

## 結論

段階2を通過したので、段階3のT\*対T\*′のSTCへ進む。
採用の決定は段階4のLTCが下すので、この結果だけでは採用しない。
