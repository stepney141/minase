# search-revival-tstar-vs-tprime-stc

## 目的

[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)の段階3として、候補T\*と、T\*から8項目と新しい係数だけを取り除いたT\*′を標準STCのGSPRTで比較し、T\*のMに対する向上を8項目に帰属できるかを判定する。

## コマンドライン

```console
match_runner --run-dir data/matches/search-revival-tstar-vs-tprime-stc --seed 2027000000 \
  --candidate commit:539bc27 --baseline commit:2d5b60e \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット539bc27（T\*）、基準はコミット2d5b60e（T\*′、M = b96a931に[調整セッション](search-revival-t.md)の既存の探索係数16個の最終値だけを写した版、ブランチ`search-revival-t-prime`）である。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、候補と基準の`Threads`は1、`USI_Hash`は既定の256MB、同時対局数は自動計算の19である。

## 結果

233ペアを実行し、有効ペア222、手数上限による破棄ペア11であった。
ペンタノミアル度数は[29, 3, 98, 10, 82]であり、候補の得点率は62.7%（278.5／444）である。
LLRは2.981で`decision: H1`である。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は2,073秒である。

## 結論

T\*′に対して`H1`なので、調整した系から8項目を取り除くと弱くなり、T\*のMに対する向上には8項目が寄与していると読める。
設計書の「帰属の判定の偏り」のとおり、この判定はT\*′を既存の係数だけで調整し直した版より強いとは主張しない。
段階4のT\*対MのLTCへ進む。
