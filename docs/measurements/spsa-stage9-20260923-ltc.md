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
2026年9月25日00時28分にユーザーサービス`minase-spsa-stage9-ltc.service`として開始し、同日09時18分に終了した。
測定中の2026年9月25日に、同じ測定機で20スレッドの模擬計算（[spsa-gain-simulation](spsa-gain-simulation.md)）を約13分間実行した。この間の対局は高負荷の下で指されたが、時間切れとエンジン異常は、その時点までの226ペアで0件だった。

## 結果

判定には1,137ペアを取り込み、1,077ペアが有効、60ペアが破棄された。
ペンタノミアル度数は`[195, 34, 555, 41, 252]`、LLRは`+2.9998194185`、判定は`H1`である。
エンジン異常、時間切れ、および拒否着手はすべて0件で、経過時間は31,817.369717秒だった。
判定後に並列対局で保存されたペアは、この集計に含めない。

## 結論

[標準の採否規則](../guides/sprt.md#機能採否の段階ゲートstcとltc)の採用条件を満たした。
