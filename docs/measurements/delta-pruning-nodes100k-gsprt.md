# 静止探索のdelta pruningの採否GSPRT（nodes=100000）

## 目的

静止探索にdelta pruningを導入したコミットと直前コミットを同一ノード数の対等条件で対戦させ、棋力を有意に高めるかを判定する。

## コマンドライン

```console
match_runner --seed 20260827 --candidate commit:facd37f --baseline commit:38bef0a --each nodes=100000 --concurrency 8 gsprt
```

## エンジン

候補はコミットfacd37f（delta pruningあり、探索部マイルストーンの最終コミット）、基準はコミット38bef0a（late move reductions採用時点）である。
規則セットは`commit:`指定の既定であるengine-defaultである。

## 環境

CPU型、物理コア数、論理コア数は記録なし。
ワーカー数は候補と基準とも1である（本マイルストーンは単一スレッド設計）。
`USI_Hash`は記録なし（設計上の既定値は256MB）。
同時対局数は8である。

## 結果

ペンタノミアル度数は[32, 1, 162, 3, 137]（有効335ペア）であった。
LLRは2.947で判定はH1（候補が有意に強い）である。
破棄ペア数は3、異常件数は0である。
`time_forfeits`は該当なし（ノード数固定）。
経過時間は2777秒である。
benchの総ノード数は、depth=3で6,345,744から1,190,540へ、depth=4で13,976,111から4,573,594へ減った。

## 結論

静止探索のdelta pruningを採用する。
