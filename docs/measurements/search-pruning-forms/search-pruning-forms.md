# 枝刈りとLMRの関数形を変えた候補のSPSA

## 目的

[探索の枝刈りとLMRの関数形の見直し](../../plans/search-pruning-forms.md)のフェーズ3として、形の変更の最後のコミット`388c5244`の68係数をSPSAで30,000ペア調整し、採否の測定にかける候補の値を得る。
SPSAの対局結果は採否の根拠にしない。
最終値は、この予算と設定で得た候補の値であり、最適値とは呼ばない。

## コマンドライン

[統括計画](../../plans/search-forms.md)の調整機で、そのスクリプトの完成を待たずに、時間切れの事前確認を通した後に次を手作業で起動した。

```console
target/release/minase spsa --run-dir /workspace/job/search-pruning-forms --seed 3250000000 \
  --engine commit:388c5244 --params /workspace/job/search-pruning-forms.params \
  --rules engine-default --each time=14600+146 --concurrency 95 \
  --iterations 3750 --pairs-per-iteration 8
```

パラメーターファイルは`data/spsa/search-pruning-forms.params`（68行）であり、内容は次の表の範囲、`c_end`、および開始値のとおりである。
`r_end`はすべて0.002であり、利得の指数はα = 0.602とγ = 0.101、学習率の安定化の定数はA = 375である。
持ち時間の`time=14600+146`は、測定機のSTC`time=10000+100`の基本時間と加算に、統括計画の換算係数`f` = 1.46を掛けた値である。
実行ディレクトリは、30分ごとに測定機の`data/vast-spsa/search-pruning-forms/`へ取得し、終了後に`data/spsa/search-pruning-forms`へ移した。

## エンジン

調整対象は`388c5244950a8fec1dcc7738b1c6b48e1426f532`の調整用ビルドであり、調整機でビルドした実行ファイルのSHA-256は`c4a1b7b5e73956688befce57e3baff809393ffb080e44dde6337881376c5f311`である。
runnerは、調整機で同じコミットをビルドした通常のminase（2.4.0）であり、そのSHA-256は`029a87159c8955a11910e6019cc99b0625aaa630e2afc6d62c9b566f71353483`である。
`invocations.json`の起動の記録は1件であり、セッションは中断なく1回の起動で終えた。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiの出品55530776で借りたAMD Ryzen Threadripper PRO 9995WX（ノルウェー、機械のID 7584、インスタンス56409205）である。
物理96コア、論理192スレッドで、`cpu_ram`は257,025 MB、時間単価は1.285米ドルである。
runnerが記録したCPUの数は、物理96コア、論理184スレッドである。論理スレッドが192より少ないのは、コンテナのCPU時間の上限による。
`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は95である。
測定機（Intel Core Ultra 7 265KF）の負荷時の速度は1,264,584 NPS、調整機の負荷時の速度は868,714 NPSであり、`f`は1.46である（[試走の記録](../search-forms/vast-spsa-trial.md)）。
時間切れの事前確認は同じ調整機で78反復を行い、`time_forfeits`を含む異常は0件だった（[記録](../search-pruning-forms-safety.md)）。
調整機を作り直して再開したことはない。

セッションは2026年10月18日1時16分（日本時間）に始まり、17時28分に終わった。
セッションの費用は約20.8米ドル（16.2時間 × 1時間1.285米ドル）である。

## 結果

3,750反復をすべて適用し、有効ペア29,600、手数上限による破棄ペア400であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は58,304秒（約16.2時間）である。

θの開始値と最終値は次のとおりである。
最終値（整数）は`minase spsa apply`が四捨五入した値であり、範囲の端に達した係数には「上限」または「下限」と付記した。
移動量は、開始値から最終値（実数）までの変化を`c_end`で割った値である。

| 係数 | 範囲 | `c_end` | 開始値 | 最終値（実数） | 最終値（整数） | 移動量 |
|---|---|---:|---:|---:|---:|---:|
| `LmrDivisor` | 100〜400 | 50.00 | 203 | 147.14 | 147 | −1.12 |
| `LmrHistoryThreshold` | 0〜512 | 85.33 | 146 | 34.79 | 35 | −1.30 |
| `LmrMaxReduction` | 1〜8 | 1.17 | 3 | 3.07 | 3 | +0.06 |
| `LmrBase` | 0〜200 | 33.33 | 93 | 126.62 | 127 | +1.01 |
| `LmrOffset` | 0〜200 | 33.33 | 68 | 106.03 | 106 | +1.14 |
| `LmrNonImproving` | 0〜100 | 16.67 | 38 | 3.34 | 3 | −2.08 |
| `LmrPvReduction` | 0〜200 | 33.33 | 50 | 5.53 | 6 | −1.33 |
| `LmrTtPv` | 0〜200 | 33.33 | 91 | 70.05 | 70 | −0.63 |
| `LmrTtPvBase` | 0〜500 | 83.33 | 295 | 337.25 | 337 | +0.51 |
| `LmrTtPvPv` | 0〜200 | 33.33 | 98 | 13.43 | 13 | −2.54 |
| `LmrTtPvValue` | 0〜200 | 33.33 | 86 | 94.93 | 95 | +0.27 |
| `LmrTtPvDepth` | 0〜200 | 33.33 | 80 | 76.83 | 77 | −0.10 |
| `LmrTtPvDepthCutNode` | 0〜200 | 33.33 | 92 | 90.45 | 90 | −0.05 |
| `LmrMoveCount` | 0〜20 | 3.33 | 6 | 0.30 | 0（下限） | −1.71 |
| `LmrCutNode` | 0〜600 | 100.00 | 393 | 163.56 | 164 | −2.29 |
| `LmrCutNodeNoTtMove` | 0〜200 | 33.33 | 91 | 59.13 | 59 | −0.96 |
| `LmrTtCapture` | 0〜250 | 41.67 | 105 | 0.08 | 0（下限） | −2.52 |
| `LmrCutoffCount` | 0〜100 | 16.67 | 26 | 48.11 | 48 | +1.33 |
| `LmrCutoffCountMany` | 0〜250 | 41.67 | 107 | 231.62 | 232 | +2.99 |
| `LmrCutoffCountAllNode` | 0〜250 | 41.67 | 111 | 129.41 | 129 | +0.44 |
| `LmrTtMove` | 0〜500 | 83.33 | 213 | 268.69 | 269 | +0.67 |
| `LmrAlphaEval` | 0〜150 | 25.00 | 61 | 46.32 | 46 | −0.59 |
| `LmrNoTtMove` | 0〜250 | 41.67 | 110 | 56.65 | 57 | −1.28 |
| `LmrAllNode` | 0〜200 | 33.33 | 108 | 141.78 | 142 | +1.01 |
| `LmrDeeperMargin` | 0〜100 | 16.67 | 25 | 82.29 | 82 | +3.44 |
| `LmrShallowerMargin` | 0〜40 | 6.67 | 4 | 25.98 | 26 | +3.30 |
| `FutilityMargin1` | 0〜400 | 66.67 | 165 | 317.13 | 317 | +2.28 |
| `FutilityMargin2` | 0〜400 | 66.67 | 239 | 263.88 | 264 | +0.37 |
| `FutilityMargin3` | 0〜400 | 66.67 | 283 | 314.67 | 315 | +0.47 |
| `NonImprovingFutility1` | 0〜100 | 16.67 | 71 | 82.36 | 82 | +0.68 |
| `NonImprovingFutility2` | 0〜100 | 16.67 | 72 | 80.86 | 81 | +0.53 |
| `NonImprovingFutility3` | 0〜100 | 16.67 | 86 | 84.39 | 84 | −0.10 |
| `LmpBase` | 0〜8,600 | 1433.33 | 300 | 1,798.72 | 1,799 | +1.05 |
| `LmpSlope` | 0〜3,600 | 600.00 | 100 | 47.90 | 48 | −0.09 |
| `LmpNonImproving` | 25〜150 | 20.83 | 50 | 30.66 | 31 | −0.93 |
| `ReverseFutilityMultBase` | 0〜450 | 75.00 | 22 | 235.17 | 235 | +2.84 |
| `ReverseFutilityMultSlope` | 0〜12 | 2.00 | 2 | 3.71 | 4 | +0.86 |
| `ReverseFutilityNoTtHit` | 0〜50 | 8.33 | 10 | 0.42 | 0（下限） | −1.15 |
| `ReverseFutilityImproving` | 0〜400 | 66.67 | 272 | 144.47 | 144 | −1.91 |
| `ReverseFutilityOpponentWorsening` | 0〜100 | 16.67 | 33 | 20.14 | 20 | −0.77 |
| `ReverseFutilityCorrection` | 0〜200 | 33.33 | 66 | 123.70 | 124 | +1.73 |
| `RazoringMargin1` | 0〜2,668 | 444.67 | 1,606 | 2,622.09 | 2,622 | +2.29 |
| `RazoringMargin2` | 0〜2,885 | 480.83 | 2,015 | 2,885.00 | 2,885（上限） | +1.81 |
| `SeeMargin1` | 0〜400 | 66.67 | 10 | 14.57 | 15 | +0.07 |
| `SeeMargin2` | 0〜400 | 66.67 | 192 | 97.29 | 97 | −1.42 |
| `SeeMargin3` | 0〜400 | 66.67 | 13 | 2.76 | 3 | −0.15 |
| `AspirationDelta` | 10〜200 | 31.67 | 55 | 56.75 | 57 | +0.06 |
| `AspirationGrowth` | 125〜400 | 45.83 | 190 | 167.57 | 168 | −0.49 |
| `NullMoveBase` | 1,200〜4,800 | 600.00 | 3,617 | 4,798.80 | 4,799 | +1.97 |
| `NullMoveSlope` | 100〜400 | 50.00 | 257 | 392.89 | 393 | +2.72 |
| `NullMoveEvalScale` | 0〜400 | 66.67 | 56 | 132.98 | 133 | +1.15 |
| `HistoryLimit` | 4,096〜65,536 | 10240.00 | 17,408 | 5,808.07 | 5,808 | −1.13 |
| `HistoryBonusScale` | 0〜400 | 66.67 | 100 | 206.66 | 207 | +1.60 |
| `HistoryMalusScale` | 0〜400 | 66.67 | 100 | 109.86 | 110 | +0.15 |
| `HistoryDecay` | 0〜100 | 16.67 | 23 | 36.73 | 37 | +0.82 |
| `CaptureHistoryDecay` | 0〜100 | 16.67 | 23 | 13.44 | 13 | −0.57 |
| `CaptureHistoryLimit` | 4,096〜65,536 | 10240.00 | 16,950 | 8,375.69 | 8,376 | −0.84 |
| `CaptureHistoryScale` | 0〜400 | 66.67 | 49 | 155.30 | 155 | +1.59 |
| `CorrectionCap` | 50〜400 | 58.33 | 220 | 180.88 | 181 | −0.67 |
| `CorrectionWeight` | 8〜128 | 20.00 | 37 | 75.89 | 76 | +1.94 |
| `DeltaMargin` | 50〜500 | 75.00 | 355 | 499.85 | 500（上限） | +1.93 |
| `QsearchMoveLimit` | 1〜7 | 1.00 | 4 | 6.06 | 6 | +2.06 |
| `ExpectedPlies` | 250〜700 | 75.00 | 432 | 457.96 | 458 | +0.35 |
| `MinMoves` | 40〜200 | 26.67 | 88 | 40.06 | 40（下限） | −1.80 |
| `IncrementShare` | 30〜100 | 11.67 | 76 | 86.70 | 87 | +0.92 |
| `HardSoftRatio` | 150〜800 | 108.33 | 451 | 730.09 | 730 | +2.58 |
| `HardRemainingShare` | 10〜50 | 6.67 | 27 | 39.05 | 39 | +1.81 |
| `IterationRatio` | 150〜400 | 41.67 | 263 | 276.69 | 277 | +0.33 |

丸めた最終値が範囲の端に達した係数は6個である。
下限に達したのは`LmrMoveCount`、`LmrTtCapture`、`ReverseFutilityNoTtHit`、および`MinMoves`、上限に達したのは`RazoringMargin2`と`DeltaMargin`である。
`LmrMaxReduction`の最終値は3であり、暫定の上限8には達していない。

新しい31係数と開始値を置き直した3係数について、最終値が開始値（Stockfishの換算値）と現行の値からどれだけ離れたかを、`c_end`に対する比で次に示す。
現行の値は、設計書の新しい係数の表の「現行」の値であり、現行に対応する項がない係数では項が働かない値（多くは0）である。
`LmrDeeperMargin`と`LmrShallowerMargin`は、現行の探索に対応する調整がないので、現行の値からの距離を示さない。
置き直した3係数（`LmrDivisor`、`LmpBase`、`LmpSlope`）の現行の値は、起点の表の値である。
この比は、SPSAがStockfishの値と現行の挙動のどちらへ寄ったかを読むための目安であり、採否には使わない。

| 係数 | Stockfishの換算値（開始値） | 現行の値 | 最終値 | 開始値からの移動量 | 現行の値からの距離 |
|---|---:|---:|---:|---:|---:|
| `LmrDivisor` | 203 | 156 | 147 | −1.12 | −0.18 |
| `LmrMaxReduction` | 3 | 3 | 3 | +0.06 | +0.06 |
| `LmrBase` | 93 | 0 | 127 | +1.01 | +3.80 |
| `LmrOffset` | 68 | 0 | 106 | +1.14 | +3.18 |
| `LmrNonImproving` | 38 | 0 | 3 | −2.08 | +0.20 |
| `LmrPvReduction` | 50 | 0 | 6 | −1.33 | +0.17 |
| `LmrTtPv` | 91 | 0 | 70 | −0.63 | +2.10 |
| `LmrTtPvBase` | 295 | 0 | 337 | +0.51 | +4.05 |
| `LmrTtPvPv` | 98 | 0 | 13 | −2.54 | +0.40 |
| `LmrTtPvValue` | 86 | 0 | 95 | +0.27 | +2.85 |
| `LmrTtPvDepth` | 80 | 0 | 77 | −0.10 | +2.30 |
| `LmrTtPvDepthCutNode` | 92 | 0 | 90 | −0.05 | +2.71 |
| `LmrMoveCount` | 6 | 0 | 0 | −1.71 | +0.09 |
| `LmrCutNode` | 393 | 0 | 164 | −2.29 | +1.64 |
| `LmrCutNodeNoTtMove` | 91 | 0 | 59 | −0.96 | +1.77 |
| `LmrTtCapture` | 105 | 0 | 0 | −2.52 | +0.00 |
| `LmrCutoffCount` | 26 | 0 | 48 | +1.33 | +2.89 |
| `LmrCutoffCountMany` | 107 | 0 | 232 | +2.99 | +5.56 |
| `LmrCutoffCountAllNode` | 111 | 0 | 129 | +0.44 | +3.11 |
| `LmrTtMove` | 213 | 0 | 269 | +0.67 | +3.22 |
| `LmrAlphaEval` | 61 | 0 | 46 | −0.59 | +1.85 |
| `LmrNoTtMove` | 110 | 0 | 57 | −1.28 | +1.36 |
| `LmrAllNode` | 108 | 0 | 142 | +1.01 | +4.25 |
| `LmrDeeperMargin` | 25 | — | 82 | +3.44 | — |
| `LmrShallowerMargin` | 4 | — | 26 | +3.30 | — |
| `LmpBase` | 300 | 382 | 1,799 | +1.05 | +0.99 |
| `LmpSlope` | 100 | 133 | 48 | −0.09 | −0.14 |
| `LmpNonImproving` | 50 | 100 | 31 | −0.93 | −3.33 |
| `ReverseFutilityMultBase` | 22 | 428 | 235 | +2.84 | −2.57 |
| `ReverseFutilityMultSlope` | 2 | 0 | 4 | +0.86 | +1.86 |
| `ReverseFutilityNoTtHit` | 10 | 0 | 0 | −1.15 | +0.05 |
| `ReverseFutilityImproving` | 272 | 0 | 144 | −1.91 | +2.17 |
| `ReverseFutilityOpponentWorsening` | 33 | 0 | 20 | −0.77 | +1.21 |
| `ReverseFutilityCorrection` | 66 | 0 | 124 | +1.73 | +3.71 |

34係数のうち、開始値からの移動量の絶対値が1を超えたのは20個である。
現行の値が0の係数のうち、`LmrTtCapture`は0まで、`LmrMoveCount`と`ReverseFutilityNoTtHit`は0の近くまで動き、これらの項はほとんど働かない値になった。
一方、`LmrCutoffCountMany`、`LmrAllNode`、`LmrTtPvBase`、`LmrBase`、および`ReverseFutilityCorrection`は、開始値よりさらに0から離れた。

## 結論

68係数の30,000ペアのSPSAは、異常0件で完了した。
候補の値は、ブランチ`search-pruning-forms`のコミット`d52aefb3`で係数の表の既定値へ書き込み、事前対局と採否の測定にかける。
