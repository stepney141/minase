# 校正カード負例の現行時間制御200ペア

## 目的

P型NNUE（コミット`05362b1`）を候補、学習PST（コミット`7e13888`）を基準とする負例カードを現行時間制御で200ペア対局させ、期待する負のElo差が出ることと、保存記録だけから校正指標を再集計できることを確認する。

## コマンドライン

コマンドラインの原文は残っていないため、次の行は実行ディレクトリの`manifest.json`に保存された条件から復元した。
実行に使ったバイナリは、`target/match-calibration/phase3-20260828/`へ固定した`match_runner`（SHA-256 `7a837ef8771bc03a83dd203288d9f5775ca9b1a2b622dcbbf7431ff56acfd714`）である。

```console
target/match-calibration/phase3-20260828/match_runner \
  --run-dir data/match-harness-efficiency/phase3/negative-current --seed 20260828 \
  --candidate commit:05362b1 --baseline commit:7e13888 \
  --rules engine-default --each time=30000+300,byoyomi=500 --concurrency 8 elo --pairs 200
```

再集計は`target/match-calibration/phase3-20260828/match_report`（SHA-256 `bf57bc34bcc4d38c0314b2126a586b2abbaac1bce5d65bb55a3d5d7b76fef20a`）で行った。

## エンジン

候補はP型NNUEのコミット`05362b124ae9e2d0c95a4dda6692bd560d4ef26d`であり、バイナリのSHA-256は`38f63d65592dbfb5e73573ce9c687da8942d0877ee11a3d8a158d0f9c2b03d0b`である。
基準は学習PSTのコミット`7e13888a8abe1752e9dd2aba4c4a6a16e61d192e`であり、バイナリのSHA-256は`bd01c5ff193055282563235540f207e1236d327db79c9e21ae158ce36f98a693`である。
規則セットは`engine-default`（L0＋P0＋R1＋E0）を両エンジンと審判層に与えた。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ33,218,965,504バイト）であり、OSはLinuxである。
候補と基準のワーカー数はいずれも`Threads=1`、`USI_Hash`はいずれも256 MB、同時対局数は8、手数上限は4,096手、応答タイムアウトは120秒とした。

## 結果

200ペアを9,579.00秒（`summary.json`の`active_wall_time_ns`）で完走した。
ペンタノミアル度数は`[200, 0, 0, 0, 0]`であり、有効ペアは200、破棄は0ペアだった。
候補は400局全敗（得点率0/400）であり、ロジスティックEloの点推定は無限大になるため、1局あたり期待スコアの95%上限（rule of three）は3/400＝0.0075である。
エンジン異常、`time_forfeits`、および拒否着手はいずれも0件であり、終局理由は400局すべて詰みだった。
両エンジンの総CPU時間は保存記録から71,784.57秒と再集計できたが、正規化ペア得点の分散が0なので`match_report`は`pair score variance is zero`として主指標の算出を拒否した。

## 結論

負例は期待した負の符号を示し、保存記録と分散0に対する算出不能の拒否は機能したが、差が大きすぎて時間制御間の効率を比較する校正カードとしては不適切だった。
