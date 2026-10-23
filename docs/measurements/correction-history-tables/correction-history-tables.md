# correction historyの表の変更の候補のSPSA

## 目的

[correction historyの表の追加と持ち越しの設計書](../../plans/archived/correction-history-tables.md)のフェーズ3として、形の変更の最後のコミット`e695176a`の89係数をSPSAで30,000ペア調整し、事前対局とSTCにかける候補の値を得る。
SPSAの対局結果は強さの根拠にしない。
最終値は、この予算と設定で得た候補の値であり、最適値とは呼ばない。

## コマンドライン

[統括計画](../../plans/archived/search-forms.md)の調整機で、`scripts/vast-spsa/remote.py`が[時間切れの事前確認](correction-history-tables-safety.md)を通した後に次を実行した。

```console
target/release/minase spsa --run-dir /workspace/job/correction-history-tables --seed 4150000000 \
  --engine commit:e695176a58cd33c1c01a054dfc3899820803601a --params /workspace/job/params \
  --rules engine-default --each time=12900+129 --concurrency 191 \
  --iterations 3750 --pairs-per-iteration 8
```

パラメーターファイルは`data/spsa/correction-history-tables.params`（89行）であり、内容は次の表の範囲、`c_end`、および開始値のとおりである。
`r_end`はすべて0.002であり、利得の指数はα = 0.602とγ = 0.101、学習率の安定化の定数はA = 375である。
持ち時間の`time=12900+129`は、測定機のSTC`time=10000+100`の基本時間と加算に、換算係数`f` = 1.29を掛けた値である。換算係数の求め方は[事前確認の記録](correction-history-tables-safety.md)にある。
実行ディレクトリは、`scripts/vast-spsa/fetch.py --keep`が10分ごとに測定機の`data/vast-spsa/correction-history-tables/`へ取得し、全反復がそろった時点で`minase spsa apply`による記録の検査に通した。終了後に`data/spsa/correction-history-tables`へ移した。
2026年10月20日19時6分（日本時間）ごろから、vast.aiのAPIが口座の操作に2要素認証でのログインを求め、要求を拒否するようになったので、`fetch.py`は調整機の状態の照会に6回続けて失敗して終了した。調整機は止まらずにセッションを続けた。利用者が認証し直すまでの約50分は、コンテナのSSHを通したrsyncで実行ディレクトリとログを取得し、19時57分に`fetch.py`を起動し直した。

## エンジン

調整対象は`e695176a58cd33c1c01a054dfc3899820803601a`の調整用ビルドであり、調整機でビルドした実行ファイルのSHA-256は`2a500f1128562f374f6717a196c2f08ac40accca58279842bf06b63f400f97df`である。
runnerは、調整機で同じコミットをビルドした通常のminase（2.4.0）であり、そのSHA-256は`d3e7ea8c9c22d304616af42cbaccbe5527ee67ecf8fc3954b5af7b695cbada2a`である。
`invocations.json`の起動の記録は1件であり、セッションは中断なく1回の起動で終えた。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiの出品56499914で借りたAMD EPYC 9K84の2基構成（ベトナム、機械のID 51088、インスタンス56856484）である。[サブタスク3のSPSA](../history-forms/history-forms.md)と同じ機械である。
物理192コア、論理384スレッドで、`cpu_ram`は773,514 MB、時間単価は約1.87米ドルである。
runnerが記録したCPUの数は、物理192コア、論理368スレッドである。
`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は191である。
測定機（Intel Core Ultra 7 265KF）の負荷時の速度は476,382 NPS、調整機の191本の負荷時の速度は367,951 NPSであり、`f`は1.29である。
開始時に見込んだ所要時間は8.15時間だった。
調整機を作り直して再開したことはない。

調整機は、照会した出品のうち条件に合う最安のものを`launch.py`がその場で作成する方式で借りた。作成は2026年10月20日16時33分（日本時間）である。
1時間2.5米ドル以下の96コアのCPUの出品が現れるまで、10時19分から約6.2時間待った。
セッションは2026年10月20日17時ごろに始まり、10月21日0時15分ごろに終わった。

## 結果

3,750反復をすべて適用し、有効ペア29,568、手数上限による破棄ペア432であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は26,118秒（約7.3時間）である。

θ（調整中の係数の中心値の組）の開始値と最終値は次のとおりである。
最終値（整数）は`minase spsa apply`が丸めた値であり、範囲の端に達した係数には「上限」または「下限」と付記した。
移動量は、開始値から最終値（実数）までの変化を`c_end`で割った値である。

| 係数 | 範囲 | `c_end` | 開始値 | 最終値（実数） | 最終値（整数） | 移動量 |
|---|---|---:|---:|---:|---:|---:|
| `LmrDivisor` | 100〜400 | 50.00 | 169 | 194.92 | 195 | +0.52 |
| `LmrHistoryThreshold` | 0〜2,000 | 333.33 | 39 | 711.47 | 711 | +2.02 |
| `LmrHistoryScale` | 0〜1,000 | 166.67 | 687 | 612.01 | 612 | −0.45 |
| `LmrMaxReduction` | 1〜8 | 1.17 | 4 | 4.27 | 4 | +0.23 |
| `LmrBase` | 0〜200 | 33.33 | 35 | 8.27 | 8 | −0.80 |
| `LmrOffset` | 0〜200 | 33.33 | 195 | 196.04 | 196 | +0.03 |
| `LmrNonImproving` | 0〜100 | 16.67 | 22 | 43.90 | 44 | +1.31 |
| `LmrPvReduction` | 0〜200 | 33.33 | 68 | 148.30 | 148 | +2.41 |
| `LmrTtPv` | 0〜200 | 33.33 | 9 | 36.54 | 37 | +0.83 |
| `LmrTtPvBase` | 0〜500 | 83.33 | 350 | 195.16 | 195 | −1.86 |
| `LmrTtPvPv` | 0〜200 | 33.33 | 19 | 11.68 | 12 | −0.22 |
| `LmrTtPvValue` | 0〜200 | 33.33 | 162 | 130.82 | 131 | −0.94 |
| `LmrTtPvDepth` | 0〜200 | 33.33 | 80 | 73.91 | 74 | −0.18 |
| `LmrTtPvDepthCutNode` | 0〜200 | 33.33 | 149 | 187.72 | 188 | +1.16 |
| `LmrMoveCount` | 0〜20 | 3.33 | 0 | 0.71 | 1 | +0.21 |
| `LmrCorrection` | 0〜200 | 33.33 | 101 | 67.81 | 68 | −1.00 |
| `LmrCutNode` | 0〜600 | 100.00 | 216 | 57.20 | 57 | −1.59 |
| `LmrCutNodeNoTtMove` | 0〜200 | 33.33 | 79 | 61.54 | 62 | −0.52 |
| `LmrTtCapture` | 0〜250 | 41.67 | 22 | 11.53 | 12 | −0.25 |
| `LmrCutoffCount` | 0〜100 | 16.67 | 66 | 34.54 | 35 | −1.89 |
| `LmrCutoffCountMany` | 0〜250 | 41.67 | 242 | 233.14 | 233 | −0.21 |
| `LmrCutoffCountAllNode` | 0〜250 | 41.67 | 95 | 139.84 | 140 | +1.08 |
| `LmrTtMove` | 0〜500 | 83.33 | 373 | 469.31 | 469 | +1.16 |
| `LmrAlphaEval` | 0〜150 | 25.00 | 7 | 13.57 | 14 | +0.26 |
| `LmrNoTtMove` | 0〜250 | 41.67 | 105 | 96.47 | 96 | −0.20 |
| `LmrAllNode` | 0〜200 | 33.33 | 184 | 168.84 | 169 | −0.45 |
| `LmrDeeperMargin` | 0〜100 | 16.67 | 100 | 89.22 | 89 | −0.65 |
| `LmrShallowerMargin` | 0〜40 | 6.67 | 40 | 35.85 | 36 | −0.62 |
| `ContinuationPruning` | 1,000〜12,000 | 1833.33 | 5,673 | 5,021.48 | 5,021 | −0.36 |
| `FutilityBase` | 0〜400 | 66.67 | 65 | 44.35 | 44 | −0.31 |
| `FutilityLmrDepth` | 0〜300 | 50.00 | 100 | 76.45 | 76 | −0.47 |
| `FutilityAboveAlpha` | -100〜300 | 66.67 | 98 | -18.09 | -18 | −1.74 |
| `QuietSeeMargin` | 0〜100 | 16.67 | 42 | 48.61 | 49 | +0.40 |
| `LmpBase` | 0〜8,600 | 1433.33 | 2,565 | 2,696.73 | 2,697 | +0.09 |
| `LmpSlope` | 0〜3,600 | 600.00 | 27 | 3.60 | 4 | −0.04 |
| `LmpNonImproving` | 25〜150 | 20.83 | 27 | 25.21 | 25（下限） | −0.09 |
| `ReverseFutilityMultBase` | 0〜450 | 75.00 | 265 | 122.91 | 123 | −1.89 |
| `ReverseFutilityMultSlope` | 0〜12 | 2.00 | 5 | 7.11 | 7 | +1.06 |
| `ReverseFutilityNoTtHit` | 0〜50 | 8.33 | 0 | 0.81 | 1 | +0.10 |
| `ReverseFutilityImproving` | 0〜400 | 66.67 | 20 | 66.39 | 66 | +0.70 |
| `ReverseFutilityOpponentWorsening` | 0〜100 | 16.67 | 1 | 8.13 | 8 | +0.43 |
| `ReverseFutilityCorrection` | 0〜200 | 33.33 | 148 | 198.46 | 198 | +1.51 |
| `RazoringDepthMargin` | 0〜3,000 | 500.00 | 2,953 | 2,980.38 | 2,980 | +0.05 |
| `SeeDepthMargin` | 0〜400 | 66.67 | 251 | 233.89 | 234 | −0.26 |
| `SeeCaptureHistory` | -50〜200 | 41.67 | 190 | 176.51 | 177 | −0.32 |
| `CaptureFutilityBase` | 0〜600 | 100.00 | 569 | 427.85 | 428 | −1.41 |
| `CaptureFutilityLmrDepth` | 0〜500 | 83.33 | 405 | 424.41 | 424 | +0.23 |
| `CaptureFutilityHistory` | -100〜600 | 116.67 | 412 | 499.21 | 499 | +0.75 |
| `AspirationDelta` | 10〜200 | 31.67 | 79 | 105.11 | 105 | +0.82 |
| `AspirationGrowth` | 125〜400 | 45.83 | 196 | 148.28 | 148 | −1.04 |
| `NullMoveBase` | 1,200〜10,000 | 1466.67 | 9,900 | 9,236.40 | 9,236 | −0.45 |
| `NullMoveSlope` | 100〜600 | 83.33 | 482 | 495.32 | 495 | +0.16 |
| `NullMoveEvalScale` | 0〜400 | 66.67 | 235 | 198.26 | 198 | −0.55 |
| `NullMoveEvalMargin` | -1,000〜1,000 | 333.33 | -991 | -552.96 | -553 | +1.31 |
| `NullMoveEvalDepth` | -50〜100 | 25.00 | 4 | 22.54 | 23 | +0.74 |
| `NullMoveEvalImproving` | -200〜400 | 100.00 | 234 | 374.28 | 374 | +1.40 |
| `NullMoveFailHighBonus` | 0〜200 | 33.33 | 23 | 48.61 | 49 | +0.77 |
| `HistoryBonusSlope` | 0〜400 | 66.67 | 233 | 260.64 | 261 | +0.41 |
| `HistoryBonusBase` | -100〜300 | 66.67 | 4 | 110.71 | 111 | +1.60 |
| `HistoryBonusCap` | 200〜7,183 | 1163.83 | 1,621 | 2,112.16 | 2,112 | +0.42 |
| `HistoryMalusSlope` | 0〜2,500 | 416.67 | 1,237 | 1,074.07 | 1,074 | −0.39 |
| `HistoryMalusBase` | -200〜800 | 166.67 | -36 | -188.87 | -189 | −0.92 |
| `HistoryMalusCap` | 200〜7,183 | 1163.83 | 3,404 | 3,360.92 | 3,361 | −0.04 |
| `HistoryMalusDecay` | 700〜1,024 | 54.00 | 1,024 | 1,019.96 | 1,020 | −0.07 |
| `HistoryTtCutoffBonus` | 0〜300 | 50.00 | 181 | 138.94 | 139 | −0.84 |
| `HistoryTtMoveBonus` | 0〜1,000 | 166.67 | 397 | 468.00 | 468 | +0.43 |
| `HistoryPriorBonus` | 0〜600 | 100.00 | 328 | 542.65 | 543 | +2.15 |
| `HistoryEvalDiff` | 0〜40 | 6.67 | 0 | 0.34 | 0（下限） | +0.05 |
| `HistoryDecay` | 0〜100 | 16.67 | 38 | 6.98 | 7 | −1.86 |
| `CaptureHistoryDecay` | 0〜100 | 16.67 | 55 | 67.83 | 68 | +0.77 |
| `CaptureHistoryScale` | 0〜2,000 | 333.33 | 1,064 | 1,289.58 | 1,290 | +0.68 |
| `OrderingCheckBonus` | 0〜32,768 | 5461.33 | 12,297 | 10,178.28 | 10,178 | −0.39 |
| `OrderingThreatWeight` | 0〜12,000 | 2000.00 | 3,020 | 2,493.98 | 2,494 | −0.26 |
| `CorrectionMaterialWeight` | 0〜500 | 83.33 | 277 | 153.65 | 154 | −1.48 |
| `CorrectionPawnWeight` | 0〜200 | 33.33 | 52 | 35.79 | 36 | −0.49 |
| `CorrectionMinorWeight` | 0〜200 | 33.33 | 36 | 82.36 | 82 | +1.39 |
| `CorrectionNonPawnWeight` | 0〜200 | 33.33 | 44 | 84.25 | 84 | +1.21 |
| `CorrectionContinuationWeight` | 0〜200 | 33.33 | 30 | 103.80 | 104 | +2.21 |
| `CorrectionContinuation6Weight` | 0〜200 | 33.33 | 24 | 111.49 | 111 | +2.62 |
| `CorrectionBonus` | 4〜60 | 9.33 | 20 | 22.62 | 23 | +0.28 |
| `CorrectionFailLowBonus` | 6〜90 | 14.00 | 29 | 21.13 | 21 | −0.56 |
| `DeltaMargin` | 50〜500 | 75.00 | 500 | 499.10 | 499 | −0.01 |
| `QsearchMoveLimit` | 1〜7 | 1.00 | 6 | 6.16 | 6 | +0.16 |
| `ExpectedPlies` | 250〜700 | 75.00 | 453 | 429.31 | 429 | −0.32 |
| `MinMoves` | 40〜200 | 26.67 | 42 | 41.44 | 41 | −0.02 |
| `IncrementShare` | 30〜100 | 11.67 | 93 | 96.54 | 97 | +0.30 |
| `HardSoftRatio` | 150〜800 | 108.33 | 666 | 670.00 | 670 | +0.04 |
| `HardRemainingShare` | 10〜50 | 6.67 | 46 | 47.43 | 47 | +0.21 |
| `IterationRatio` | 150〜400 | 41.67 | 237 | 256.81 | 257 | +0.48 |

最終値が範囲の端に達した係数は、`LmpNonImproving`（下限）と`HistoryEvalDiff`（下限）の2個である。
89係数のうち、開始値からの移動量の絶対値が1を超えたのは25個である。

新しい9係数について、最終値をStockfishの値（開始値）、YaneuraOuの値、および起点の値と比べると次のとおりである。
起点の値は、起点で対応する値（材料表の重みは起点の`CorrectionCap`、表や項がない係数は0）であり、距離はいずれも`c_end`を単位とする。

| 係数 | Stockfishの換算値（開始値） | YaneuraOuの換算値 | 起点の値 | 最終値 | 開始値からの移動量 | YaneuraOuとの距離 | 起点との距離 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `CorrectionMaterialWeight` | 277 | — | 277 | 154 | −1.48 | — | −1.48 |
| `CorrectionPawnWeight` | 52 | 105 | 0 | 36 | −0.49 | −2.08 | +1.07 |
| `CorrectionMinorWeight` | 36 | 75 | 0 | 82 | +1.39 | +0.22 | +2.47 |
| `CorrectionNonPawnWeight` | 44 | 107 | 0 | 84 | +1.21 | −0.68 | +2.53 |
| `CorrectionContinuationWeight` | 30 | 69 | 0 | 104 | +2.21 | +1.04 | +3.11 |
| `CorrectionContinuation6Weight` | 24 | — | 0 | 111 | +2.62 | — | +3.34 |
| `CorrectionBonus` | 20 | 8 | — | 23 | +0.28 | +1.57 | — |
| `CorrectionFailLowBonus` | 29 | 12 | — | 21 | −0.56 | +0.65 | — |
| `LmrCorrection` | 101 | 45 | 0 | 68 | −1.00 | +0.68 | +2.03 |

材料表の重み`CorrectionMaterialWeight`は277から154へ下がり、Stockfishにある配置の表（minor、non-pawn）の重みとcontinuation correction historyの重みはいずれも上がった。
continuation correction historyの2手前と4手前の重みは30から104へ、6手前の重みは24から111へ上がり、YaneuraOuの換算値（69）も上回った。
pawnの表の重み`CorrectionPawnWeight`は52から36へ下がった。
LMRの補正項の係数`LmrCorrection`は101から68へ下がり、YaneuraOuの換算値（45）とStockfishの換算値の間に入った。
reverse futility pruningの余裕値の補正量の項`ReverseFutilityCorrection`は148から198へ上がり、上限の200の近くに達した。
LMRの3段階のhistory補正の閾値`LmrHistoryThreshold`は39から711へ上がり、補正が働く手を大きく減らす値になった。

## 結論

89係数の30,000ペアのSPSAは、異常0件で完了した。
候補の値は、ブランチ`correction-history-tables`のコミット`5ed505ec`で係数の表の既定値へ書き込み、事前対局とSTCにかける。
