# 段階9のSPSA候補の長時間測定

## 目的

[短時間測定](spsa-stage9-20260923-stc.md)で`H1`となったSPSA候補を調整前の基準と比較し、採否を判定する。

## コマンドライン

```console
data/worktrees/spsa-stage9-20260923-runner/target/release/match_runner \
  --run-dir data/matches/spsa-stage9-20260923-ltc --seed 76200000 \
  --candidate commit:2ec3a5e563f998afed7bb33961d59bfdae0c1218 \
  --baseline commit:7a5a0e3b219d41cacdd69cab4fd5406a217205a1 \
  --each time=60000+200 --concurrency 16 gsprt
```

## エンジン

候補はコミット`2ec3a5e563f998afed7bb33961d59bfdae0c1218`、基準はコミット`7a5a0e3b219d41cacdd69cab4fd5406a217205a1`である。
規則は`engine-default`（`L0,P0,R1,E0`）を用い、固定runnerのSHA-256は`06fe712111f3a19fdfc63b9d3a66a9bc768d9bdcc2ca8b2c7a27c46302f15bd0`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、両エンジンは`Threads=1`、`USI_Hash=256 MB`、同時対局数は16である。
実行ディレクトリは`data/matches/spsa-stage9-20260923-ltc`、ログは`data/matches/spsa-stage9-20260923-ltc.log`である。
2026年9月25日00時28分にユーザーサービス`minase-spsa-stage9-ltc.service`として開始した。

## 結果

測定中。終了後に最終集計を記録する。

## 結論

判定待ち。
