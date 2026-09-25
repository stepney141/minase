# SPSAの時間切れ事前確認（減衰する利得C5）

2026年9月25日に、[減衰する利得C5による調整セッション](spsa-stage9-20260925-c5.md)に先立って時間管理係数の端点を確認した。
調整対象は`9bc6898d8c428a1f208a9ab4c2b2f67de0c67cc4`、runnerは`996300e`であり、調整用エンジンのSHA-256は`f2afd0446d2282cc9a8930a69acd8ba0a19bfe2db4a24d74b3383d886c5f9827`、runnerのSHA-256は`42e94a301cda14db0fde1e34f968e4e5b87e1c1b0ed2cf0807854c1829ac5b69`である。
規則は`engine-default`、時間制御は`time=10000+100`、同時対局数は16、基本シードは`78020000`とした。
利得は[利得の較正](../plans/spsa-gain-calibration.md)で採用したC5（α = 0.602、γ = 0.101、A = 0.1N、`c_end`は範囲の1/6、`r_end`は0.002）である。
パラメーターファイルは[一定の利得での事前確認](spsa-stage9-20260925-safety.md)と同一であり、時間管理の4係数だけを最も長く考える側の端点に置いた。

```console
data/worktrees/spsa-stage9-20260925-c5-runner/target/release/spsa_runner \
  --run-dir data/spsa/stage9-20260925-c5-safety --seed 78020000 \
  --engine commit:9bc6898d8c428a1f208a9ab4c2b2f67de0c67cc4 \
  --params data/spsa/stage9-20260925-c5-safety.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 13 --pairs-per-iteration 8
```

13反復のうち有効101ペア、破棄3ペアで、所要時間は1,049.766秒だった。
不正着手、クラッシュ、応答期限超過、時間切れ、拒否着手はすべて0件であり、本番セッションの開始条件を満たした。
この短いセッションの最終値は係数候補として使わない。
