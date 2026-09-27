# strength-stage12-qsearch-limit-stc

## 目的

[棋力向上段階12](../plans/strength-stage12.md)の項目4（静止探索の手数制限、`N=1`）の実装を段階開始版と短時間条件で対局させ、LTCへ進めるかを判定する。

## コマンドライン

測定は、STCの判定に応じてLTCを続けて開始する監視スクリプト（`data/experiments/stage12-diag/gate.sh`、煙試験を先に行う`smoke_then_gate.sh`経由）から、次のコマンドで実行した。

```console
data/worktrees/strength-stage12-runner/target/release/match_runner \
  --run-dir data/matches/strength-stage12-qsearch-limit-stc --seed 81700000 \
  --candidate commit:a3845f27a03c8fdc05573c4723324fcd1ea91b30 \
  --baseline commit:df0c75ef980b74040e25313c8d60aff17ddb985a \
  --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  --each time=10000+100 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`a3845f27a03c8fdc05573c4723324fcd1ea91b30`（バイナリのSHA-256は`b98b7805e5ea6dd722910e3c7552728e022fcee2ba13f64819c69b6396b781ec`）、基準は段階開始版のコミット`df0c75ef980b74040e25313c8d60aff17ddb985a`（`7a1ba00c426634cf98fb1eaf14ee64cc87d188c24320100d648d9277343df40e`）である。
規則は既定の`engine-default`（`L0,P0,R1,E0`）である。
runnerのSHA-256は`ac3485eff3a5f5c8005d7c80e135a8d64f031422b923aefff190711ae7ada075`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、両エンジンは`Threads=1`、`USI_Hash=256 MB`、同時対局数は16である。
2026年9月27日に実行した。

## 結果

判定には154ペアを取り込み、147ペアが有効、7ペアが手数上限への到達で破棄された。
ペンタノミアル度数は`[81, 6, 43, 1, 16]`、LLRは`−2.9461927157`、判定は`H0`である。
候補の得点率は27.0%であり、ロジスティック換算で約−173 Eloに当たる。
エンジン異常、時間切れ、および拒否着手はすべて0件で、経過時間は1,726.1秒（`summary.json`の累計は1,712.9秒）だった。

## 結論

`H0`なので、項目4は採用せず、LTCへ進めない。
フェーズ1の診断では、`N=1`で失う良い結果はbench深さ6で4.83%、時間制御の予算で2.57〜3.04%だったが、自己対局の損失はこの割合から想定される大きさを大きく上回った。
