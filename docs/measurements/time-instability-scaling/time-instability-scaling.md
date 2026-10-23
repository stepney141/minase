# 持ち時間の伸縮の形の変更の候補のSPSA

## 目的

[探索の不安定さに応じた持ち時間の伸縮の設計書](../../plans/archived/time-instability-scaling.md)のフェーズ3として、形の変更の最後のコミット`68d3a4d4`の103係数をSPSAで30,000ペア調整し、事前対局とSTCにかける候補の値を得る。
SPSAの対局結果は強さの根拠にしない。
最終値は、この予算と設定で得た候補の値であり、最適値とは呼ばない。

## コマンドライン

[統括計画](../../plans/search-forms.md)の調整機で、`scripts/vast-spsa/remote.py`が[時間切れの事前確認](time-instability-scaling-safety.md)を通した後に次を実行した。

```console
target/release/minase spsa --run-dir /workspace/job/time-instability-scaling --seed 5050000000 \
  --engine commit:68d3a4d491c94dee29f979b8de70025fca902565 --params /workspace/job/params \
  --rules engine-default --each time=10000+100 --concurrency 95 \
  --iterations 3750 --pairs-per-iteration 8
```

パラメーターファイルは`data/spsa/time-instability-scaling.params`（103行）であり、内容は次の表の範囲、`c_end`、および開始値のとおりである。
`r_end`はすべて0.002であり、利得の指数はα = 0.602とγ = 0.101、学習率の安定化の定数はA = 375である。
持ち時間の`time=10000+100`は、測定機のSTCの基本時間と加算に換算係数`f` = 1.00を掛けた値であり、STCと同じになった。換算係数の求め方は[事前確認の記録](time-instability-scaling-safety.md)にある。
実行ディレクトリは、`scripts/vast-spsa/fetch.py --keep`が10分ごとに測定機の`data/vast-spsa/time-instability-scaling/`へ取得した。全反復がそろった時点で記録を検査し、終了後に`data/spsa/time-instability-scaling`へ移した。

## エンジン

調整対象は`68d3a4d491c94dee29f979b8de70025fca902565`の調整用ビルドであり、調整機でビルドした実行ファイルのSHA-256は`f4e99032b39c7cdad1a974d5b8f6935402c6c6bf16f88173ad7feefe57f0bcef`である。
runnerは、調整機で同じコミットをビルドした通常のminase（2.4.0）であり、そのSHA-256は`462b83dec31543a69a56b218855af13dd9ecc68b6251d7c0664f81f75a1341fb`である。
`invocations.json`の起動の記録は1件であり、セッションは中断なく1回の起動で終えた。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiの出品57221898で借りたAMD Ryzen Threadripper PRO 9995WX（ギリシャ、機械のID 154306、インスタンス57236964）である。
物理96コア、論理192スレッドで、`cpu_ram`は192,513 MB、時間単価は約2.00米ドルである。
runnerが記録したCPUの数は、物理96コア、論理184スレッドである。コンテナのCPU割り当てが184 CPUぶんだったためである。
`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は95である。
測定機（Intel Core Ultra 7 265KF）の負荷時の速度は448,693 NPS、調整機の95本の負荷時の速度は450,346 NPSであり、`f`は1.00である。
開始時に見込んだ所要時間は12.70時間だった。
調整機を作り直して再開したことはない。

統括計画の候補とした物理192コアのEPYCの出品が出なかったので、この調整機は利用者の決定で借りた。経緯は[事前確認の記録](time-instability-scaling-safety.md)にある。
作成は2026年10月23日1時5分（日本時間）である。
セッションは2026年10月23日1時29分ごろに始まり、12時56分ごろに終わった。

## 結果

3,750反復をすべて適用し、有効ペア29,524、手数上限による破棄ペア476であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は41,181秒（約11.4時間）である。

θ（調整中の係数の中心値の組）の開始値と最終値は次のとおりである。
最終値（整数）は`minase spsa apply`が丸めた値であり、範囲の端に達した係数には「上限」または「下限」と付記した。
移動量は、開始値から最終値（実数）までの変化を`c_end`で割った値である。

| 係数 | 範囲 | `c_end` | 開始値 | 最終値（実数） | 最終値（整数） | 移動量 |
|---|---|---:|---:|---:|---:|---:|
| `LmrDivisor` | 100〜400 | 50.00 | 188 | 200.71 | 201 | +0.25 |
| `LmrHistoryThreshold` | 0〜2,000 | 333.33 | 874 | 631.39 | 631 | −0.73 |
| `LmrHistoryScale` | 0〜1,000 | 166.67 | 39 | 108.54 | 109 | +0.42 |
| `LmrMaxReduction` | 1〜8 | 1.17 | 6 | 6.79 | 7 | +0.68 |
| `LmrBase` | 0〜200 | 33.33 | 16 | 16.82 | 17 | +0.02 |
| `LmrOffset` | 0〜200 | 33.33 | 188 | 196.33 | 196 | +0.25 |
| `LmrNonImproving` | 0〜100 | 16.67 | 17 | 3.16 | 3 | −0.83 |
| `LmrPvReduction` | 0〜200 | 33.33 | 117 | 131.99 | 132 | +0.45 |
| `LmrTtPv` | 0〜200 | 33.33 | 5 | 41.70 | 42 | +1.10 |
| `LmrTtPvBase` | 0〜500 | 83.33 | 47 | 4.57 | 5 | −0.51 |
| `LmrTtPvPv` | 0〜200 | 33.33 | 4 | 30.84 | 31 | +0.81 |
| `LmrTtPvValue` | 0〜200 | 33.33 | 116 | 96.57 | 97 | −0.58 |
| `LmrTtPvDepth` | 0〜200 | 33.33 | 41 | 34.74 | 35 | −0.19 |
| `LmrTtPvDepthCutNode` | 0〜200 | 33.33 | 140 | 192.87 | 193 | +1.59 |
| `LmrMoveCount` | 0〜20 | 3.33 | 1 | 0.31 | 0（下限） | −0.21 |
| `LmrCorrection` | 0〜200 | 33.33 | 34 | 40.49 | 40 | +0.19 |
| `LmrCutNode` | 0〜600 | 100.00 | 430 | 360.91 | 361 | −0.69 |
| `LmrCutNodeNoTtMove` | 0〜200 | 33.33 | 143 | 189.65 | 190 | +1.40 |
| `LmrTtCapture` | 0〜250 | 41.67 | 45 | 1.08 | 1 | −1.05 |
| `LmrCutoffCount` | 0〜100 | 16.67 | 45 | 50.50 | 51 | +0.33 |
| `LmrCutoffCountMany` | 0〜250 | 41.67 | 214 | 228.00 | 228 | +0.34 |
| `LmrCutoffCountAllNode` | 0〜250 | 41.67 | 213 | 195.76 | 196 | −0.41 |
| `LmrTtMove` | 0〜500 | 83.33 | 467 | 473.89 | 474 | +0.08 |
| `LmrAlphaEval` | 0〜150 | 25.00 | 23 | 1.11 | 1 | −0.88 |
| `LmrNoTtMove` | 0〜250 | 41.67 | 108 | 122.42 | 122 | +0.35 |
| `LmrAllNode` | 0〜200 | 33.33 | 135 | 124.57 | 125 | −0.31 |
| `LmrDeeperMargin` | 0〜100 | 16.67 | 93 | 95.89 | 96 | +0.17 |
| `LmrShallowerMargin` | 0〜40 | 6.67 | 37 | 34.03 | 34 | −0.45 |
| `ContinuationPruning` | 1,000〜12,000 | 1833.33 | 3,318 | 3,056.08 | 3,056 | −0.14 |
| `FutilityBase` | 0〜400 | 66.67 | 90 | 59.83 | 60 | −0.45 |
| `FutilityLmrDepth` | 0〜300 | 50.00 | 101 | 91.33 | 91 | −0.19 |
| `FutilityAboveAlpha` | -100〜300 | 66.67 | 112 | 116.42 | 116 | +0.07 |
| `QuietSeeMargin` | 0〜100 | 16.67 | 52 | 39.75 | 40 | −0.74 |
| `LmpBase` | 0〜8,600 | 1433.33 | 3,838 | 3,806.42 | 3,806 | −0.02 |
| `LmpSlope` | 0〜3,600 | 600.00 | 42 | 20.42 | 20 | −0.04 |
| `LmpNonImproving` | 25〜150 | 20.83 | 26 | 25.75 | 26 | −0.01 |
| `ReverseFutilityMultBase` | 0〜450 | 75.00 | 93 | 181.01 | 181 | +1.17 |
| `ReverseFutilityMultSlope` | 0〜12 | 2.00 | 9 | 8.05 | 8 | −0.47 |
| `ReverseFutilityNoTtHit` | 0〜50 | 8.33 | 6 | 4.08 | 4 | −0.23 |
| `ReverseFutilityImproving` | 0〜400 | 66.67 | 3 | 8.66 | 9 | +0.08 |
| `ReverseFutilityOpponentWorsening` | 0〜100 | 16.67 | 1 | 0.67 | 1 | −0.02 |
| `ReverseFutilityCorrection` | 0〜200 | 33.33 | 199 | 194.95 | 195 | −0.12 |
| `RazoringDepthMargin` | 0〜3,000 | 500.00 | 1,355 | 1,312.69 | 1,313 | −0.08 |
| `SeeDepthMargin` | 0〜400 | 66.67 | 362 | 300.94 | 301 | −0.92 |
| `SeeCaptureHistory` | -50〜200 | 41.67 | 198 | 160.25 | 160 | −0.91 |
| `CaptureFutilityBase` | 0〜600 | 100.00 | 337 | 356.42 | 356 | +0.19 |
| `CaptureFutilityLmrDepth` | 0〜500 | 83.33 | 451 | 386.37 | 386 | −0.78 |
| `CaptureFutilityHistory` | -100〜600 | 116.67 | 390 | 380.85 | 381 | −0.08 |
| `AspirationDelta` | 1〜200 | 33.17 | 100 | 95.44 | 95 | −0.14 |
| `AspirationScoreWidth` | 0〜400 | 66.67 | 63 | 4.81 | 5 | −0.87 |
| `AspirationGrowth` | 125〜400 | 45.83 | 146 | 156.29 | 156 | +0.22 |
| `NullMoveBase` | 1,200〜10,000 | 1466.67 | 9,720 | 9,838.71 | 9,839 | +0.08 |
| `NullMoveSlope` | 100〜600 | 83.33 | 508 | 556.06 | 556 | +0.58 |
| `NullMoveEvalScale` | 0〜400 | 66.67 | 180 | 283.30 | 283 | +1.55 |
| `NullMoveEvalMargin` | -1,000〜1,000 | 333.33 | 326 | -50.33 | -50 | −1.13 |
| `NullMoveEvalDepth` | -50〜100 | 25.00 | 62 | 45.21 | 45 | −0.67 |
| `NullMoveEvalImproving` | -200〜400 | 100.00 | 368 | 334.98 | 335 | −0.33 |
| `NullMoveFailHighBonus` | 0〜200 | 33.33 | 48 | 62.26 | 62 | +0.43 |
| `HistoryBonusSlope` | 0〜400 | 66.67 | 394 | 376.10 | 376 | −0.27 |
| `HistoryBonusBase` | -100〜300 | 66.67 | 21 | -43.17 | -43 | −0.96 |
| `HistoryBonusCap` | 200〜7,183 | 1163.83 | 2,981 | 2,008.59 | 2,009 | −0.84 |
| `HistoryMalusSlope` | 0〜2,500 | 416.67 | 750 | 691.93 | 692 | −0.14 |
| `HistoryMalusBase` | -200〜800 | 166.67 | -196 | -198.33 | -198 | −0.01 |
| `HistoryMalusCap` | 200〜7,183 | 1163.83 | 1,983 | 1,938.29 | 1,938 | −0.04 |
| `HistoryMalusDecay` | 700〜1,024 | 54.00 | 1,021 | 1,024.00 | 1,024（上限） | +0.06 |
| `HistoryTtCutoffBonus` | 0〜300 | 50.00 | 156 | 144.06 | 144 | −0.24 |
| `HistoryTtMoveBonus` | 0〜1,000 | 166.67 | 398 | 250.02 | 250 | −0.89 |
| `HistoryPriorBonus` | 0〜600 | 100.00 | 523 | 523.43 | 523 | +0.00 |
| `HistoryEvalDiff` | 0〜40 | 6.67 | 0 | 1.37 | 1 | +0.21 |
| `HistoryDecay` | 0〜100 | 16.67 | 13 | 5.91 | 6 | −0.43 |
| `CaptureHistoryDecay` | 0〜100 | 16.67 | 72 | 79.15 | 79 | +0.43 |
| `CaptureHistoryScale` | 0〜2,000 | 333.33 | 975 | 1,404.08 | 1,404 | +1.29 |
| `OrderingCheckBonus` | 0〜32,768 | 5461.33 | 14,922 | 14,023.91 | 14,024 | −0.16 |
| `OrderingThreatWeight` | 0〜12,000 | 2000.00 | 2,756 | 3,367.41 | 3,367 | +0.31 |
| `CorrectionMaterialWeight` | 0〜500 | 83.33 | 5 | 1.00 | 1 | −0.05 |
| `CorrectionPawnWeight` | 0〜200 | 33.33 | 58 | 35.71 | 36 | −0.67 |
| `CorrectionMinorWeight` | 0〜200 | 33.33 | 171 | 160.30 | 160 | −0.32 |
| `CorrectionNonPawnWeight` | 0〜200 | 33.33 | 93 | 98.51 | 99 | +0.17 |
| `CorrectionContinuationWeight` | 0〜200 | 33.33 | 200 | 200.00 | 200（上限） | +0.00 |
| `CorrectionContinuation6Weight` | 0〜200 | 33.33 | 154 | 189.04 | 189 | +1.05 |
| `CorrectionBonus` | 4〜60 | 9.33 | 45 | 52.69 | 53 | +0.82 |
| `CorrectionFailLowBonus` | 6〜90 | 14.00 | 62 | 65.14 | 65 | +0.22 |
| `DeltaMargin` | 50〜500 | 75.00 | 132 | 77.67 | 78 | −0.72 |
| `QsearchSeeMargin` | 0〜200 | 33.33 | 66 | 83.34 | 83 | +0.52 |
| `QsearchMoveLimit` | 1〜16 | 2.50 | 7 | 7.83 | 8 | +0.33 |
| `ExpectedPlies` | 250〜700 | 75.00 | 438 | 435.83 | 436 | −0.03 |
| `MinMoves` | 40〜200 | 26.67 | 40 | 42.48 | 42 | +0.09 |
| `IncrementShare` | 30〜100 | 11.67 | 99 | 98.87 | 99 | −0.01 |
| `HardSoftRatio` | 150〜800 | 108.33 | 571 | 755.31 | 755 | +1.70 |
| `HardRemainingShare` | 10〜50 | 6.67 | 47 | 49.83 | 50（上限） | +0.42 |
| `IterationRatio` | 150〜400 | 41.67 | 203 | 152.14 | 152 | −1.22 |
| `StableIterations` | 2〜8 | 1.00 | 4 | 5.08 | 5 | +1.08 |
| `TimeFallingBase` | 0〜1,500 | 250.00 | 115 | 32.71 | 33 | −0.33 |
| `TimeFallingPrevious` | 0〜8,000 | 1333.33 | 4,784 | 2,140.63 | 2,141 | −1.98 |
| `TimeFallingIteration` | 0〜5,000 | 833.33 | 2,288 | 1,351.35 | 1,351 | −1.12 |
| `TimeReductionIntercept` | 0〜1,500 | 250.00 | 254 | 461.60 | 462 | +0.83 |
| `TimeReductionSlope` | 0〜300 | 50.00 | 78 | 83.55 | 84 | +0.11 |
| `TimeReductionScale` | 1,000〜4,000 | 500.00 | 2,284 | 2,650.55 | 2,651 | +0.73 |
| `TimeInstabilityBase` | 500〜1,500 | 166.67 | 1,077 | 1,254.57 | 1,255 | +1.07 |
| `TimeInstabilitySlope` | 0〜4,000 | 666.67 | 2,229 | 803.42 | 803 | −2.14 |
| `TimeEffortIntercept` | 500〜2,500 | 333.33 | 1,642 | 1,752.15 | 1,752 | +0.33 |
| `TimeEffortSlope` | -2,000〜500 | 416.67 | -888 | -788.04 | -788 | +0.24 |
| `TimeAdvantage` | 0〜1,000 | 166.67 | 900 | 897.36 | 897 | −0.02 |

範囲の端に達した係数は、`LmrMoveCount`（下限）、`HistoryMalusDecay`（上限）、`CorrectionContinuationWeight`（上限）、および`HardRemainingShare`（上限）の4つである。

設計書が加えた12係数について、最終値をStockfishの値（開始値）、YaneuraOuの値、および起点の値と比べると次のとおりである。
換算値の求め方と、対応する値がない係数（「—」）の扱いは、設計書の係数の表のとおりである。起点の値は、起点の形がその項を持たない場合に、その項を働かせない値（加算の項なら0）とした。距離はいずれも`c_end`を単位とする。

| 係数 | Stockfishの換算値（開始値） | YaneuraOuの換算値 | 起点の値 | 最終値 | 開始値からの移動量 | YaneuraOuとの距離 | 起点との距離 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `TimeFallingBase` | 115 | 113 | — | 33 | −0.33 | −0.32 | — |
| `TimeFallingPrevious` | 4,784 | 1,904 | 0 | 2,141 | −1.98 | +0.18 | +1.61 |
| `TimeFallingIteration` | 2,288 | 888 | 0 | 1,351 | −1.12 | +0.56 | +1.62 |
| `TimeReductionIntercept` | 254 | — | — | 462 | +0.83 | — | — |
| `TimeReductionSlope` | 78 | — | 0 | 84 | +0.11 | — | +1.67 |
| `TimeReductionScale` | 2,284 | 2,238 | — | 2,651 | +0.73 | +0.83 | — |
| `TimeInstabilityBase` | 1,077 | 1,040 | 1,000 | 1,255 | +1.07 | +1.29 | +1.53 |
| `TimeInstabilitySlope` | 2,229 | 1,896 | 0 | 803 | −2.14 | −1.64 | +1.21 |
| `TimeEffortIntercept` | 1,642 | — | — | 1,752 | +0.33 | — | — |
| `TimeEffortSlope` | -888 | — | 0 | -788 | +0.24 | — | −1.89 |
| `TimeAdvantage` | 900 | — | 0 | 897 | −0.02 | — | +5.38 |
| `StableIterations` | 4 | — | 4 | 5 | +1.08 | — | +1.08 |

評価値の下落に反応する2係数は大きく下がった。前の探索の評価値からの下落に掛かる`TimeFallingPrevious`は4,784から2,141へ下がり、YaneuraOuの換算値（約1,904）に近づいた。直前の反復の評価値からの下落に掛かる`TimeFallingIteration`は2,288から1,351へ下がり、Stockfishの換算値とYaneuraOuの換算値（約888）の間にある。
最善手の変化回数に掛かる`TimeInstabilitySlope`は2,229から803へ下がり、移動量は103係数の中で最も大きい（`c_end`の2.14倍）。一方で切片の`TimeInstabilityBase`は1,077から1,255へ上がったので、最善手が変わらない局面でも目安の時間を延ばし、変化回数への反応は弱める形になった。
最善手の安定に関わる3係数では、`TimeReductionIntercept`が254から462へ、`TimeReductionScale`が2,284から2,651へ上がった。
二値の短縮の反復数`StableIterations`は4から5へ上がった。
相手との残り時間の差による補正の`TimeAdvantage`（900から897）と、最善手の部分木の割合に掛かる2係数（`TimeEffortIntercept`と`TimeEffortSlope`）は、開始値からあまり動かなかった。

既存の時間管理の係数では、softに対するhardの比`HardSoftRatio`が571から755へ上がり、hardの残り時間の割合`HardRemainingShare`は47から上限の50へ上がった。次の反復の完了時刻の予測に使う倍率`IterationRatio`は203から152へ下がった。
時間管理の係数のほかにも、大きく動いた係数がある。LMRでは、ttPvかつ置換表の深さが足りるcutNodeの減算量`LmrTtPvDepthCutNode`が140から193へ、置換表の手がないcutNodeの加算量`LmrCutNodeNoTtMove`が143から190へ上がった。null moveでは、静的評価とβの差から求める追加減深の倍率`NullMoveEvalScale`が180から283へ上がり、試すかを決める静的評価の余裕値の切片`NullMoveEvalMargin`が326から−50へ下がった。ほかに、手の整列キーへ加える捕獲履歴の換算値の絶対値の上限`CaptureHistoryScale`が975から1,404へ、reverse futility pruningの余裕値の定数`ReverseFutilityMultBase`が93から181へ上がった。
material補正の重み`CorrectionMaterialWeight`は5から1へ下がった。

## 結論

103係数の30,000ペアのSPSAは、異常0件で完了した。
候補の値は、ブランチ`time-instability-scaling`のコミット`1930dcb2`で係数の表の既定値へ書き込み、事前対局とSTCにかける。
