# 段階9SPSA候補のSTC採否測定

[事前対局](spsa-stage9-20260923-smoke.md)で時間切れとエンジン異常が0件だったため、2026年9月24日にSTCのGSPRTを開始した。
候補コミットは`2ec3a5e563f998afed7bb33961d59bfdae0c1218`、調整前の基準コミットは`7a5a0e3b219d41cacdd69cab4fd5406a217205a1`である。
基本シードは`76100000`、同時対局数は16、上限は3,000ペアである。
固定worktreeのrunnerのSHA-256は`06fe712111f3a19fdfc63b9d3a66a9bc768d9bdcc2ca8b2c7a27c46302f15bd0`である。

実行ディレクトリは`data/matches/spsa-stage9-20260923-stc`、ログは`data/matches/spsa-stage9-20260923-stc.log`である。
測定は次のコマンドでユーザーサービス`minase-spsa-stage9-stc.service`として起動した。

```console
data/worktrees/spsa-stage9-20260923-runner/target/release/match_runner \
  --run-dir data/matches/spsa-stage9-20260923-stc --seed 76100000 \
  --candidate commit:2ec3a5e563f998afed7bb33961d59bfdae0c1218 \
  --baseline commit:7a5a0e3b219d41cacdd69cab4fd5406a217205a1 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

判定には[標準の振分け規則](../guides/sprt.md#機能採否の段階ゲートstcとltc)を使う。
`decision: H1`ならLTCへ進み、`decision: H0`なら不採用とする。
上限3,000ペアで判定保留なら、その時点のLLRが0以上の場合に限りLTCへ進む。
2026年9月24日にユーザーサービス`minase-spsa-stage9-transition.service`を起動し、STCの終了後にこの規則でLTCへの移行を自動判定するようにした。
監視プログラムは`data/spsa/stage9-20260923-transition.py`（SHA-256 `bb4150356abee26e65a1966c3a21ed57fc25906293fa0ca3ac573d8a072aa6eb`）で、状態は`data/spsa/stage9-20260923-transition.json`、ログは`data/spsa/stage9-20260923-transition.log`に置く。
監視は新しい対局記録にエンジン異常を見つけた時点でSTCを停止し、手動調査を求める。
正常終了後は最終集計と保存記録を照合し、通過時だけシード`76200000`と固定済みrunnerを使って`data/matches/spsa-stage9-20260923-ltc`にLTCのGSPRTを起動する。
LTCの採用判定は、その測定が終了してから別に行う。
測定結果は終了後に記録する。
