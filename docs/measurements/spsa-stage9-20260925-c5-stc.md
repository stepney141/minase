# C5のSPSA候補の短時間測定

## 目的

[減衰する利得C5による調整セッション](spsa-stage9-20260925-c5.md)の最終値を反映した候補を調整前の基準と比較し、長時間測定へ進めるか判定する。

## コマンドライン

```console
data/worktrees/spsa-stage9-20260925-c5-runner/target/release/match_runner \
  --run-dir data/matches/spsa-stage9-20260925-c5-stc --seed 78300000 \
  --candidate commit:631974ee6b61789ab3400403003845d26bb3a6d4 \
  --baseline commit:9bc6898d8c428a1f208a9ab4c2b2f67de0c67cc4 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

## エンジン

候補はコミット`631974ee6b61789ab3400403003845d26bb3a6d4`、基準はコミット`9bc6898d8c428a1f208a9ab4c2b2f67de0c67cc4`である。
規則は`engine-default`（`L0,P0,R1,E0`）を用い、固定runnerのSHA-256は`06fe712111f3a19fdfc63b9d3a66a9bc768d9bdcc2ca8b2c7a27c46302f15bd0`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア）、両エンジンは`Threads=1`、`USI_Hash=256 MB`、同時対局数は16である。
実行ディレクトリは`data/matches/spsa-stage9-20260925-c5-stc`、ログは`data/matches/spsa-stage9-20260925-c5-stc.log`である。
2026年9月25日19時55分に、事前対局の異常0件を確かめた監視スクリプトがユーザーサービス`minase-spsa-c5-stc.service`として開始した。
同じスクリプトは、STCの判定が標準の振分け規則でLTCへ進む場合に限り、基本シード`78400000`でLTCを続けて開始する。

## 結果

判定には1,123ペアを取り込み、1,055ペアが有効、68ペアが破棄された。
ペンタノミアル度数は`[191, 29, 552, 36, 247]`、LLRは`+2.9671200406`、判定は`H1`である。
エンジン異常、時間切れ、および拒否着手はすべて0件で、経過時間は10,862.228855秒だった。

## 結論

[標準の振分け規則](../guides/sprt.md#機能採否の段階ゲートstcとltc)に従い、候補を[長時間測定](spsa-stage9-20260925-c5-ltc.md)へ進めた。
監視スクリプトは最終集計の判定と異常件数を確かめ、2026年9月25日22時56分にLTCを開始した。
