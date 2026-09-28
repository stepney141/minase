# strength-stage12-capture-history-stc

## 目的

[棋力向上段階12](../plans/strength-stage12.md)の項目3（捕獲履歴）の実装を段階開始版と短時間条件で対局させ、LTCへ進めるかを判定する。

## コマンドライン

測定は、STCの判定に応じてLTCを続けて開始する監視スクリプト（`data/experiments/stage12-diag/gate.sh`）から、次のコマンドで実行した。

```console
data/worktrees/strength-stage12-runner/target/release/match_runner \
  --run-dir data/matches/strength-stage12-capture-history-stc --seed 81500000 \
  --candidate commit:1ca19fc02f71f3788d2d653091a48948051d4e57 \
  --baseline commit:df0c75ef980b74040e25313c8d60aff17ddb985a \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`1ca19fc02f71f3788d2d653091a48948051d4e57`（バイナリのSHA-256は`490eb017442b516515368fccb0ec5cc55a29de07189b036571f0a0af7b774501`）、基準は段階開始版のコミット`df0c75ef980b74040e25313c8d60aff17ddb985a`（`7a1ba00c426634cf98fb1eaf14ee64cc87d188c24320100d648d9277343df40e`）である。
規則は既定の`engine-default`（`L0,P0,R1,E0`）である。
runnerのSHA-256は`ac3485eff3a5f5c8005d7c80e135a8d64f031422b923aefff190711ae7ada075`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、両エンジンは`Threads=1`、`USI_Hash=256 MB`、同時対局数は16である。
2026年9月27日に実行した。

## 結果

判定には787ペアを取り込み、750ペアが有効、37ペアが手数上限への到達で破棄された。
ペンタノミアル度数は`[184, 21, 375, 24, 146]`、LLRは`−2.9872521544`、判定は`H0`である。
候補の得点率は47.6%である。
エンジン異常、時間切れ、および拒否着手はすべて0件で、経過時間は7,400.2秒（`summary.json`の累計は7,339.8秒）だった。

## 結論

`H0`なので、項目3は採用せず、LTCへ進めない。
