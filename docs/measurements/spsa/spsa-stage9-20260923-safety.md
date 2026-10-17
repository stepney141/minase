# SPSAの時間切れ事前確認

2026年9月23日に、[最初の調整セッション](spsa-stage9-20260923.md)に先立って時間管理係数の端点を確認した。
調整対象とrunnerはともに`7a5a0e3b219d41cacdd69cab4fd5406a217205a1`であり、調整用エンジンのSHA-256は`d0669ad9a36ad3ec8b97af5172427850e5f54ce60f155cae2c063687da99fb08`、runnerのSHA-256は`7c77876de32fe34c4f485d10e3df3c6ff9ea8800cb57584d6af4b5894c658439`である。
規則は`engine-default`、時間制御は`time=10000+100`、同時対局数は16、基本シードは`73000000`とした。

```console
data/worktrees/spsa-stage9-20260923-runner/target/release/spsa_runner \
  --run-dir data/spsa/stage9-20260923-safety --seed 73000000 \
  --engine commit:7a5a0e3b219d41cacdd69cab4fd5406a217205a1 \
  --params data/spsa/stage9-20260923-safety.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 13 --pairs-per-iteration 8
```

パラメーターファイルは調整用エンジンの宣言から生成し、`MinMoves`、`HardSoftRatio`、`HardRemainingShare`、`IterationRatio`の開始値だけを、最も長く考える側の端点へ変更した。
ファイルの全内容は次のとおりである。

```text
LmrDivisor, 200, 100, 400, 15, 0.002
LmrHistoryThreshold, 128, 0, 512, 25.6, 0.002
FutilityMargin1, 50, 0, 400, 20, 0.002
FutilityMargin2, 150, 0, 400, 20, 0.002
FutilityMargin3, 150, 0, 400, 20, 0.002
SeeMargin1, 0, 0, 400, 20, 0.002
SeeMargin2, 200, 0, 400, 20, 0.002
SeeMargin3, 0, 0, 400, 20, 0.002
AspirationDelta, 50, 10, 200, 9.5, 0.002
AspirationGrowth, 200, 125, 400, 13.75, 0.002
NullMoveBase, 2400, 1200, 4800, 180, 0.002
NullMoveSlope, 200, 100, 400, 15, 0.002
HistoryLimit, 16384, 4096, 65536, 3072, 0.002
CorrectionCap, 200, 50, 400, 17.5, 0.002
CorrectionWeight, 32, 8, 128, 6, 0.002
DeltaMargin, 200, 50, 500, 22.5, 0.002
ExpectedPlies, 450, 250, 700, 22.5, 0.002
MinMoves, 40, 40, 200, 8, 0.002
IncrementShare, 70, 30, 100, 3.5, 0.002
HardSoftRatio, 800, 150, 800, 32.5, 0.002
HardRemainingShare, 50, 10, 50, 2, 0.002
IterationRatio, 150, 150, 400, 12.5, 0.002
```

13反復のうち有効99ペア、破棄5ペアで、所要時間は1,228.049秒だった。
不正着手、クラッシュ、応答期限超過、時間切れ、拒否着手はすべて0件であり、本番セッションの開始条件を満たした。
この短いセッションの最終値は係数候補として使わない。
