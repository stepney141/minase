# killer手とhistoryヒューリスティックの採否GSPRT（nodes=100000）

## 目的

指し手順序付けにkiller手とhistoryヒューリスティックを加えたコミットと直前コミットを同一ノード数の対等条件で対戦させ、棋力を有意に高めるかを判定する。

## コマンドライン

```console
match_runner --seed 20260823 --candidate commit:5a5ae33 --baseline commit:8da1d24 --each nodes=100000 --concurrency 8 gsprt
```

## エンジン

候補はコミット5a5ae33（killer手とhistoryあり）、基準はコミット8da1d24（mimallocとVec巻き上げ採用時点）である。
規則セットは`commit:`指定の既定であるengine-defaultである。

## 環境

CPU型、物理コア数、論理コア数は記録なし。
ワーカー数は候補と基準とも1である（本マイルストーンは単一スレッド設計）。
`USI_Hash`は記録なし（設計上の既定値は256MB）。
同時対局数は8である。

## 結果

ペンタノミアル度数は[77, 3, 248, 3, 185]（有効516ペア）であった。
LLRは2.949で判定はH1（候補が有意に強い）である。
破棄ペア数は5、異常件数は0である。
`time_forfeits`は該当なし（ノード数固定）。
経過時間は3875秒である。
benchの総ノード数は、depth=3で7,393,052から6,345,744へ減った。

## 結論

killer手とhistoryヒューリスティックを採用する。
