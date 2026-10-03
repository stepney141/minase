# search-revival-t

## 目的

[不採用だった探索部の改良の再調整](../plans/search-revival-spsa.md)の処置群Tについて、既存の探索係数16個と新しい係数13個の29係数をSPSAで調整し、候補T\*の値を得る。
SPSAの対局結果は採否の根拠にしない。

## コマンドライン

```console
spsa_runner --run-dir data/spsa/search-revival-t --seed 2026800000 \
  --engine commit:85f6e87 --params data/spsa/search-revival-params/session.txt \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 375 --pairs-per-iteration 8
```

パラメーターファイルは、`spsa_runner params`が85f6e87から生成した既定のファイルから時間管理の6係数を除き、新しい13係数の`c_end`を設計書の表の値で上書きしたものである。
`r_end`はすべて既定の0.002、利得の指数はα = 0.602、γ = 0.101、A = 37.5である。

## エンジン

調整対象はコミット85f6e87（M = b96a931に8項目を戻し、新しい係数を開始値に置いた処置群T）の調整用ビルドである。
runnerは同じブランチのビルドを`data/spsa/search-revival-runner/spsa_runner`に複製して使い、そのSHA-256は7740a85fb9991feefc6b3bdb5f9556f56c642980ecc019a911c17141cedad810である。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、`Threads`は1、`USI_Hash`は既定の256MB、同時対局数は16である。

## 結果

375反復を適用し、有効ペア2,817、手数上限による破棄ペア183であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は29,463秒（約8.2時間）である。

θの開始値と最終値（整数へ丸めた値）は次のとおりである。
移動量は、開始値から最終値までの変化を`c_end`で割った値である。

| 係数 | 開始値 | 最終値 | 移動量 |
|---|---:|---:|---:|
| `LmrDivisor` | 166 | 156 | −0.20 |
| `LmrHistoryThreshold` | 111 | 146 | +0.40 |
| `FutilityMargin1` | 101 | 165 | +0.96 |
| `FutilityMargin2` | 196 | 239 | +0.64 |
| `FutilityMargin3` | 207 | 283 | +1.14 |
| `NonImprovingFutility1` | 63 | 71 | +0.45 |
| `NonImprovingFutility2` | 75 | 72 | −0.20 |
| `NonImprovingFutility3` | 75 | 86 | +0.78 |
| `LmpBase` | 300 | 382 | +0.50 |
| `LmpSlope` | 100 | 133 | +0.60 |
| `ReverseFutilityMargin` | 994 | 428 | −1.75 |
| `RazoringMargin1` | 1,534 | 1,606 | +0.16 |
| `RazoringMargin2` | 1,643 | 2,015 | +0.77 |
| `SeeMargin1` | 2 | 10 | +0.13 |
| `SeeMargin2` | 210 | 192 | −0.26 |
| `SeeMargin3` | 7 | 13 | +0.09 |
| `AspirationDelta` | 46 | 55 | +0.29 |
| `AspirationGrowth` | 201 | 190 | −0.25 |
| `NullMoveBase` | 3,529 | 3,617 | +0.15 |
| `NullMoveSlope` | 238 | 257 | +0.39 |
| `NullMoveEvalScale` | 50 | 56 | +0.23 |
| `HistoryLimit` | 20,755 | 17,408 | −0.33 |
| `HistoryDecay` | 37 | 23 | −0.81 |
| `CaptureHistoryLimit` | 20,755 | 16,950 | −0.42 |
| `CaptureHistoryScale` | 50 | 49 | −0.04 |
| `CorrectionCap` | 193 | 220 | +0.46 |
| `CorrectionWeight` | 33 | 37 | +0.21 |
| `DeltaMargin` | 258 | 355 | +1.30 |
| `QsearchMoveLimit` | 4 | 4 | +0.48 |

移動量の算出には、丸める前の最終値を使った。

## 結論

最終値を書き込んだコミット539bc27をT\*とし、既存の探索係数16個の最終値だけをMへ写したコミット2d5b60e（ブランチ`search-revival-t-prime`）をT\*′とする。
厳密な無効値へ達した係数はなく、reverse futility pruningの余裕値は開始値の半分以下へ狭まり、futility pruningとdelta pruningの余裕値は広がった。
採否は設計書の「効果の帰属」の手順で判定する。
