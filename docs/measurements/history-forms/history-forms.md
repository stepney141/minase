# historyの形の変更の候補のSPSA

## 目的

[historyの更新量の形とcontinuation historyの設計書](../../plans/archived/history-forms.md)のフェーズ3として、形の変更の最後のコミット`b2cae818`の82係数をSPSAで30,000ペア調整し、事前対局とSTCにかける候補の値を得る。
SPSAの対局結果は強さの根拠にしない。
最終値は、この予算と設定で得た候補の値であり、最適値とは呼ばない。

## コマンドライン

[統括計画](../../plans/archived/search-forms.md)の調整機で、`scripts/vast-spsa/remote.py`が[時間切れの事前確認](history-forms-safety.md)を通した後に次を実行した。

```console
target/release/minase spsa --run-dir /workspace/job/history-forms --seed 3850000000 \
  --engine commit:b2cae818cc8c7b912a087017c9ae8781172fa909 --params /workspace/job/params \
  --rules engine-default --each time=14500+145 --concurrency 191 \
  --iterations 3750 --pairs-per-iteration 8
```

パラメーターファイルは`data/spsa/history-forms.params`（82行）であり、内容は次の表の範囲、`c_end`、および開始値のとおりである。
`r_end`はすべて0.002であり、利得の指数はα = 0.602とγ = 0.101、学習率の安定化の定数はA = 375である。
持ち時間の`time=14500+145`は、測定機のSTC`time=10000+100`の基本時間と加算に、換算係数`f` = 1.45を掛けた値である。換算係数の求め方は[事前確認の記録](history-forms-safety.md)にある。
実行ディレクトリは、`scripts/vast-spsa/fetch.py --keep`が10分ごとに測定機の`data/vast-spsa/history-forms/`へ取得し、全反復がそろった時点で`minase spsa apply`による記録の検査に通した。終了後に`data/spsa/history-forms`へ移した。

## エンジン

調整対象は`b2cae818cc8c7b912a087017c9ae8781172fa909`の調整用ビルドであり、調整機でビルドした実行ファイルのSHA-256は`8b5e020835f8ea683865e86a7a63e74034059ea8700043952db18188249f4dac`である。
runnerは、調整機で同じコミットをビルドした通常のminase（2.4.0）であり、そのSHA-256は`39fc5882bdaf6d53218926e1ed98591ed8b3870c7d7042ada5415519ffa5fa2f`である。
`invocations.json`の起動の記録は1件であり、セッションは中断なく1回の起動で終えた。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiの出品56499914で借りたAMD EPYC 9K84の2基構成（ベトナム、機械のID 51088、インスタンス56723270）である。
物理192コア、論理384スレッドで、`cpu_ram`は773,514 MB、時間単価は約1.87米ドルである。
runnerが記録したCPUの数は、物理192コア、論理368スレッドである。
`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は191である。
測定機（Intel Core Ultra 7 265KF）の負荷時の速度は542,019 NPS、調整機の191本の負荷時の速度は373,782 NPSであり、`f`は1.45である。
開始時に見込んだ所要時間は9.16時間だった。
調整機を作り直して再開したことはない。
この調整機の前に、CPUの割り当てが論理208個に制限された別の調整機を、見込みの所要時間が上限を超えたので削除した（[事前確認の記録](history-forms-safety.md)）。

調整機は、照会した出品のうち条件に合う最安のものを`launch.py`がその場で作成する方式で、2026年10月19日21時1分（日本時間）に作成した。
1時間2.5米ドル以下の96コアの型の出品が現れるまで、15時15分から約6時間待った。
セッションは2026年10月19日21時30分ごろに始まり、10月20日5時35分ごろに終わった。

## 結果

3,750反復をすべて適用し、有効ペア29,600、手数上限による破棄ペア400であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は29,085秒（約8.1時間）である。

θ（調整中の係数の中心値の組）の開始値と最終値は次のとおりである。
最終値（整数）は`minase spsa apply`が丸めた値であり、範囲の端に達した係数には「上限」または「下限」と付記した。
移動量は、開始値から最終値（実数）までの変化を`c_end`で割った値である。

| 係数 | 範囲 | `c_end` | 開始値 | 最終値（実数） | 最終値（整数） | 移動量 |
|---|---|---:|---:|---:|---:|---:|
| `LmrDivisor` | 100〜400 | 50.00 | 100 | 168.56 | 169 | +1.37 |
| `LmrHistoryThreshold` | 0〜2,000 | 333.33 | 15 | 39.01 | 39 | +0.07 |
| `LmrHistoryScale` | 0〜1,000 | 166.67 | 439 | 687.40 | 687 | +1.49 |
| `LmrMaxReduction` | 1〜8 | 1.17 | 3 | 3.92 | 4 | +0.79 |
| `LmrBase` | 0〜200 | 33.33 | 65 | 34.90 | 35 | −0.90 |
| `LmrOffset` | 0〜200 | 33.33 | 137 | 195.02 | 195 | +1.74 |
| `LmrNonImproving` | 0〜100 | 16.67 | 6 | 21.56 | 22 | +0.93 |
| `LmrPvReduction` | 0〜200 | 33.33 | 2 | 67.67 | 68 | +1.97 |
| `LmrTtPv` | 0〜200 | 33.33 | 39 | 9.27 | 9 | −0.89 |
| `LmrTtPvBase` | 0〜500 | 83.33 | 310 | 350.11 | 350 | +0.48 |
| `LmrTtPvPv` | 0〜200 | 33.33 | 29 | 18.76 | 19 | −0.31 |
| `LmrTtPvValue` | 0〜200 | 33.33 | 121 | 162.18 | 162 | +1.24 |
| `LmrTtPvDepth` | 0〜200 | 33.33 | 47 | 80.02 | 80 | +0.99 |
| `LmrTtPvDepthCutNode` | 0〜200 | 33.33 | 158 | 148.86 | 149 | −0.27 |
| `LmrMoveCount` | 0〜20 | 3.33 | 0 | 0.35 | 0（下限） | +0.10 |
| `LmrCutNode` | 0〜600 | 100.00 | 260 | 215.92 | 216 | −0.44 |
| `LmrCutNodeNoTtMove` | 0〜200 | 33.33 | 96 | 78.87 | 79 | −0.51 |
| `LmrTtCapture` | 0〜250 | 41.67 | 18 | 22.02 | 22 | +0.10 |
| `LmrCutoffCount` | 0〜100 | 16.67 | 78 | 66.15 | 66 | −0.71 |
| `LmrCutoffCountMany` | 0〜250 | 41.67 | 205 | 241.69 | 242 | +0.88 |
| `LmrCutoffCountAllNode` | 0〜250 | 41.67 | 123 | 94.69 | 95 | −0.68 |
| `LmrTtMove` | 0〜500 | 83.33 | 227 | 373.32 | 373 | +1.76 |
| `LmrAlphaEval` | 0〜150 | 25.00 | 10 | 6.77 | 7 | −0.13 |
| `LmrNoTtMove` | 0〜250 | 41.67 | 105 | 105.08 | 105 | +0.00 |
| `LmrAllNode` | 0〜200 | 33.33 | 158 | 183.98 | 184 | +0.78 |
| `LmrDeeperMargin` | 0〜100 | 16.67 | 92 | 99.56 | 100（上限） | +0.45 |
| `LmrShallowerMargin` | 0〜40 | 6.67 | 27 | 39.70 | 40（上限） | +1.91 |
| `ContinuationPruning` | 1,000〜12,000 | 1833.33 | 4,136 | 5,672.93 | 5,673 | +0.84 |
| `FutilityBase` | 0〜400 | 66.67 | 106 | 64.80 | 65 | −0.62 |
| `FutilityLmrDepth` | 0〜300 | 50.00 | 219 | 100.21 | 100 | −2.38 |
| `FutilityAboveAlpha` | −100〜300 | 66.67 | 132 | 98.20 | 98 | −0.51 |
| `QuietSeeMargin` | 0〜100 | 16.67 | 48 | 41.86 | 42 | −0.37 |
| `LmpBase` | 0〜8,600 | 1433.33 | 6,436 | 2,564.72 | 2,565 | −2.70 |
| `LmpSlope` | 0〜3,600 | 600.00 | 346 | 26.53 | 27 | −0.53 |
| `LmpNonImproving` | 25〜150 | 20.83 | 26 | 26.73 | 27 | +0.03 |
| `ReverseFutilityMultBase` | 0〜450 | 75.00 | 269 | 264.88 | 265 | −0.05 |
| `ReverseFutilityMultSlope` | 0〜12 | 2.00 | 6 | 5.24 | 5 | −0.38 |
| `ReverseFutilityNoTtHit` | 0〜50 | 8.33 | 0 | 0.10 | 0（下限） | +0.01 |
| `ReverseFutilityImproving` | 0〜400 | 66.67 | 163 | 19.98 | 20 | −2.15 |
| `ReverseFutilityOpponentWorsening` | 0〜100 | 16.67 | 23 | 0.67 | 1 | −1.34 |
| `ReverseFutilityCorrection` | 0〜200 | 33.33 | 91 | 148.20 | 148 | +1.72 |
| `RazoringDepthMargin` | 0〜3,000 | 500.00 | 2,197 | 2,952.54 | 2,953 | +1.51 |
| `SeeDepthMargin` | 0〜400 | 66.67 | 168 | 250.73 | 251 | +1.24 |
| `SeeCaptureHistory` | −50〜200 | 41.67 | 122 | 190.38 | 190 | +1.64 |
| `CaptureFutilityBase` | 0〜600 | 100.00 | 504 | 569.21 | 569 | +0.65 |
| `CaptureFutilityLmrDepth` | 0〜500 | 83.33 | 148 | 404.68 | 405 | +3.08 |
| `CaptureFutilityHistory` | −100〜600 | 116.67 | 475 | 411.87 | 412 | −0.54 |
| `AspirationDelta` | 10〜200 | 31.67 | 80 | 79.50 | 79 | −0.02 |
| `AspirationGrowth` | 125〜400 | 45.83 | 198 | 196.39 | 196 | −0.04 |
| `NullMoveBase` | 1,200〜10,000 | 1466.67 | 9,985 | 9,899.50 | 9,900 | −0.06 |
| `NullMoveSlope` | 100〜600 | 83.33 | 474 | 482.18 | 482 | +0.10 |
| `NullMoveEvalScale` | 0〜400 | 66.67 | 238 | 235.02 | 235 | −0.04 |
| `NullMoveEvalMargin` | −1,000〜1,000 | 333.33 | −847 | −991.34 | −991 | −0.43 |
| `NullMoveEvalDepth` | −50〜100 | 25.00 | −11 | 4.37 | 4 | +0.61 |
| `NullMoveEvalImproving` | −200〜400 | 100.00 | 76 | 233.61 | 234 | +1.58 |
| `NullMoveFailHighBonus` | 0〜200 | 33.33 | 24 | 22.98 | 23 | −0.03 |
| `HistoryBonusSlope` | 0〜400 | 66.67 | 133 | 232.69 | 233 | +1.50 |
| `HistoryBonusBase` | −100〜300 | 66.67 | 81 | 3.81 | 4 | −1.16 |
| `HistoryBonusCap` | 200〜7,183 | 1163.83 | 1,487 | 1,620.54 | 1,621 | +0.11 |
| `HistoryMalusSlope` | 0〜2,500 | 416.67 | 968 | 1,237.50 | 1,237 | +0.65 |
| `HistoryMalusBase` | −200〜800 | 166.67 | 235 | −36.24 | −36 | −1.63 |
| `HistoryMalusCap` | 200〜7,183 | 1163.83 | 2,244 | 3,403.92 | 3,404 | +1.00 |
| `HistoryMalusDecay` | 700〜1,024 | 54.00 | 921 | 1,023.68 | 1,024（上限） | +1.90 |
| `HistoryTtCutoffBonus` | 0〜300 | 50.00 | 131 | 181.44 | 181 | +1.01 |
| `HistoryTtMoveBonus` | 0〜1,000 | 166.67 | 364 | 397.44 | 397 | +0.20 |
| `HistoryPriorBonus` | 0〜600 | 100.00 | 215 | 328.26 | 328 | +1.13 |
| `HistoryEvalDiff` | 0〜40 | 6.67 | 11 | 0.00 | 0（下限） | −1.65 |
| `HistoryDecay` | 0〜100 | 16.67 | 71 | 37.83 | 38 | −1.99 |
| `CaptureHistoryDecay` | 0〜100 | 16.67 | 61 | 54.58 | 55 | −0.39 |
| `CaptureHistoryScale` | 0〜2,000 | 333.33 | 734 | 1,064.50 | 1,064 | +0.99 |
| `OrderingCheckBonus` | 0〜32,768 | 5461.33 | 16,384 | 12,297.10 | 12,297 | −0.75 |
| `OrderingThreatWeight` | 0〜12,000 | 2000.00 | 4,160 | 3,020.49 | 3,020 | −0.57 |
| `CorrectionCap` | 50〜400 | 58.33 | 126 | 277.09 | 277 | +2.59 |
| `CorrectionWeight` | 8〜128 | 20.00 | 82 | 70.52 | 71 | −0.57 |
| `DeltaMargin` | 50〜500 | 75.00 | 500 | 499.55 | 500（上限） | −0.01 |
| `QsearchMoveLimit` | 1〜7 | 1.00 | 6 | 6.25 | 6 | +0.25 |
| `ExpectedPlies` | 250〜700 | 75.00 | 451 | 452.71 | 453 | +0.02 |
| `MinMoves` | 40〜200 | 26.67 | 42 | 41.82 | 42 | −0.01 |
| `IncrementShare` | 30〜100 | 11.67 | 77 | 93.20 | 93 | +1.39 |
| `HardSoftRatio` | 150〜800 | 108.33 | 631 | 665.68 | 666 | +0.32 |
| `HardRemainingShare` | 10〜50 | 6.67 | 37 | 45.99 | 46 | +1.35 |
| `IterationRatio` | 150〜400 | 41.67 | 222 | 237.07 | 237 | +0.36 |

最終値が範囲の端に達した係数は、`LmrMoveCount`（下限）、`LmrDeeperMargin`（上限）、`LmrShallowerMargin`（上限）、`ReverseFutilityNoTtHit`（下限）、`HistoryMalusDecay`（上限）、`HistoryEvalDiff`（下限）、および`DeltaMargin`（上限）の7個である。

新しい15係数と開始値を置き直した3係数について、最終値をStockfishの値（開始値）、YaneuraOuの値、および起点の値と比べると次のとおりである。
起点の値は、新しい係数では仕組みがない起点の状態に当たる値（設計書の「新しい係数」の表の「現行」）であり、距離はいずれも`c_end`を単位とする。

| 係数 | Stockfishの換算値（開始値） | YaneuraOuの換算値 | 起点の値 | 最終値 | 開始値からの移動量 | YaneuraOuとの距離 | 起点との距離 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `HistoryBonusSlope` | 133 | 128 | 0 | 233 | +1.50 | +1.57 | +3.49 |
| `HistoryBonusBase` | 81 | 77 | 0 | 4 | −1.16 | −1.10 | +0.06 |
| `HistoryBonusCap` | 1,487 | 1,529 | 7,183 | 1,621 | +0.11 | +0.08 | −4.78 |
| `HistoryMalusSlope` | 968 | 882 | 0 | 1,237 | +0.65 | +0.85 | +2.97 |
| `HistoryMalusBase` | 235 | 204 | 0 | −36 | −1.63 | −1.44 | −0.22 |
| `HistoryMalusCap` | 2,244 | 2,122 | 7,183 | 3,404 | +1.00 | +1.10 | −3.25 |
| `HistoryMalusDecay` | 921 | 977 | 1,024 | 1,024 | +1.90 | +0.86 | −0.01 |
| `HistoryTtCutoffBonus` | 131 | 130 | 0 | 181 | +1.01 | +1.03 | +3.63 |
| `HistoryTtMoveBonus` | 364 | 353 | 0 | 397 | +0.20 | +0.27 | +2.38 |
| `HistoryPriorBonus` | 215 | 235 | 0 | 328 | +1.13 | +0.93 | +3.28 |
| `HistoryEvalDiff` | 11 | 10 | 0 | 0 | −1.65 | −1.50 | +0.00 |
| `ContinuationPruning` | 4,136 | 4,097 | — | 5,673 | +0.84 | +0.86 | — |
| `LmrHistoryScale` | 439 | 428 | 0 | 687 | +1.49 | +1.56 | +4.12 |
| `OrderingCheckBonus` | 16,384 | 16,384 | 0 | 12,297 | −0.75 | −0.75 | +2.25 |
| `OrderingThreatWeight` | 4,160 | — | 0 | 3,020 | −0.57 | — | +1.51 |
| `HistoryDecay` | 71 | 100 | 32 | 38 | −1.99 | −3.73 | +0.35 |
| `CaptureHistoryScale` | 734 | 1,697 | 285 | 1,064 | +0.99 | −1.90 | +2.34 |
| `LmrHistoryThreshold` | 15 | — | 9 | 39 | +0.07 | — | +0.09 |

18係数のうち、開始値からの移動量の絶対値が1を超えたのは9個である。
減点の逓減`HistoryMalusDecay`は上限の1024に達し、探索済みの静かな手を逓減させずに同じ量で減点する値になった。
静的評価の変化によるbutterfly historyの更新の倍率`HistoryEvalDiff`は下限の0に達し、この更新は働かない値になった。
加点と減点の深さ1あたりの増分（`HistoryBonusSlope`、`HistoryMalusSlope`）は上がり、切片（`HistoryBonusBase`、`HistoryMalusBase`）は下がって、浅いノードでも更新量が大きくなる側へ動いた。
LMRのstatScoreの比例係数`LmrHistoryScale`は439から687へ上がった。
continuation historyによる枝刈りの閾値`ContinuationPruning`は4,136から5,673へ上がり、枝刈りを控える側へ動いた。
根の探索ごとの減衰`HistoryDecay`は71から38へ下がり、起点の32に近い値に戻った。

## 結論

82係数の30,000ペアのSPSAは、異常0件で完了した。
候補の値は、ブランチ`history-forms`のコミット`b06c8c9a`で係数の表の既定値へ書き込み、事前対局とSTCにかける。
