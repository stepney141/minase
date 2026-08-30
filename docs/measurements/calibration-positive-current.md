# 校正カード正例の現行時間制御200ペア

## 目的

学習PST（コミット`7e13888`）を候補、駒得評価（コミット`e553493`）を基準とする正例カードを現行時間制御で200ペア対局させ、期待する正のElo差が出ることと、保存記録だけから校正指標を再集計できることを確認する。

## コマンドライン

コマンドラインの原文は残っていないため、次の行は実行ディレクトリの`manifest.json`に保存された条件から復元した。
実行に使ったバイナリは、`target/match-calibration/phase3-20260828/`へ固定した`match_runner`（SHA-256 `7a837ef8771bc03a83dd203288d9f5775ca9b1a2b622dcbbf7431ff56acfd714`）である。

```console
target/match-calibration/phase3-20260828/match_runner \
  --run-dir data/match-harness-efficiency/phase3/positive-current --seed 20260828 \
  --candidate commit:7e13888 --baseline commit:e553493 \
  --rules engine-default --each time=30000+300,byoyomi=500 --concurrency 8 elo --pairs 200
```

再集計は`target/match-calibration/phase3-20260828/match_report`（SHA-256 `bf57bc34bcc4d38c0314b2126a586b2abbaac1bce5d65bb55a3d5d7b76fef20a`）で行った。

## エンジン

候補は学習PSTのコミット`7e13888a8abe1752e9dd2aba4c4a6a16e61d192e`であり、バイナリのSHA-256は`bd01c5ff193055282563235540f207e1236d327db79c9e21ae158ce36f98a693`である。
基準は駒得評価のコミット`e553493161d9c873698c980cd8f43a5fe8e34444`であり、バイナリのSHA-256は`b0af5fcc66be62008a89c31dbc70ef853f598245c262d80721c0125a3975ff70`である。
規則セットは`engine-default`（L0＋P0＋R1＋E0）を両エンジンと審判層に与えた。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ33,218,965,504バイト）であり、OSはLinuxである。
候補と基準のワーカー数はいずれも`Threads=1`、`USI_Hash`はいずれも256 MB、同時対局数は8、手数上限は4,096手、応答タイムアウトは120秒とした。

## 結果

200ペアを12,221.71秒（`summary.json`の`active_wall_time_ns`）で完走した。
ペンタノミアル度数は`[28, 0, 101, 1, 69]`であり、有効ペアは199、手数上限による破棄は1ペア（`cutoff`1局）だった。
Eloは73.53、95%信頼区間は40.43〜108.02である。
エンジン異常、`time_forfeits`、および拒否着手はいずれも0件であり、終局理由は詰み393局、反復裁定5局、駒枯れ引き分け1局、手数上限1局だった。
両エンジンの総CPU時間は88,819.70秒、有効ペア毎時は58.62、校正の第1の主指標（正規化ペア得点の母分散と平均CPU時間の積）は49,676,925,600.77、第2の主指標（単位CPU時間当たりの証拠の強さ）は0.0002188645だった。

## 結論

正例は期待した正の符号を有意に示し、200ペアの保存、再集計、および異常検査が実データで機能した。
