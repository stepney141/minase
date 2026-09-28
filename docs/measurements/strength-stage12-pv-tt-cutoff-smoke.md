# strength-stage12-pv-tt-cutoff-smoke

## 目的

[棋力向上段階12](../plans/strength-stage12.md)の項目8（PVノードでの置換表の打ち切りの停止）の実装を段階開始版と短時間条件で32ペア対局させ、STCの本測定の前に時間切れ、エンジン異常、および拒否着手が起きないことを確かめる。

## コマンドライン

煙試験で異常が0件ならSTCとLTCへ続けて進む監視スクリプト（`data/experiments/stage12-diag/smoke_then_gate.sh`）から、次のコマンドで実行した。

```console
data/worktrees/strength-stage12-runner/target/release/match_runner \
  --run-dir data/matches/strength-stage12-pv-tt-cutoff-smoke --seed 82250000 \
  --candidate commit:5f021046307e9539bc9539beb651a1c3b4e49051 --baseline commit:df0c75e \
  --each time=10000+100 --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  gsprt --max-pairs 32
```

## エンジン

候補はコミット`5f021046307e9539bc9539beb651a1c3b4e49051`（バイナリのSHA-256は`fbbf97008cf7251ee0019aeafef527dd920b2263d23b9b72668c309cbf9008c5`）、基準は段階開始版のコミット`df0c75ef980b74040e25313c8d60aff17ddb985a`（`7a1ba00c426634cf98fb1eaf14ee64cc87d188c24320100d648d9277343df40e`）である。
規則は既定の`engine-default`（`L0,P0,R1,E0`）である。
runnerは段階開始版に固定したworktree `data/worktrees/strength-stage12-runner` でビルドし、SHA-256は`ac3485eff3a5f5c8005d7c80e135a8d64f031422b923aefff190711ae7ada075`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、両エンジンは`Threads=1`、`USI_Hash=256 MB`、同時対局数は16である。
2026年9月27日に実行した。

## 結果

32ペアのすべてが有効だった。
ペンタノミアル度数は`[7, 2, 14, 2, 7]`、LLRは`−0.028`、判定は`pending`である。
エンジン異常、時間切れ、および拒否着手はすべて0件で、経過時間は両コミットのビルドを含めて362.6秒だった。
この件数は採否の根拠にしない。

## 結論

時間切れ、エンジン異常、および拒否着手が0件なので、STCの本測定へ進める。
