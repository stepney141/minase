# 静止探索の補正と枝刈りの形の変更の候補のSPSA

## 目的

[静止探索の補正と枝刈りの形の見直しの設計書](../../plans/archived/qsearch-forms.md)のフェーズ3として、形の変更の最後のコミット`e9abbc86`の90係数をSPSAで30,000ペア調整し、事前対局とSTCにかける候補の値を得る。
SPSAの対局結果は強さの根拠にしない。
最終値は、この予算と設定で得た候補の値であり、最適値とは呼ばない。

## コマンドライン

[統括計画](../../plans/search-forms.md)の調整機で、`scripts/vast-spsa/remote.py`が[時間切れの事前確認](qsearch-forms-safety.md)を通した後に次を実行した。

```console
target/release/minase spsa --run-dir /workspace/job/qsearch-forms --seed 4450000000 \
  --engine commit:e9abbc864554983399e250a2b28fdcf3d304debc --params /workspace/job/params \
  --rules engine-default --each time=12600+126 --concurrency 191 \
  --iterations 3750 --pairs-per-iteration 8
```

パラメーターファイルは`data/spsa/qsearch-forms.params`（90行）であり、内容は次の表の範囲、`c_end`、および開始値のとおりである。
`r_end`はすべて0.002であり、利得の指数はα = 0.602とγ = 0.101、学習率の安定化の定数はA = 375である。
持ち時間の`time=12600+126`は、測定機のSTC`time=10000+100`の基本時間と加算に、換算係数`f` = 1.26を掛けた値である。換算係数の求め方は[事前確認の記録](qsearch-forms-safety.md)にある。
実行ディレクトリは、`scripts/vast-spsa/fetch.py --keep`が10分ごとに測定機の`data/vast-spsa/qsearch-forms/`へ取得し、全反復がそろった時点で`minase spsa apply`による記録の検査に通した。終了後に`data/spsa/qsearch-forms`へ移した。

## エンジン

調整対象は`e9abbc864554983399e250a2b28fdcf3d304debc`の調整用ビルドであり、調整機でビルドした実行ファイルのSHA-256は`c5189135c4f9e8a806c981978f58e41357717bac7a8e3ab41acffe384a47c518`である。
runnerは、調整機で同じコミットをビルドした通常のminase（2.4.0）であり、そのSHA-256は`1585bb71aa3b4174a7a5141002ecf2b998cd6350c4d3e3cf41fe5cbb8f2f0279`である。
`invocations.json`の起動の記録は1件であり、セッションは中断なく1回の起動で終えた。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiの出品28164421で借りたAMD EPYC 9654の2基構成（スペイン、機械のID 13577、インスタンス56989145）である。
物理192コア、論理384スレッドで、`cpu_ram`は644,487 MB、時間単価は約2.01米ドルである。
runnerが記録したCPUの数は、物理192コア、論理368スレッドである。
`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は191である。
測定機（Intel Core Ultra 7 265KF）の負荷時の速度は377,413 NPS、調整機の191本の負荷時の速度は298,454 NPSであり、`f`は1.26である。
開始時に見込んだ所要時間は7.96時間だった。
調整機を作り直して再開したことはない。

調整機は、照会した出品のうち条件に合う最安のものを`launch.py`がその場で作成する方式で借りた。作成は2026年10月21日12時08分（日本時間）である。
セッションは2026年10月21日12時36分ごろに始まり、19時50分ごろに終わった。

## 結果

3,750反復をすべて適用し、有効ペア29,574、手数上限による破棄ペア426であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は25,965秒（約7.2時間）である。

θ（調整中の係数の中心値の組）の開始値と最終値は次のとおりである。
最終値（整数）は`minase spsa apply`が丸めた値であり、範囲の端に達した係数には「上限」または「下限」と付記した。
移動量は、開始値から最終値（実数）までの変化を`c_end`で割った値である。

| 係数 | 範囲 | `c_end` | 開始値 | 最終値（実数） | 最終値（整数） | 移動量 |
|---|---|---:|---:|---:|---:|---:|
| `LmrDivisor` | 100〜400 | 50.00 | 195 | 205.12 | 205 | +0.20 |
| `LmrHistoryThreshold` | 0〜2,000 | 333.33 | 711 | 616.32 | 616 | −0.28 |
| `LmrHistoryScale` | 0〜1,000 | 166.67 | 612 | 24.30 | 24 | −3.53 |
| `LmrMaxReduction` | 1〜8 | 1.17 | 4 | 6.27 | 6 | +1.95 |
| `LmrBase` | 0〜200 | 33.33 | 8 | 36.54 | 37 | +0.86 |
| `LmrOffset` | 0〜200 | 33.33 | 196 | 185.51 | 186 | −0.31 |
| `LmrNonImproving` | 0〜100 | 16.67 | 44 | 40.50 | 41 | −0.21 |
| `LmrPvReduction` | 0〜200 | 33.33 | 148 | 129.29 | 129 | −0.56 |
| `LmrTtPv` | 0〜200 | 33.33 | 37 | 60.72 | 61 | +0.71 |
| `LmrTtPvBase` | 0〜500 | 83.33 | 195 | 36.61 | 37 | −1.90 |
| `LmrTtPvPv` | 0〜200 | 33.33 | 12 | 0.54 | 1 | −0.34 |
| `LmrTtPvValue` | 0〜200 | 33.33 | 131 | 137.10 | 137 | +0.18 |
| `LmrTtPvDepth` | 0〜200 | 33.33 | 74 | 44.44 | 44 | −0.89 |
| `LmrTtPvDepthCutNode` | 0〜200 | 33.33 | 188 | 140.32 | 140 | −1.43 |
| `LmrMoveCount` | 0〜20 | 3.33 | 1 | 0.76 | 1 | −0.07 |
| `LmrCorrection` | 0〜200 | 33.33 | 68 | 20.19 | 20 | −1.43 |
| `LmrCutNode` | 0〜600 | 100.00 | 57 | 434.62 | 435 | +3.78 |
| `LmrCutNodeNoTtMove` | 0〜200 | 33.33 | 62 | 115.65 | 116 | +1.61 |
| `LmrTtCapture` | 0〜250 | 41.67 | 12 | 8.71 | 9 | −0.08 |
| `LmrCutoffCount` | 0〜100 | 16.67 | 35 | 42.91 | 43 | +0.47 |
| `LmrCutoffCountMany` | 0〜250 | 41.67 | 233 | 224.20 | 224 | −0.21 |
| `LmrCutoffCountAllNode` | 0〜250 | 41.67 | 140 | 117.86 | 118 | −0.53 |
| `LmrTtMove` | 0〜500 | 83.33 | 469 | 469.85 | 470 | +0.01 |
| `LmrAlphaEval` | 0〜150 | 25.00 | 14 | 13.11 | 13 | −0.04 |
| `LmrNoTtMove` | 0〜250 | 41.67 | 96 | 97.79 | 98 | +0.04 |
| `LmrAllNode` | 0〜200 | 33.33 | 169 | 197.07 | 197 | +0.84 |
| `LmrDeeperMargin` | 0〜100 | 16.67 | 89 | 88.09 | 88 | −0.05 |
| `LmrShallowerMargin` | 0〜40 | 6.67 | 36 | 38.94 | 39 | +0.44 |
| `ContinuationPruning` | 1,000〜12,000 | 1833.33 | 5,021 | 3,160.20 | 3,160 | −1.01 |
| `FutilityBase` | 0〜400 | 66.67 | 44 | 68.47 | 68 | +0.37 |
| `FutilityLmrDepth` | 0〜300 | 50.00 | 76 | 95.29 | 95 | +0.39 |
| `FutilityAboveAlpha` | -100〜300 | 66.67 | -18 | 70.94 | 71 | +1.33 |
| `QuietSeeMargin` | 0〜100 | 16.67 | 49 | 50.86 | 51 | +0.11 |
| `LmpBase` | 0〜8,600 | 1433.33 | 2,697 | 3,713.25 | 3,713 | +0.71 |
| `LmpSlope` | 0〜3,600 | 600.00 | 4 | 0.00 | 0（下限） | −0.01 |
| `LmpNonImproving` | 25〜150 | 20.83 | 25 | 26.30 | 26 | +0.06 |
| `ReverseFutilityMultBase` | 0〜450 | 75.00 | 123 | 170.13 | 170 | +0.63 |
| `ReverseFutilityMultSlope` | 0〜12 | 2.00 | 7 | 9.02 | 9 | +1.01 |
| `ReverseFutilityNoTtHit` | 0〜50 | 8.33 | 1 | 1.72 | 2 | +0.09 |
| `ReverseFutilityImproving` | 0〜400 | 66.67 | 66 | 0.00 | 0（下限） | −0.99 |
| `ReverseFutilityOpponentWorsening` | 0〜100 | 16.67 | 8 | 3.51 | 4 | −0.27 |
| `ReverseFutilityCorrection` | 0〜200 | 33.33 | 198 | 198.84 | 199 | +0.03 |
| `RazoringDepthMargin` | 0〜3,000 | 500.00 | 2,980 | 2,353.99 | 2,354 | −1.25 |
| `SeeDepthMargin` | 0〜400 | 66.67 | 234 | 328.40 | 328 | +1.42 |
| `SeeCaptureHistory` | -50〜200 | 41.67 | 177 | 177.99 | 178 | +0.02 |
| `CaptureFutilityBase` | 0〜600 | 100.00 | 428 | 357.47 | 357 | −0.71 |
| `CaptureFutilityLmrDepth` | 0〜500 | 83.33 | 424 | 426.56 | 427 | +0.03 |
| `CaptureFutilityHistory` | -100〜600 | 116.67 | 499 | 367.54 | 368 | −1.13 |
| `AspirationDelta` | 10〜200 | 31.67 | 105 | 97.39 | 97 | −0.24 |
| `AspirationGrowth` | 125〜400 | 45.83 | 148 | 148.73 | 149 | +0.02 |
| `NullMoveBase` | 1,200〜10,000 | 1466.67 | 9,236 | 9,822.50 | 9,822 | +0.40 |
| `NullMoveSlope` | 100〜600 | 83.33 | 495 | 448.67 | 449 | −0.56 |
| `NullMoveEvalScale` | 0〜400 | 66.67 | 198 | 134.90 | 135 | −0.95 |
| `NullMoveEvalMargin` | -1,000〜1,000 | 333.33 | -553 | -70.56 | -71 | +1.45 |
| `NullMoveEvalDepth` | -50〜100 | 25.00 | 23 | -5.35 | -5 | −1.13 |
| `NullMoveEvalImproving` | -200〜400 | 100.00 | 374 | 377.46 | 377 | +0.03 |
| `NullMoveFailHighBonus` | 0〜200 | 33.33 | 49 | 43.68 | 44 | −0.16 |
| `HistoryBonusSlope` | 0〜400 | 66.67 | 261 | 345.52 | 346 | +1.27 |
| `HistoryBonusBase` | -100〜300 | 66.67 | 111 | 120.56 | 121 | +0.14 |
| `HistoryBonusCap` | 200〜7,183 | 1163.83 | 2,112 | 2,082.09 | 2,082 | −0.03 |
| `HistoryMalusSlope` | 0〜2,500 | 416.67 | 1,074 | 612.91 | 613 | −1.11 |
| `HistoryMalusBase` | -200〜800 | 166.67 | -189 | -170.24 | -170 | +0.11 |
| `HistoryMalusCap` | 200〜7,183 | 1163.83 | 3,361 | 1,732.96 | 1,733 | −1.40 |
| `HistoryMalusDecay` | 700〜1,024 | 54.00 | 1,020 | 1,023.78 | 1,024（上限） | +0.07 |
| `HistoryTtCutoffBonus` | 0〜300 | 50.00 | 139 | 134.65 | 135 | −0.09 |
| `HistoryTtMoveBonus` | 0〜1,000 | 166.67 | 468 | 579.00 | 579 | +0.67 |
| `HistoryPriorBonus` | 0〜600 | 100.00 | 543 | 507.14 | 507 | −0.36 |
| `HistoryEvalDiff` | 0〜40 | 6.67 | 0 | 0.23 | 0（下限） | +0.03 |
| `HistoryDecay` | 0〜100 | 16.67 | 7 | 0.61 | 1 | −0.38 |
| `CaptureHistoryDecay` | 0〜100 | 16.67 | 68 | 70.07 | 70 | +0.12 |
| `CaptureHistoryScale` | 0〜2,000 | 333.33 | 1,290 | 991.62 | 992 | −0.90 |
| `OrderingCheckBonus` | 0〜32,768 | 5461.33 | 10,178 | 13,717.84 | 13,718 | +0.65 |
| `OrderingThreatWeight` | 0〜12,000 | 2000.00 | 2,494 | 3,179.74 | 3,180 | +0.34 |
| `CorrectionMaterialWeight` | 0〜500 | 83.33 | 154 | 6.24 | 6 | −1.77 |
| `CorrectionPawnWeight` | 0〜200 | 33.33 | 36 | 66.37 | 66 | +0.91 |
| `CorrectionMinorWeight` | 0〜200 | 33.33 | 82 | 139.33 | 139 | +1.72 |
| `CorrectionNonPawnWeight` | 0〜200 | 33.33 | 84 | 103.97 | 104 | +0.60 |
| `CorrectionContinuationWeight` | 0〜200 | 33.33 | 104 | 200.00 | 200（上限） | +2.88 |
| `CorrectionContinuation6Weight` | 0〜200 | 33.33 | 111 | 117.48 | 117 | +0.19 |
| `CorrectionBonus` | 4〜60 | 9.33 | 23 | 45.29 | 45 | +2.39 |
| `CorrectionFailLowBonus` | 6〜90 | 14.00 | 21 | 53.49 | 53 | +2.32 |
| `DeltaMargin` | 50〜500 | 75.00 | 147 | 131.00 | 131 | −0.21 |
| `QsearchSeeMargin` | 0〜200 | 33.33 | 36 | 42.26 | 42 | +0.19 |
| `QsearchMoveLimit` | 1〜16 | 2.50 | 2 | 8.05 | 8 | +2.42 |
| `ExpectedPlies` | 250〜700 | 75.00 | 429 | 418.38 | 418 | −0.14 |
| `MinMoves` | 40〜200 | 26.67 | 41 | 43.39 | 43 | +0.09 |
| `IncrementShare` | 30〜100 | 11.67 | 97 | 95.43 | 95 | −0.13 |
| `HardSoftRatio` | 150〜800 | 108.33 | 670 | 593.59 | 594 | −0.71 |
| `HardRemainingShare` | 10〜50 | 6.67 | 47 | 49.96 | 50（上限） | +0.44 |
| `IterationRatio` | 150〜400 | 41.67 | 257 | 228.36 | 228 | −0.69 |

最終値を丸めた整数が範囲の端に達した係数は、`LmpSlope`（下限）、`ReverseFutilityImproving`（下限）、`HistoryMalusDecay`（上限）、`HistoryEvalDiff`（下限）、`CorrectionContinuationWeight`（上限）、および`HardRemainingShare`（上限）の6つである。
`DeltaMargin`は範囲の端に達しなかった。

本書が加えた係数と開始値を置き直した係数について、最終値をStockfishの値（開始値）、YaneuraOuの値、および起点の値と比べると次のとおりである。
起点の値は、設計書を始めた時点のmasterの既定値（静止探索のSEEの下限は起点にないので、損のない捕獲だけを読むことに当たる0）であり、距離はいずれも`c_end`を単位とする。
`QsearchMoveLimit`の起点の値6は、実際に読んだ捕獲手の数を数える起点の形での上限なので、候補の列の順番を数える本書の形の上限とは意味が違う。

| 係数 | Stockfishの換算値（開始値） | YaneuraOuの換算値 | 起点の値 | 最終値 | 開始値からの移動量 | YaneuraOuとの距離 | 起点との距離 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `QsearchSeeMargin` | 36 | 81 | 0 | 42 | +0.19 | −1.16 | +1.27 |
| `QsearchMoveLimit` | 2 | — | 6 | 8 | +2.42 | — | +0.82 |
| `DeltaMargin` | 147 | 364 | 499 | 131 | −0.21 | −3.11 | −4.91 |


`DeltaMargin`は147から131へ下がり、起点の499とYaneuraOuの364から離れる方向へ動いた。入口のdelta pruningをやめ、取り返し、成る手、および王手になる手を枝刈りから外した形では、余裕値を小さくして枝刈りを強める方向が選ばれた。
`QsearchMoveLimit`は2から8へ上がり、移動量は`c_end`の2.42倍だった。候補の列の順番で数えると、Stockfishの2より多くの候補を読む方向が選ばれた。
`QsearchSeeMargin`は36から42へわずかに上がり、Stockfishの換算値とYaneuraOuの換算値（81）の間にとどまった。

本書が加えた係数のほかにも、大きく動いた係数がある。
correction historyの材料表の重み`CorrectionMaterialWeight`は154から6へ下がって0の近くになり、continuation correction historyの重み`CorrectionContinuationWeight`は104から上限の200へ上がった。補正表の更新量`CorrectionBonus`と`CorrectionFailLowBonus`は、それぞれ23から45へ、21から53へ上がった。
LMRでは、cutNodeの項`LmrCutNode`が57から435へ上がり、3段階のhistory補正の倍率`LmrHistoryScale`は612から24へ下がった。LMRの減深量の上限`LmrMaxReduction`は4から6へ上がった。
静止探索の補正値は、[correction historyのADR](../../adr/sum-material-placement-and-continuation-correction-histories-and-apply-them-to-the-qsearch-stand-pat.md)のとおり通常探索と同じ合成値を使うので、静止探索に補正を加えたことと補正表の重みの大きな変化の関係は、この記録からは切り分けられない。

## 結論

90係数の30,000ペアのSPSAは、異常0件で完了した。
候補の値は、ブランチ`qsearch-forms`のコミット`c6cf0c72`で係数の表の既定値へ書き込み、事前対局とSTCにかける。
