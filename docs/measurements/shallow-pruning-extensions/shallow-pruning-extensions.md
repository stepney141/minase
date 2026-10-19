# 浅い前向き枝刈りの拡張の候補のSPSA

## 目的

[浅い前向き枝刈りの深さと対象の拡張](../../plans/archived/shallow-pruning-extensions.md)のフェーズ3として、形の変更の最後のコミット`364c10aa`の71係数をSPSAで30,000ペア調整し、事前対局とSTCにかける候補の値を得る。
SPSAの対局結果は強さの根拠にしない。
最終値は、この予算と設定で得た候補の値であり、最適値とは呼ばない。

## コマンドライン

[統括計画](../../plans/search-forms.md)の調整機で、`scripts/vast-spsa/remote.py`が[時間切れの事前確認](shallow-pruning-extensions-safety.md)を通した後に次を実行した。

```console
target/release/minase spsa --run-dir /workspace/job/shallow-pruning-extensions --seed 3550000000 \
  --engine commit:364c10aac8b32a6fe3953e4a1be06e8db638ac79 --params /workspace/job/params \
  --rules engine-default --each time=15700+157 --concurrency 191 \
  --iterations 3750 --pairs-per-iteration 8
```

パラメーターファイルは`data/spsa/shallow-pruning-extensions.params`（71行）であり、内容は次の表の範囲、`c_end`、および開始値のとおりである。
`r_end`はすべて0.002であり、利得の指数はα = 0.602とγ = 0.101、学習率の安定化の定数はA = 375である。
持ち時間の`time=15700+157`は、測定機のSTC`time=10000+100`の基本時間と加算に、換算係数`f` = 1.57を掛けた値である。換算係数の求め方は[事前確認の記録](shallow-pruning-extensions-safety.md)にある。
実行ディレクトリは、`scripts/vast-spsa/fetch.py`が10分ごとに測定機の`data/vast-spsa/shallow-pruning-extensions/`へ取得し、全反復がそろった時点で`minase spsa apply`による記録の検査に通した。終了後に`data/spsa/shallow-pruning-extensions`へ写した。

## エンジン

調整対象は`364c10aac8b32a6fe3953e4a1be06e8db638ac79`の調整用ビルドであり、調整機でビルドした実行ファイルのSHA-256は`ee84473384ea7d9f0ad3864ee60e1cc4fb96e5f208664abece4dedc46741f43a`である。
runnerは、調整機で同じコミットをビルドした通常のminase（2.4.0）であり、そのSHA-256は`d70c957afebd29773a32c23fb1849fdd1f7ab484bc0a94e04bed2c8f7dd4e3f0`である。
`invocations.json`の起動の記録は1件であり、セッションは中断なく1回の起動で終えた。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

調整機は、vast.aiの出品28164421で借りたAMD EPYC 9654の2基構成（スペイン、機械のID 13577、インスタンス56567047）である。
物理192コア、論理384スレッドで、`cpu_ram`は644,487 MB、時間単価は2.01米ドルである。
runnerが記録したCPUの数は、物理192コア、論理368スレッドである。
`Threads`は1、`USI_Hash`は既定の256 MB、同時対局数は191である。
測定機（Intel Core Ultra 7 265KF）の負荷時の速度は776,638 NPS、調整機の191本の負荷時の速度は495,019 NPSであり、`f`は1.57である。
開始時に見込んだ所要時間は9.92時間だった。
調整機を作り直して再開したことはない。

調整機は、照会した出品のうち条件に合う最安のものを`launch.py`がその場で作成する方式で、2026年10月18日23時28分（日本時間）に作成した。
同じ型の安い出品は、照会してから出品IDを渡して作成するまでの間に、ほかの利用者に借りられて2度消えていた。
セッションは2026年10月18日23時59分に始まり、10月19日8時30分に終わった。
作成から終了までの約9.0時間の費用は、約18.2米ドル（1時間2.01米ドル）である。

## 結果

3,750反復をすべて適用し、有効ペア29,701、手数上限による破棄ペア299であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は30,630秒（約8.5時間）である。

θの開始値と最終値は次のとおりである。
最終値（整数）は`minase spsa apply`が丸めた値であり、範囲の端に達した係数には「上限」または「下限」と付記した。
移動量は、開始値から最終値（実数）までの変化を`c_end`で割った値である。

| 係数 | 範囲 | `c_end` | 開始値 | 最終値（実数） | 最終値（整数） | 移動量 |
|---|---|---:|---:|---:|---:|---:|
| `LmrDivisor` | 100〜400 | 50.00 | 147 | 100.00 | 100（下限） | −0.94 |
| `LmrHistoryThreshold` | 0〜512 | 85.33 | 35 | 8.74 | 9 | −0.31 |
| `LmrMaxReduction` | 1〜8 | 1.17 | 3 | 2.90 | 3 | −0.09 |
| `LmrBase` | 0〜200 | 33.33 | 127 | 65.34 | 65 | −1.85 |
| `LmrOffset` | 0〜200 | 33.33 | 106 | 136.64 | 137 | +0.92 |
| `LmrNonImproving` | 0〜100 | 16.67 | 3 | 6.50 | 6 | +0.21 |
| `LmrPvReduction` | 0〜200 | 33.33 | 6 | 1.54 | 2 | −0.13 |
| `LmrTtPv` | 0〜200 | 33.33 | 70 | 38.54 | 39 | −0.94 |
| `LmrTtPvBase` | 0〜500 | 83.33 | 337 | 310.46 | 310 | −0.32 |
| `LmrTtPvPv` | 0〜200 | 33.33 | 13 | 28.88 | 29 | +0.48 |
| `LmrTtPvValue` | 0〜200 | 33.33 | 95 | 120.52 | 121 | +0.77 |
| `LmrTtPvDepth` | 0〜200 | 33.33 | 77 | 47.18 | 47 | −0.89 |
| `LmrTtPvDepthCutNode` | 0〜200 | 33.33 | 90 | 157.73 | 158 | +2.03 |
| `LmrMoveCount` | 0〜20 | 3.33 | 0 | 0.23 | 0（下限） | +0.07 |
| `LmrCutNode` | 0〜600 | 100.00 | 164 | 260.19 | 260 | +0.96 |
| `LmrCutNodeNoTtMove` | 0〜200 | 33.33 | 59 | 95.98 | 96 | +1.11 |
| `LmrTtCapture` | 0〜250 | 41.67 | 0 | 17.80 | 18 | +0.43 |
| `LmrCutoffCount` | 0〜100 | 16.67 | 48 | 78.30 | 78 | +1.82 |
| `LmrCutoffCountMany` | 0〜250 | 41.67 | 232 | 205.23 | 205 | −0.64 |
| `LmrCutoffCountAllNode` | 0〜250 | 41.67 | 129 | 123.23 | 123 | −0.14 |
| `LmrTtMove` | 0〜500 | 83.33 | 269 | 227.22 | 227 | −0.50 |
| `LmrAlphaEval` | 0〜150 | 25.00 | 46 | 9.55 | 10 | −1.46 |
| `LmrNoTtMove` | 0〜250 | 41.67 | 57 | 105.11 | 105 | +1.15 |
| `LmrAllNode` | 0〜200 | 33.33 | 142 | 157.98 | 158 | +0.48 |
| `LmrDeeperMargin` | 0〜100 | 16.67 | 82 | 91.73 | 92 | +0.58 |
| `LmrShallowerMargin` | 0〜40 | 6.67 | 26 | 27.00 | 27 | +0.15 |
| `FutilityBase` | 0〜400 | 66.67 | 79 | 105.71 | 106 | +0.40 |
| `FutilityLmrDepth` | 0〜300 | 50.00 | 57 | 218.60 | 219 | +3.23 |
| `FutilityAboveAlpha` | −100〜300 | 66.67 | 43 | 131.51 | 132 | +1.33 |
| `QuietSeeMargin` | 0〜100 | 16.67 | 11 | 47.80 | 48 | +2.21 |
| `LmpBase` | 0〜8,600 | 1433.33 | 1,799 | 6,435.82 | 6,436 | +3.23 |
| `LmpSlope` | 0〜3,600 | 600.00 | 48 | 346.29 | 346 | +0.50 |
| `LmpNonImproving` | 25〜150 | 20.83 | 31 | 25.58 | 26 | −0.26 |
| `ReverseFutilityMultBase` | 0〜450 | 75.00 | 235 | 269.27 | 269 | +0.46 |
| `ReverseFutilityMultSlope` | 0〜12 | 2.00 | 4 | 6.34 | 6 | +1.17 |
| `ReverseFutilityNoTtHit` | 0〜50 | 8.33 | 0 | 0.22 | 0（下限） | +0.03 |
| `ReverseFutilityImproving` | 0〜400 | 66.67 | 144 | 162.81 | 163 | +0.28 |
| `ReverseFutilityOpponentWorsening` | 0〜100 | 16.67 | 20 | 23.02 | 23 | +0.18 |
| `ReverseFutilityCorrection` | 0〜200 | 33.33 | 124 | 90.62 | 91 | −1.00 |
| `RazoringDepthMargin` | 0〜3,000 | 500.00 | 164 | 2,197.35 | 2,197 | +4.07 |
| `SeeDepthMargin` | 0〜400 | 66.67 | 85 | 167.94 | 168 | +1.24 |
| `SeeCaptureHistory` | −50〜200 | 41.67 | 34 | 121.87 | 122 | +2.11 |
| `CaptureFutilityBase` | 0〜600 | 100.00 | 113 | 503.92 | 504 | +3.91 |
| `CaptureFutilityLmrDepth` | 0〜500 | 83.33 | 119 | 147.53 | 148 | +0.34 |
| `CaptureFutilityHistory` | −100〜600 | 116.67 | 134 | 474.83 | 475 | +2.92 |
| `AspirationDelta` | 10〜200 | 31.67 | 57 | 79.90 | 80 | +0.72 |
| `AspirationGrowth` | 125〜400 | 45.83 | 168 | 198.04 | 198 | +0.66 |
| `NullMoveBase` | 1,200〜10,000 | 1466.67 | 7,200 | 9,985.21 | 9,985 | +1.90 |
| `NullMoveSlope` | 100〜600 | 83.33 | 400 | 473.92 | 474 | +0.89 |
| `NullMoveEvalScale` | 0〜400 | 66.67 | 163 | 238.39 | 238 | +1.13 |
| `NullMoveEvalMargin` | −1,000〜1,000 | 333.33 | 175 | −846.80 | −847 | −3.07 |
| `NullMoveEvalDepth` | −50〜100 | 25.00 | 6 | −10.67 | −11 | −0.67 |
| `NullMoveEvalImproving` | −200〜400 | 100.00 | 23 | 76.10 | 76 | +0.53 |
| `NullMoveFailHighBonus` | 0〜200 | 33.33 | 24 | 23.84 | 24 | −0.00 |
| `HistoryLimit` | 4,096〜65,536 | 10240.00 | 5,808 | 4,384.43 | 4,384 | −0.14 |
| `HistoryBonusScale` | 0〜400 | 66.67 | 207 | 336.00 | 336 | +1.94 |
| `HistoryMalusScale` | 0〜400 | 66.67 | 110 | 165.66 | 166 | +0.83 |
| `HistoryDecay` | 0〜100 | 16.67 | 37 | 31.86 | 32 | −0.31 |
| `CaptureHistoryDecay` | 0〜100 | 16.67 | 13 | 60.61 | 61 | +2.86 |
| `CaptureHistoryLimit` | 4,096〜65,536 | 10240.00 | 8,376 | 6,134.04 | 6,134 | −0.22 |
| `CaptureHistoryScale` | 0〜400 | 66.67 | 155 | 285.11 | 285 | +1.95 |
| `CorrectionCap` | 50〜400 | 58.33 | 181 | 125.51 | 126 | −0.95 |
| `CorrectionWeight` | 8〜128 | 20.00 | 76 | 81.97 | 82 | +0.30 |
| `DeltaMargin` | 50〜500 | 75.00 | 500 | 499.55 | 500（上限） | −0.01 |
| `QsearchMoveLimit` | 1〜7 | 1.00 | 6 | 5.90 | 6 | −0.10 |
| `ExpectedPlies` | 250〜700 | 75.00 | 458 | 451.19 | 451 | −0.09 |
| `MinMoves` | 40〜200 | 26.67 | 40 | 41.76 | 42 | +0.07 |
| `IncrementShare` | 30〜100 | 11.67 | 87 | 76.69 | 77 | −0.88 |
| `HardSoftRatio` | 150〜800 | 108.33 | 730 | 630.61 | 631 | −0.92 |
| `HardRemainingShare` | 10〜50 | 6.67 | 39 | 36.92 | 37 | −0.31 |
| `IterationRatio` | 150〜400 | 41.67 | 277 | 222.45 | 222 | −1.31 |

丸めた最終値が範囲の端に達した係数は4個である。
下限に達したのは`LmrDivisor`、`LmrMoveCount`、および`ReverseFutilityNoTtHit`、上限に達したのは`DeltaMargin`である。
`LmrMoveCount`、`ReverseFutilityNoTtHit`、および`DeltaMargin`は、[前の設計書のSPSA](../search-pruning-forms/search-pruning-forms.md)でも同じ端にあった。
`NullMoveBase`（上限10,000に対して9,985）と`NullMoveEvalMargin`（下限−1,000に対して−847）も端の近くにある。
71係数のうち、開始値からの移動量の絶対値が1を超えたのは23個である。

新しい14係数と開始値を置き直した3係数について、最終値が開始値（Stockfishの換算値）、YaneuraOuの換算値、および起点の値からどれだけ離れたかを、`c_end`に対する比で次に示す。
YaneuraOuの換算値と起点の値は、設計書の新しい係数の表の値である。対応する項がない場合は「—」とした。
`RazoringDepthMargin`は、YaneuraOuが深さの2乗に比例する形を、起点が深さ別の余裕値を持つので、両方とも深さ1の値と比べた。
この比は、中将棋で値がStockfishからどれだけ離れるかを読むための目安であり、採否には使わない。

| 係数 | Stockfishの換算値（開始値） | YaneuraOuの換算値 | 起点の値 | 最終値 | 開始値からの移動量 | YaneuraOuとの距離 | 起点との距離 |
|---|---:|---:|---:|---:|---:|---:|---:|
| `FutilityBase` | 79 | 47 | 317 | 106 | +0.40 | +0.88 | −3.17 |
| `FutilityLmrDepth` | 57 | 133 | — | 219 | +3.23 | +1.71 | — |
| `FutilityAboveAlpha` | 43 | 96 | 0 | 132 | +1.33 | +0.53 | +1.97 |
| `SeeDepthMargin` | 85 | 186 | — | 168 | +1.24 | −0.27 | — |
| `SeeCaptureHistory` | 34 | 79 | 0 | 122 | +2.11 | +1.03 | +2.92 |
| `CaptureFutilityBase` | 113 | 242 | — | 504 | +3.91 | +2.62 | — |
| `CaptureFutilityLmrDepth` | 119 | 248 | — | 148 | +0.34 | −1.21 | — |
| `CaptureFutilityHistory` | 134 | 303 | — | 475 | +2.92 | +1.47 | — |
| `QuietSeeMargin` | 11 | 28 | — | 48 | +2.21 | +1.19 | — |
| `RazoringDepthMargin` | 164 | 898 | 2,622 | 2,197 | +4.07 | +2.60 | −0.85 |
| `NullMoveEvalMargin` | 175 | 420 | — | −847 | −3.07 | −3.80 | — |
| `NullMoveEvalDepth` | 6 | 18 | — | −11 | −0.67 | −1.15 | — |
| `NullMoveEvalImproving` | 23 | 59 | — | 76 | +0.53 | +0.17 | — |
| `NullMoveFailHighBonus` | 24 | — | 0 | 24 | −0.00 | — | +0.72 |
| `NullMoveBase` | 7,200 | 7,200 | 4,799 | 9,985 | +1.90 | +1.90 | +3.54 |
| `NullMoveSlope` | 400 | 400 | 393 | 474 | +0.89 | +0.89 | +0.97 |
| `NullMoveEvalScale` | 163 | 0 | 133 | 238 | +1.13 | +3.58 | +1.58 |


17係数のうち、開始値からの移動量の絶対値が1を超えたのは11個である。
浅い前向き枝刈りの余裕値の10係数は、すべてStockfishの換算値より大きい側へ、つまり枝刈りを控える側へ動いた。
静かな手のfutility pruningのlmrDepthあたりの増分`FutilityLmrDepth`、捕獲手のfutility pruningの定数`CaptureFutilityBase`とcapture historyの重み`CaptureFutilityHistory`、静かな手のSEEの`QuietSeeMargin`は、いずれもYaneuraOuの換算値も超えた。
razoringの余裕値`RazoringDepthMargin`は開始値164から2,197へ上がり、起点の深さ1の値2,622に近く、razoringはほとんど働かない値に戻った。
late move pruningの上限の切片`LmpBase`も、開始値1,799から6,436へ上がった。
一方、null moveを試す条件の定数`NullMoveEvalMargin`は175から−847へ下がってnull moveを試すノードを増やす側へ動き、減深量の切片`NullMoveBase`は上限の近くまで上がった。

## 結論

71係数の30,000ペアのSPSAは、異常0件で完了した。
候補の値は、ブランチ`shallow-pruning-extensions`のコミット`29443569`で係数の表の既定値へ書き込み、事前対局とSTCにかける。
