# C5のSPSA候補の長時間測定

## 目的

[短時間測定](spsa-stage9-20260925-c5-stc.md)で`H1`となった、[減衰する利得C5による調整セッション](spsa-stage9-20260925-c5.md)の候補を調整前の基準と比較し、採否を判定する。

## コマンドライン

```console
data/worktrees/spsa-stage9-20260925-c5-runner/target/release/match_runner \
  --run-dir data/matches/spsa-stage9-20260925-c5-ltc --seed 78400000 \
  --candidate commit:631974ee6b61789ab3400403003845d26bb3a6d4 \
  --baseline commit:9bc6898d8c428a1f208a9ab4c2b2f67de0c67cc4 \
  --each time=60000+200 --concurrency 16 gsprt
```

## エンジン

候補はコミット`631974ee6b61789ab3400403003845d26bb3a6d4`、基準はコミット`9bc6898d8c428a1f208a9ab4c2b2f67de0c67cc4`である。
規則は`engine-default`（`L0,P0,R1,E0`）を用い、固定runnerのSHA-256は`06fe712111f3a19fdfc63b9d3a66a9bc768d9bdcc2ca8b2c7a27c46302f15bd0`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、両エンジンは`Threads=1`、`USI_Hash=256 MB`、同時対局数は16である。
実行ディレクトリは`data/matches/spsa-stage9-20260925-c5-ltc`、ログは`data/matches/spsa-stage9-20260925-c5-ltc.log`である。
2026年9月25日22時56分に、STCの判定を確かめた監視スクリプトがユーザーサービス`minase-spsa-c5-ltc.service`として開始した。

## 結果

測定中。

## 結論

判定待ち。
