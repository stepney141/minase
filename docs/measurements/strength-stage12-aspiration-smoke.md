# strength-stage12-aspiration-smoke

## 目的

[棋力向上段階12](../plans/strength-stage12.md)の項目1（aspiration windowsのfail-high時の減深）の実装を段階開始版と短時間条件で32ペア対局させ、STCの本測定の前に時間切れ、エンジン異常、および拒否着手が起きないことを確かめる。

## コマンドライン

```console
data/worktrees/strength-stage12-runner/target/release/match_runner \
  --run-dir data/matches/strength-stage12-aspiration-smoke --seed 81050000 \
  --candidate commit:19b58f6 --baseline commit:df0c75e \
  --each time=10000+100 --candidate-hash 256 --baseline-hash 256 --concurrency 16 \
  gsprt --max-pairs 32
```

## エンジン

候補はコミット`19b58f69b5e58174da69fffbeb65dcb6084d99b8`（バイナリのSHA-256は`761104a966bc6d8ecd5869677c9dc72a0035615a31dbae781b743c4a81d17b9b`）、基準は段階開始版のコミット`df0c75ef980b74040e25313c8d60aff17ddb985a`（`7a1ba00c426634cf98fb1eaf14ee64cc87d188c24320100d648d9277343df40e`）である。
規則は既定の`engine-default`（`L0,P0,R1,E0`）である。
runnerは段階開始版に固定したworktree `data/worktrees/strength-stage12-runner` でビルドし、SHA-256は`ac3485eff3a5f5c8005d7c80e135a8d64f031422b923aefff190711ae7ada075`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、両エンジンは`Threads=1`、`USI_Hash=256 MB`、同時対局数は16である。
2026年9月27日に実行した。

## 結果

32ペアのうち31ペアが有効、1ペアが破棄された。
ペンタノミアル度数は`[8, 0, 14, 4, 5]`、LLRは`−0.093`、判定は`pending`である。
エンジン異常、時間切れ、および拒否着手はすべて0件で、経過時間は両コミットのビルドを含めて463.9秒だった。
この件数は採否の根拠にしない。

## 結論

時間切れ、エンジン異常、および拒否着手が0件なので、STCの本測定へ進める。
