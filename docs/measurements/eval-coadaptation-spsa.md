# eval-coadaptation-spsa

## 目的

[共適応の検証](../plans/eval-search-coadaptation.md)のフェーズ3として、混合比λ=1.0のPSTを載せた候補Cλについて、探索係数29個をSPSAで調整し、候補Cλ\*の値を得る。
SPSAの対局結果は採否の根拠にしない。

## コマンドライン

```console
data/spsa/eval-coadaptation-runner/spsa_runner --run-dir data/spsa/eval-coadaptation-clambda --seed 14000000 \
  --engine commit:5f8f4256283a94cb99e45fc94be86bbd9610a653 \
  --params data/spsa/eval-coadaptation-params/session.txt \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 375 --pairs-per-iteration 8
```

パラメーターファイルは、`spsa_runner params`がCλの調整用ビルドから生成した既定のファイルから時間管理の6係数を除き、13係数の`c_end`を設計書の「摂動幅」の規則で上書きしたものである。
規則は、範囲の1/6と、開始値から近い方の範囲の端までの距離を375^0.101（約1.8196）で割った値の小さい方を、小数第1位へ丸める。
この規則を[探索部の再調整のパラメーターファイル](search-revival-t.md)の開始値に当てはめると、同ファイルの13係数の`c_end`がすべて再現されることを確かめた。
`r_end`はすべて既定の0.002、利得の指数はα = 0.602、γ = 0.101、A = 37.5である。
同じファイルを、条件付きの対照S0\*のセッションにも使う。
ファイルの全行（名前、開始値、最小、最大、`c_end`、`r_end`）は次のとおりである。

```text
LmrDivisor, 156, 100, 400, 50, 0.002
LmrHistoryThreshold, 146, 0, 512, 85.33333333333333, 0.002
FutilityMargin1, 165, 0, 400, 66.66666666666667, 0.002
FutilityMargin2, 239, 0, 400, 66.66666666666667, 0.002
FutilityMargin3, 283, 0, 400, 66.66666666666667, 0.002
NonImprovingFutility1, 71, 0, 100, 15.9, 0.002
NonImprovingFutility2, 72, 0, 100, 15.4, 0.002
NonImprovingFutility3, 86, 0, 100, 7.7, 0.002
LmpBase, 382, 0, 8600, 209.9, 0.002
LmpSlope, 133, 0, 3600, 73.1, 0.002
ReverseFutilityMargin, 428, 0, 1938, 235.2, 0.002
RazoringMargin1, 1606, 0, 2668, 444.7, 0.002
RazoringMargin2, 2015, 0, 2885, 478.1, 0.002
SeeMargin1, 10, 0, 400, 66.66666666666667, 0.002
SeeMargin2, 192, 0, 400, 66.66666666666667, 0.002
SeeMargin3, 13, 0, 400, 66.66666666666667, 0.002
AspirationDelta, 55, 10, 200, 31.666666666666668, 0.002
AspirationGrowth, 190, 125, 400, 45.833333333333336, 0.002
NullMoveBase, 3617, 1200, 4800, 600, 0.002
NullMoveSlope, 257, 100, 400, 50, 0.002
NullMoveEvalScale, 56, 0, 400, 30.8, 0.002
HistoryLimit, 17408, 4096, 65536, 10240, 0.002
HistoryDecay, 23, 0, 100, 12.6, 0.002
CaptureHistoryLimit, 16950, 4096, 65536, 7064.2, 0.002
CaptureHistoryScale, 49, 0, 400, 26.9, 0.002
CorrectionCap, 220, 50, 400, 58.333333333333336, 0.002
CorrectionWeight, 37, 8, 128, 20, 0.002
DeltaMargin, 355, 50, 500, 75, 0.002
QsearchMoveLimit, 4, 1, 7, 1.0, 0.002
```

## エンジン

調整対象はCλ（ブランチ`eval-search-coadaptation`の`5f8f425`）の調整用ビルド（SHA-256 `5fdb5e67df2e313feda95a3b7ee9280111dff340b9e588750cf9f49d66342ddb`）である。
開始値は、M（master `4fb1582`）の係数の表の既定値と一致する。
runnerはmaster `4fb1582`のビルドを`data/spsa/eval-coadaptation-runner/spsa_runner`に複製して使い、そのSHA-256は460eeb30180f38d203fda74398da11dd1d917f530c363faa4405216f8842bbb3である。
規則セットは`engine-default`（`L0,P0,R1,E0`）である。

## 環境

CPUはIntel Core Ultra 7 265KF（物理20コア、論理20コア）、`Threads`は1、`USI_Hash`は既定の256MB、同時対局数は16である。
2026年10月4日22時56分から10月5日6時38分に実施し、同じ時間帯に他の対局や学習は走っていなかった。

## 結果

375反復を適用し、有効ペア2,853、手数上限による破棄ペア147であった。
不正着手、クラッシュ、応答タイムアウト、`time_forfeits`、および拒否着手はすべて0件、経過時間は27,777秒（約7.7時間）である。

θの開始値と最終値（整数へ丸めた値）は次のとおりである。
移動量は、開始値から最終値までの変化を`c_end`で割った値であり、丸める前の最終値を使った。

| 係数 | 開始値 | 最終値 | 移動量 |
|---|---:|---:|---:|
| `LmrDivisor` | 156 | 164 | +0.15 |
| `LmrHistoryThreshold` | 146 | 143 | −0.04 |
| `FutilityMargin1` | 165 | 168 | +0.04 |
| `FutilityMargin2` | 239 | 250 | +0.16 |
| `FutilityMargin3` | 283 | 305 | +0.33 |
| `NonImprovingFutility1` | 71 | 69 | −0.11 |
| `NonImprovingFutility2` | 72 | 73 | +0.07 |
| `NonImprovingFutility3` | 86 | 84 | −0.20 |
| `LmpBase` | 382 | 369 | −0.06 |
| `LmpSlope` | 133 | 164 | +0.43 |
| `ReverseFutilityMargin` | 428 | 362 | −0.28 |
| `RazoringMargin1` | 1,606 | 1,647 | +0.09 |
| `RazoringMargin2` | 2,015 | 2,278 | +0.55 |
| `SeeMargin1` | 10 | 13 | +0.05 |
| `SeeMargin2` | 192 | 191 | −0.01 |
| `SeeMargin3` | 13 | 22 | +0.14 |
| `AspirationDelta` | 55 | 65 | +0.31 |
| `AspirationGrowth` | 190 | 172 | −0.38 |
| `NullMoveBase` | 3,617 | 3,559 | −0.10 |
| `NullMoveSlope` | 257 | 272 | +0.29 |
| `NullMoveEvalScale` | 56 | 56 | −0.02 |
| `HistoryLimit` | 17,408 | 16,971 | −0.04 |
| `HistoryDecay` | 23 | 16 | −0.54 |
| `CaptureHistoryLimit` | 16,950 | 19,016 | +0.29 |
| `CaptureHistoryScale` | 49 | 43 | −0.24 |
| `CorrectionCap` | 220 | 213 | −0.12 |
| `CorrectionWeight` | 37 | 45 | +0.41 |
| `DeltaMargin` | 355 | 383 | +0.38 |
| `QsearchMoveLimit` | 4 | 4 | +0.16 |

## 結論

最終値を`spsa_runner apply`でCλへ書き込んだコミット`4212114`をCλ\*とする。
`apply`は29係数のうち27係数を変え、`NullMoveEvalScale`と`QsearchMoveLimit`は丸めた値が開始値と同じだった。
Cλ\*の調整用ビルドが宣言する既定値は、`apply`の出力した整数と一致した。
移動量の絶対値は最大で0.55（`RazoringMargin2`）であり、[探索部の再調整](search-revival-t.md)の最大1.75より小さかった。
