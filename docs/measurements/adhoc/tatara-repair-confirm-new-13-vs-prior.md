# 教師30本で約2.05エポック学んだ新型と第2世代の教師で学んだ移行後の最終版の対局

tatara方式への移行で生じたA型評価関数の棋力低下を調べるため、検証のための対局を行った。
旧型はtatara方式への移行前のA型、新型は移行後のA型を指す。教師30本はmasterの標準ネットの学習に使った30ファイルである。
第2世代の教師は、tatara方式への移行時に新旧を学習させた37ファイルを指す。
候補は教師30本で約2.05エポック学んだ新型、基準は第2世代の教師で学んだ移行後の最終版で、同じ開始局面から先後を入れ替えた2局を1ペアとした。

日本中将棋連盟の規則で双方に1手3,000ノードを与え、シード 20261119430000 から開始局面を生成した。
Intel Core Ultra 7 265KFで7ペアを並行実行し、両側とも探索は1スレッド、置換表は32 MBとした。

| 対局側 | エンジンのコミット | 実行ファイルのSHA-256 | 評価関数のSHA-256 |
|---|---|---|---|
| 候補 | `8b853b66` | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `70fa0052c20c57ff3e0e5515a27c59b788f311a551238b90a259d39a809cf258` |
| 基準 | `8b853b66` | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `37054c9e49c9218bb18efdd1814fe1b298de5cbfc9c14352f6ad6eaecbb70dea` |

全240ペアを集計した。
2局合計で候補の得点が順に0、0.5、1、1.5、2点だったペア数は `[2, 0, 60, 0, 178]` で、得点率は86.67%だった。
基準に対する候補の棋力差は+325.2 Elo、95%信頼区間は+284.9〜+373.8 Eloだった。
エンジン異常、時間切れ、着手の拒否はいずれも0件だった。実行時間は226.1秒だった。

## 再現手順

本体リポジトリのルートで測定した。対局ハーネスのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase match run --candidate commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --baseline commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --candidate-option EvalFile=data/tatara-p-repair-20261119/new-13/nnue.bin --baseline-option EvalFile=data/nnue-tatara-framework-and-halfkp-vm/train-after/nnue.bin --each nodes=3000 --rules JCSA --candidate-hash 32 --baseline-hash 32 --concurrency 7 --seed 20261119430000 --run-dir data/tatara-p-repair-20261119/matches/confirm-new-13-vs-prior elo --pairs 240
```
