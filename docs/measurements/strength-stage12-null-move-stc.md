# strength-stage12-null-move-stc

## 目的

[棋力向上段階12](../plans/strength-stage12.md)の項目7（null move pruningの前提条件）の実装を段階開始版と短時間条件で対局させ、LTCへ進めるかを判定する。

## コマンドライン

測定は、STCの判定に応じてLTCを続けて開始する監視スクリプト（`data/experiments/stage12-diag/gate.sh`、煙試験を先に行う`smoke_then_gate.sh`経由）から、次のコマンドで実行した。

```console
data/worktrees/strength-stage12-runner/target/release/match_runner \
  --run-dir data/matches/strength-stage12-null-move-stc --seed 82100000 \
  --candidate commit:34385c66352337ecd430752948cb8526268ad947 \
  --baseline commit:df0c75ef980b74040e25313c8d60aff17ddb985a \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`34385c66352337ecd430752948cb8526268ad947`（バイナリのSHA-256は`6b3d9a75fd68cbb200860840ca200cd3ae24a2d67881418ced61a2d49ddf49ca`）、基準は段階開始版のコミット`df0c75ef980b74040e25313c8d60aff17ddb985a`（`7a1ba00c426634cf98fb1eaf14ee64cc87d188c24320100d648d9277343df40e`）である。
規則は既定の`engine-default`（`L0,P0,R1,E0`）である。
runnerのSHA-256は`ac3485eff3a5f5c8005d7c80e135a8d64f031422b923aefff190711ae7ada075`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、両エンジンは`Threads=1`、`USI_Hash=256 MB`、同時対局数は16である。
2026年9月27日に実行した。

## 結果

判定には上限の3,000ペアを取り込み、2,853ペアが有効、147ペアが手数上限への到達で破棄された。
ペンタノミアル度数は`[596, 82, 1493, 92, 590]`、LLRは`−2.8070840143`、判定は`pending`である。
候補の得点率は50.0%である。
エンジン異常、時間切れ、および拒否着手はすべて0件で、経過時間は27,947.6秒（`summary.json`の累計は27,720.2秒）だった。

## 結論

上限の3,000ペアに達した`pending`であり、停止時点のLLRが負なので、段階ゲートの振分け規則により項目7はLTCへ進めず、採用しない。
