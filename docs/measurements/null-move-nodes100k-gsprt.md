# null move pruningの採否GSPRT（nodes=100000）

## 目的

null move pruningを導入したコミットと直前コミットを同一ノード数の対等条件で対戦させ、棋力を有意に高めるかを判定する。

## コマンドライン

```console
match_runner --seed 20260825 --candidate commit:06f7fd6 --baseline commit:5a5ae33 --each nodes=100000 --concurrency 8 gsprt
```

## エンジン

候補はコミット06f7fd6（null move pruningあり）、基準はコミット5a5ae33（killer手とhistory採用時点）である。
規則セットは`commit:`指定の既定であるengine-defaultである。

## 環境

CPU型、物理コア数、論理コア数は記録なし。
ワーカー数は候補と基準とも1である（本マイルストーンは単一スレッド設計）。
`USI_Hash`は記録なし（設計上の既定値は256MB）。
同時対局数は8である。

## 結果

ペンタノミアル度数は[357, 8, 1335, 16, 445]（有効2161ペア）であった。
LLRは2.945で判定はH1（候補が有意に強い）である。
破棄ペア数は12、異常件数は0である。
`time_forfeits`は該当なし（ノード数固定）。
経過時間は12034秒である。
benchの総ノード数は、depth=4で77,415,194から15,931,373へ減った。

## 結論

null move pruningを採用する。効果が数十Elo規模のため、docs/sprt.md の目安どおり判定に2000ペア超を要した。
