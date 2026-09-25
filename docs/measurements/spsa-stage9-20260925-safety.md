# SPSAの時間切れ事前確認（一定の利得）

2026年9月25日に、[一定の利得による調整セッション](spsa-stage9-20260925.md)に先立って時間管理係数の端点を確認した。
調整対象は`9bc6898d8c428a1f208a9ab4c2b2f67de0c67cc4`（段階9の構成に最初のSPSAセッションの値を採用したmaster）、runnerは`bd45b0a`であり、調整用エンジンのSHA-256は`54cf3dc2219ce059ff5e51f87aa3874012e353dea1e0dbe31c5a891ca4e78f4f`、runnerのSHA-256は`aaf990ac93128ec67c7826b7c63c37450275bf7e5a06e5e28ca2ce2088fae9df`である。
規則は`engine-default`、時間制御は`time=10000+100`、同時対局数は16、基本シードは`78000000`とした。
利得は[利得の較正](../plans/spsa-gain-calibration.md)で選んだ一定の利得（α = γ = 0、`c_end`は範囲の1/6）である。

```console
data/worktrees/spsa-stage9-20260925-runner/target/release/spsa_runner \
  --run-dir data/spsa/stage9-20260925-safety --seed 78000000 \
  --engine commit:9bc6898d8c428a1f208a9ab4c2b2f67de0c67cc4 \
  --params data/spsa/stage9-20260925-safety.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 13 --pairs-per-iteration 8
```

パラメーターファイルは調整用エンジンの宣言から生成し、`MinMoves`、`HardSoftRatio`、`HardRemainingShare`、`IterationRatio`の開始値だけを、最も長く考える側の端点へ変更した。
ファイルの全内容は次のとおりである。

```text
LmrDivisor, 186, 100, 400, 50, 0.002
LmrHistoryThreshold, 117, 0, 512, 85.33333333333333, 0.002
FutilityMargin1, 51, 0, 400, 66.66666666666667, 0.002
FutilityMargin2, 158, 0, 400, 66.66666666666667, 0.002
FutilityMargin3, 175, 0, 400, 66.66666666666667, 0.002
SeeMargin1, 0, 0, 400, 66.66666666666667, 0.002
SeeMargin2, 195, 0, 400, 66.66666666666667, 0.002
SeeMargin3, 10, 0, 400, 66.66666666666667, 0.002
AspirationDelta, 53, 10, 200, 31.666666666666668, 0.002
AspirationGrowth, 193, 125, 400, 45.833333333333336, 0.002
NullMoveBase, 2888, 1200, 4800, 600, 0.002
NullMoveSlope, 210, 100, 400, 50, 0.002
HistoryLimit, 19403, 4096, 65536, 10240, 0.002
CorrectionCap, 202, 50, 400, 58.333333333333336, 0.002
CorrectionWeight, 35, 8, 128, 20, 0.002
DeltaMargin, 212, 50, 500, 75, 0.002
ExpectedPlies, 435, 250, 700, 75, 0.002
MinMoves, 40, 40, 200, 26.666666666666668, 0.002
IncrementShare, 73, 30, 100, 11.666666666666666, 0.002
HardSoftRatio, 800, 150, 800, 108.33333333333333, 0.002
HardRemainingShare, 50, 10, 50, 6.666666666666667, 0.002
IterationRatio, 150, 150, 400, 41.666666666666664, 0.002
```

13反復のうち有効96ペア、破棄8ペアで、所要時間は1,332.651秒だった。
不正着手、クラッシュ、応答期限超過、時間切れ、拒否着手はすべて0件であり、本番セッションの開始条件を満たした。
この短いセッションの最終値は係数候補として使わない。
