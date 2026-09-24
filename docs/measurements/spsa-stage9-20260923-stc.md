# 段階9のSPSA候補の短時間測定

## 目的

[最初の調整セッション](spsa-stage9-20260923.md)の最終値を反映した候補を調整前の基準と比較し、長時間測定へ進めるか判定する。

## コマンドライン

```console
data/worktrees/spsa-stage9-20260923-runner/target/release/match_runner \
  --run-dir data/matches/spsa-stage9-20260923-stc --seed 76100000 \
  --candidate commit:2ec3a5e563f998afed7bb33961d59bfdae0c1218 \
  --baseline commit:7a5a0e3b219d41cacdd69cab4fd5406a217205a1 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`2ec3a5e563f998afed7bb33961d59bfdae0c1218`、基準はコミット`7a5a0e3b219d41cacdd69cab4fd5406a217205a1`である。
規則は`engine-default`（`L0,P0,R1,E0`）を用い、固定runnerのSHA-256は`06fe712111f3a19fdfc63b9d3a66a9bc768d9bdcc2ca8b2c7a27c46302f15bd0`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、両エンジンは`Threads=1`、`USI_Hash=256 MB`、同時対局数は16である。
実行ディレクトリは`data/matches/spsa-stage9-20260923-stc`、ログは`data/matches/spsa-stage9-20260923-stc.log`である。
2026年9月24日にユーザーサービス`minase-spsa-stage9-stc.service`として開始し、2026年9月25日に終了した。

## 結果

判定には735ペアを取り込み、688ペアが有効、47ペアが破棄された。
ペンタノミアル度数は`[114, 21, 365, 19, 169]`、LLRは`+2.9921660090`、判定は`H1`である。
エンジン異常、時間切れ、および拒否着手はすべて0件で、経過時間は7,177.803385秒だった。

## 結論

[標準の振分け規則](../guides/sprt.md#機能採否の段階ゲートstcとltc)に従い、候補を[長時間測定](spsa-stage9-20260923-ltc.md)へ進めた。
ユーザーサービス`minase-spsa-stage9-transition.service`は最終集計と保存記録を照合し、2026年9月25日00時28分にLTCを自動起動した。
監視プログラムは`data/spsa/stage9-20260923-transition.py`（SHA-256 `bb4150356abee26e65a1966c3a21ed57fc25906293fa0ca3ac573d8a072aa6eb`）で、判定結果は`data/spsa/stage9-20260923-transition.json`に残した。
