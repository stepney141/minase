# aspiration windowsの形の変更の候補のSPSA

## 目的

[aspiration windowsの幅と窓の移し方の見直しの設計書](../../plans/archived/aspiration-window-forms.md)のフェーズ3として、形の変更の最後のコミット`7631da34`の91係数をSPSAで30,000ペア調整し、事前対局とSTCにかける候補の値を得る。
SPSAの対局結果は強さの根拠にしない。
最終値は、この予算と設定で得た候補の値であり、最適値とは呼ばない。

## コマンドライン

[統括計画](../../plans/archived/search-forms.md)の調整機で、`scripts/vast-spsa/remote.py`が[時間切れの事前確認](aspiration-window-forms-safety.md)を通した後に次を実行した。

```console
target/release/minase spsa --run-dir /workspace/job/aspiration-window-forms --seed 4750000000 \
  --engine commit:7631da34e28b906ed876cfa6c228c7838a924682 --params /workspace/job/params \
  --rules engine-default --each time=13700+137 --concurrency 191 \
  --iterations 3750 --pairs-per-iteration 8
```

パラメーターファイルは`data/spsa/aspiration-window-forms.params`（91行）であり、内容は次の表の範囲、`c_end`、および開始値のとおりである。
`r_end`はすべて0.002であり、利得の指数はα = 0.602とγ = 0.101、学習率の安定化の定数はA = 375である。
持ち時間の`time=13700+137`は、測定機のSTC`time=10000+100`の基本時間と加算に、換算係数`f` = 1.37を掛けた値である。換算係数の求め方は[事前確認の記録](aspiration-window-forms-safety.md)にある。
実行ディレクトリは、`scripts/vast-spsa/fetch.py --keep`が10分ごとに測定機の`data/vast-spsa/aspiration-window-forms/`へ取得し、全反復がそろった時点で`minase spsa apply`による記録の検査に通した。終了後に`data/spsa/aspiration-window-forms`へ移した。

## エンジン

調整対象は`7631da34e28b906ed876cfa6c228c7838a924682`の調整用ビルドであり、調整機でビルドした実行ファイルのSHA-256は`aeea5725e5a4f5e4f40d6ed109f393964ed080f48ae6f7fdcb2f2468362e2d7d`である。
runnerは、調整機で同じコミットをビルドした通常のminase（2.4.0）であり、そのSHA-256は`ffaf3f22b51a209ae2972aafea8deaa5ded34e0de86e9cb88ef53de75267531f`である。
`invocations.json`の起動の記録は1件であり、セッションは中断なく1回の起動で終えた。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiの出品57006540で借りたAMD EPYC 9K84の2基構成（ベトナム、機械のID 49339、インスタンス57080553）である。
物理192コア、論理384スレッドで、`cpu_ram`は773,540 MB、時間単価は約1.88米ドルである。
runnerが記録したCPUの数は、物理192コア、論理368スレッドである。
`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は191である。
測定機（Intel Core Ultra 7 265KF）の負荷時の速度は467,260 NPS、調整機の191本の負荷時の速度は340,521 NPSであり、`f`は1.37である。
開始時に見込んだ所要時間は8.66時間だった。
調整機を作り直して再開したことはない。

調整機は、`cpu_ram`が260,000 MB以上で1時間2.1米ドル以下の出品を測定機で照会し、見つかった出品を`launch.py`に出品のIDで渡して作成した。作成は2026年10月22日1時17分（日本時間）である。
セッションは2026年10月22日1時47分ごろに始まり、9時41分ごろに終わった。

## 結果

3,750反復をすべて適用し、有効ペア29,576、手数上限による破棄ペア424であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は28,439秒（約7.9時間）である。

θ（調整中の係数の中心値の組）の開始値と最終値は次のとおりである。
最終値（整数）は`minase spsa apply`が丸めた値であり、範囲の端に達した係数には「上限」または「下限」と付記した。
移動量は、開始値から最終値（実数）までの変化を`c_end`で割った値である。

| 係数 | 範囲 | `c_end` | 開始値 | 最終値（実数） | 最終値（整数） | 移動量 |
|---|---|---:|---:|---:|---:|---:|
| `LmrDivisor` | 100〜400 | 50.00 | 205 | 187.79 | 188 | −0.34 |
| `LmrHistoryThreshold` | 0〜2,000 | 333.33 | 616 | 874.45 | 874 | +0.78 |
| `LmrHistoryScale` | 0〜1,000 | 166.67 | 24 | 39.46 | 39 | +0.09 |
| `LmrMaxReduction` | 1〜8 | 1.17 | 6 | 5.70 | 6 | −0.26 |
| `LmrBase` | 0〜200 | 33.33 | 37 | 15.52 | 16 | −0.64 |
| `LmrOffset` | 0〜200 | 33.33 | 186 | 188.01 | 188 | +0.06 |
| `LmrNonImproving` | 0〜100 | 16.67 | 41 | 16.55 | 17 | −1.47 |
| `LmrPvReduction` | 0〜200 | 33.33 | 129 | 116.76 | 117 | −0.37 |
| `LmrTtPv` | 0〜200 | 33.33 | 61 | 5.49 | 5 | −1.67 |
| `LmrTtPvBase` | 0〜500 | 83.33 | 37 | 47.14 | 47 | +0.12 |
| `LmrTtPvPv` | 0〜200 | 33.33 | 1 | 3.73 | 4 | +0.08 |
| `LmrTtPvValue` | 0〜200 | 33.33 | 137 | 116.07 | 116 | −0.63 |
| `LmrTtPvDepth` | 0〜200 | 33.33 | 44 | 40.94 | 41 | −0.09 |
| `LmrTtPvDepthCutNode` | 0〜200 | 33.33 | 140 | 139.53 | 140 | −0.01 |
| `LmrMoveCount` | 0〜20 | 3.33 | 1 | 0.81 | 1 | −0.06 |
| `LmrCorrection` | 0〜200 | 33.33 | 20 | 34.20 | 34 | +0.43 |
| `LmrCutNode` | 0〜600 | 100.00 | 435 | 430.03 | 430 | −0.05 |
| `LmrCutNodeNoTtMove` | 0〜200 | 33.33 | 116 | 143.33 | 143 | +0.82 |
| `LmrTtCapture` | 0〜250 | 41.67 | 9 | 44.92 | 45 | +0.86 |
| `LmrCutoffCount` | 0〜100 | 16.67 | 43 | 45.04 | 45 | +0.12 |
| `LmrCutoffCountMany` | 0〜250 | 41.67 | 224 | 213.88 | 214 | −0.24 |
| `LmrCutoffCountAllNode` | 0〜250 | 41.67 | 118 | 213.29 | 213 | +2.29 |
| `LmrTtMove` | 0〜500 | 83.33 | 470 | 467.36 | 467 | −0.03 |
| `LmrAlphaEval` | 0〜150 | 25.00 | 13 | 23.00 | 23 | +0.40 |
| `LmrNoTtMove` | 0〜250 | 41.67 | 98 | 107.70 | 108 | +0.23 |
| `LmrAllNode` | 0〜200 | 33.33 | 197 | 135.28 | 135 | −1.85 |
| `LmrDeeperMargin` | 0〜100 | 16.67 | 88 | 92.82 | 93 | +0.29 |
| `LmrShallowerMargin` | 0〜40 | 6.67 | 39 | 36.66 | 37 | −0.35 |
| `ContinuationPruning` | 1,000〜12,000 | 1833.33 | 3,160 | 3,317.71 | 3,318 | +0.09 |
| `FutilityBase` | 0〜400 | 66.67 | 68 | 89.83 | 90 | +0.33 |
| `FutilityLmrDepth` | 0〜300 | 50.00 | 95 | 101.32 | 101 | +0.13 |
| `FutilityAboveAlpha` | -100〜300 | 66.67 | 71 | 112.17 | 112 | +0.62 |
| `QuietSeeMargin` | 0〜100 | 16.67 | 51 | 51.78 | 52 | +0.05 |
| `LmpBase` | 0〜8,600 | 1433.33 | 3,713 | 3,837.54 | 3,838 | +0.09 |
| `LmpSlope` | 0〜3,600 | 600.00 | 0 | 42.05 | 42 | +0.07 |
| `LmpNonImproving` | 25〜150 | 20.83 | 26 | 25.80 | 26 | −0.01 |
| `ReverseFutilityMultBase` | 0〜450 | 75.00 | 170 | 92.57 | 93 | −1.03 |
| `ReverseFutilityMultSlope` | 0〜12 | 2.00 | 9 | 9.48 | 9 | +0.24 |
| `ReverseFutilityNoTtHit` | 0〜50 | 8.33 | 2 | 6.07 | 6 | +0.49 |
| `ReverseFutilityImproving` | 0〜400 | 66.67 | 0 | 2.57 | 3 | +0.04 |
| `ReverseFutilityOpponentWorsening` | 0〜100 | 16.67 | 4 | 0.60 | 1 | −0.20 |
| `ReverseFutilityCorrection` | 0〜200 | 33.33 | 199 | 198.55 | 199 | −0.01 |
| `RazoringDepthMargin` | 0〜3,000 | 500.00 | 2,354 | 1,355.35 | 1,355 | −2.00 |
| `SeeDepthMargin` | 0〜400 | 66.67 | 328 | 362.00 | 362 | +0.51 |
| `SeeCaptureHistory` | -50〜200 | 41.67 | 178 | 197.82 | 198 | +0.48 |
| `CaptureFutilityBase` | 0〜600 | 100.00 | 357 | 336.81 | 337 | −0.20 |
| `CaptureFutilityLmrDepth` | 0〜500 | 83.33 | 427 | 451.19 | 451 | +0.29 |
| `CaptureFutilityHistory` | -100〜600 | 116.67 | 368 | 390.37 | 390 | +0.19 |
| `AspirationDelta` | 1〜200 | 33.17 | 2 | 99.78 | 100 | +2.95 |
| `AspirationScoreWidth` | 0〜400 | 66.67 | 204 | 63.16 | 63 | −2.11 |
| `AspirationGrowth` | 125〜400 | 45.83 | 137 | 145.79 | 146 | +0.19 |
| `NullMoveBase` | 1,200〜10,000 | 1466.67 | 9,822 | 9,720.02 | 9,720 | −0.07 |
| `NullMoveSlope` | 100〜600 | 83.33 | 449 | 508.03 | 508 | +0.71 |
| `NullMoveEvalScale` | 0〜400 | 66.67 | 135 | 180.36 | 180 | +0.68 |
| `NullMoveEvalMargin` | -1,000〜1,000 | 333.33 | -71 | 326.00 | 326 | +1.19 |
| `NullMoveEvalDepth` | -50〜100 | 25.00 | -5 | 62.37 | 62 | +2.69 |
| `NullMoveEvalImproving` | -200〜400 | 100.00 | 377 | 368.18 | 368 | −0.09 |
| `NullMoveFailHighBonus` | 0〜200 | 33.33 | 44 | 47.86 | 48 | +0.12 |
| `HistoryBonusSlope` | 0〜400 | 66.67 | 346 | 394.36 | 394 | +0.73 |
| `HistoryBonusBase` | -100〜300 | 66.67 | 121 | 20.98 | 21 | −1.50 |
| `HistoryBonusCap` | 200〜7,183 | 1163.83 | 2,082 | 2,980.85 | 2,981 | +0.77 |
| `HistoryMalusSlope` | 0〜2,500 | 416.67 | 613 | 749.60 | 750 | +0.33 |
| `HistoryMalusBase` | -200〜800 | 166.67 | -170 | -195.66 | -196 | −0.15 |
| `HistoryMalusCap` | 200〜7,183 | 1163.83 | 1,733 | 1,983.37 | 1,983 | +0.22 |
| `HistoryMalusDecay` | 700〜1,024 | 54.00 | 1,024 | 1,021.41 | 1,021 | −0.05 |
| `HistoryTtCutoffBonus` | 0〜300 | 50.00 | 135 | 156.28 | 156 | +0.43 |
| `HistoryTtMoveBonus` | 0〜1,000 | 166.67 | 579 | 398.16 | 398 | −1.09 |
| `HistoryPriorBonus` | 0〜600 | 100.00 | 507 | 522.85 | 523 | +0.16 |
| `HistoryEvalDiff` | 0〜40 | 6.67 | 0 | 0.07 | 0（下限） | +0.01 |
| `HistoryDecay` | 0〜100 | 16.67 | 1 | 13.18 | 13 | +0.73 |
| `CaptureHistoryDecay` | 0〜100 | 16.67 | 70 | 72.16 | 72 | +0.13 |
| `CaptureHistoryScale` | 0〜2,000 | 333.33 | 992 | 974.88 | 975 | −0.05 |
| `OrderingCheckBonus` | 0〜32,768 | 5461.33 | 13,718 | 14,921.59 | 14,922 | +0.22 |
| `OrderingThreatWeight` | 0〜12,000 | 2000.00 | 3,180 | 2,756.13 | 2,756 | −0.21 |
| `CorrectionMaterialWeight` | 0〜500 | 83.33 | 6 | 5.34 | 5 | −0.01 |
| `CorrectionPawnWeight` | 0〜200 | 33.33 | 66 | 57.66 | 58 | −0.25 |
| `CorrectionMinorWeight` | 0〜200 | 33.33 | 139 | 170.71 | 171 | +0.95 |
| `CorrectionNonPawnWeight` | 0〜200 | 33.33 | 104 | 93.35 | 93 | −0.32 |
| `CorrectionContinuationWeight` | 0〜200 | 33.33 | 200 | 199.86 | 200（上限） | −0.00 |
| `CorrectionContinuation6Weight` | 0〜200 | 33.33 | 117 | 154.41 | 154 | +1.12 |
| `CorrectionBonus` | 4〜60 | 9.33 | 45 | 44.80 | 45 | −0.02 |
| `CorrectionFailLowBonus` | 6〜90 | 14.00 | 53 | 61.61 | 62 | +0.62 |
| `DeltaMargin` | 50〜500 | 75.00 | 131 | 132.39 | 132 | +0.02 |
| `QsearchSeeMargin` | 0〜200 | 33.33 | 42 | 65.52 | 66 | +0.71 |
| `QsearchMoveLimit` | 1〜16 | 2.50 | 8 | 6.85 | 7 | −0.46 |
| `ExpectedPlies` | 250〜700 | 75.00 | 418 | 438.08 | 438 | +0.27 |
| `MinMoves` | 40〜200 | 26.67 | 43 | 40.05 | 40（下限） | −0.11 |
| `IncrementShare` | 30〜100 | 11.67 | 95 | 98.93 | 99 | +0.34 |
| `HardSoftRatio` | 150〜800 | 108.33 | 594 | 570.80 | 571 | −0.21 |
| `HardRemainingShare` | 10〜50 | 6.67 | 50 | 47.04 | 47 | −0.44 |
| `IterationRatio` | 150〜400 | 41.67 | 228 | 203.44 | 203 | −0.59 |

範囲の端に達した係数は、`HistoryEvalDiff`（下限）、`CorrectionContinuationWeight`（上限）、および`MinMoves`（下限）の3つである。

本書が加えた係数と開始値を置き直した係数について、最終値をStockfishの値（開始値）、YaneuraOuの値、および起点の値と比べると次のとおりである。
起点の値は、設計書を始めた時点のmasterの既定値であり、`AspirationScoreWidth`の起点の値0は、根の値の大きさの項がない起点の形に当たる。YaneuraOuの`AspirationDelta`は約5.6%を6とした。距離はいずれも`c_end`を単位とする。

| 係数 | Stockfishの換算値（開始値） | YaneuraOuの換算値 | 起点の値 | 最終値 | 開始値からの移動量 | YaneuraOuとの距離 | 起点との距離 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `AspirationScoreWidth` | 204 | 100 | 0 | 63 | −2.11 | −0.55 | +0.95 |
| `AspirationDelta` | 2 | 6 | 97 | 100 | +2.95 | +2.83 | +0.08 |
| `AspirationGrowth` | 137 | 133 | 149 | 146 | +0.19 | +0.28 | −0.07 |

初期の半幅の定数項`AspirationDelta`は、Stockfishの換算値の2%から100%へ上がり、移動量は`c_end`の2.95倍だった。最終値は起点の97%とほぼ同じである。
根の値の大きさの項`AspirationScoreWidth`は204%から63%へ下がり、Stockfishの換算値とYaneuraOuの換算値（約100%）より小さくなった。
拡大率`AspirationGrowth`は137%から146%へわずかに上がり、起点の149%に近い。
窓の中心を平均に置き、窓外れで両端を移す形の下でも、初期の半幅は、Stockfishの狭い定数項ではなく、起点と同じ程度の広い定数項が選ばれた。

本書が加えた係数のほかにも、大きく動いた係数がある。
null moveを試す静的評価の余裕値では、切片`NullMoveEvalMargin`が−71から326へ、残り深さ1手ごとに引く量`NullMoveEvalDepth`が−5から62へ上がった。
LMRでは、子のβ打ち切り数が1より多いallNodeの追加量`LmrCutoffCountAllNode`が118から213へ上がり、allNodeで減深量に掛けて足す割合`LmrAllNode`は197から135へ、ttPvの前半の加算量`LmrTtPv`は61から5へ、良化していない場合の表の値の増分`LmrNonImproving`は41から17へ下がった。
razoringの余裕値の残り深さ1あたりの量`RazoringDepthMargin`は2,354から1,355へ下がった。
時間管理では、`IterationRatio`が228から203へ、`HardSoftRatio`が594から571へ、`HardRemainingShare`が50から47へ下がり、`MinMoves`は43から下限の40へ下がった。

## 結論

91係数の30,000ペアのSPSAは、異常0件で完了した。
候補の値は、ブランチ`aspiration-window-forms`のコミット`46900904`で係数の表の既定値へ書き込み、事前対局とSTCにかける。
