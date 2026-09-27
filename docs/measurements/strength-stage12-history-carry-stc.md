# strength-stage12-history-carry-stc

## 目的

[棋力向上段階12](../plans/strength-stage12.md)の項目5（butterfly historyの対局内の持ち越し）の実装を段階開始版と短時間条件で対局させ、LTCへ進めるかを判定する。

## コマンドライン

測定は、STCの判定に応じてLTCを続けて開始する監視スクリプト（`data/experiments/stage12-diag/gate.sh`、煙試験を先に行う`smoke_then_gate.sh`経由）から、次のコマンドで実行した。

```console
data/worktrees/strength-stage12-runner/target/release/match_runner \
  --run-dir data/matches/strength-stage12-history-carry-stc --seed 81900000 \
  --candidate commit:1b96a3db4a946aff853e373bfedef532bccc329f \
  --baseline commit:df0c75ef980b74040e25313c8d60aff17ddb985a \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`1b96a3db4a946aff853e373bfedef532bccc329f`（バイナリのSHA-256は`9c7570913c113a234b984aff98ca8c0ecf49bece3d38d04791c0a6f655ffc21c`）、基準は段階開始版のコミット`df0c75ef980b74040e25313c8d60aff17ddb985a`（`7a1ba00c426634cf98fb1eaf14ee64cc87d188c24320100d648d9277343df40e`）である。
規則は既定の`engine-default`（`L0,P0,R1,E0`）である。
runnerのSHA-256は`ac3485eff3a5f5c8005d7c80e135a8d64f031422b923aefff190711ae7ada075`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、両エンジンは`Threads=1`、`USI_Hash=256 MB`、同時対局数は16である。
2026年9月27日に実行した。

## 結果

判定には2,688ペアを取り込み、2,539ペアが有効、149ペアが手数上限への到達で破棄された。
ペンタノミアル度数は`[560, 86, 1265, 72, 556]`、LLRは`−3.0035193636`、判定は`H0`である。
候補の得点率は49.8%である。
エンジン異常、時間切れ、および拒否着手はすべて0件で、経過時間は25,289.4秒（`summary.json`の累計は25,090.8秒）だった。

## 結論

`H0`なので、項目5は採用せず、LTCへ進めない。
