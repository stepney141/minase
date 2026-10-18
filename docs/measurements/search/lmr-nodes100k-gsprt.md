# late move reductionsの採否GSPRT（nodes=100000）

## 目的

late move reductionsを導入したコミットと直前コミットを同一ノード数の対等条件で対戦させ、棋力を有意に高めるかを判定する。

## コマンドライン

```console
match_runner --seed 20260826 --candidate commit:38bef0a --baseline commit:06f7fd6 --each nodes=100000 --concurrency 8 gsprt
```

## エンジン

候補はコミット38bef0a（late move reductionsあり）、基準はコミット06f7fd6（null move pruning採用時点）である。
規則セットは`commit:`指定の既定であるengine-defaultである。

## 環境

CPU型、物理コア数、論理コア数は記録なし。
ワーカー数は候補と基準とも1である（本マイルストーンは単一スレッド設計）。
`USI_Hash`は記録なし（設計上の既定値は256MB）。
同時対局数は8である。

## 結果

ペンタノミアル度数は[161, 6, 541, 4, 258]（有効970ペア）であった。
LLRは2.956で判定はH1（候補が有意に強い）である。
破棄ペア数は7、異常件数は0である。
`time_forfeits`は該当なし（ノード数固定）。
経過時間は5782秒である。
benchの総ノード数は、depth=4で15,931,373から13,976,111へ減った。

## 結論

late move reductionsを採用する。
