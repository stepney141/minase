# 教師30本で約2.05エポック学んだ新型と第2世代の教師で約2.07エポック学んだ新型の対局

tatara方式への移行で生じたP型評価関数の棋力低下を調べるため、検証のための対局を行った。
旧型はtatara方式への移行前のP型、新型は移行後のP型を指す。教師30本はmasterの標準ネットの学習に使った30ファイルである。
第2世代の教師は、tatara方式への移行時に新旧を学習させた37ファイルを指す。
候補は教師30本で約2.05エポック学んだ新型、基準は第2世代の教師で約2.07エポック学んだ新型で、同じ開始局面から先後を入れ替えた2局を1ペアとした。

日本中将棋連盟の規則で双方に1手3,000ノードを与え、シード 20261119490000 から開始局面を生成した。
Intel Core Ultra 7 265KFで7ペアを並行実行し、両側とも探索は1スレッド、置換表は32 MBとした。

| 対局側 | エンジンのコミット | 実行ファイルのSHA-256 | 評価関数のSHA-256 |
|---|---|---|---|
| 候補 | `8b853b66` | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `70fa0052c20c57ff3e0e5515a27c59b788f311a551238b90a259d39a809cf258` |
| 基準 | `8b853b66` | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `cd96df8bc54c21175fbfed0f04b962452265b90c7b63a422e510deeb89b8b4f0` |

全200ペアを集計した。
2局合計で候補の得点が順に0、0.5、1、1.5、2点だったペア数は `[17, 0, 95, 3, 85]` で、得点率は67.38%だった。
基準に対する候補の棋力差は+126.0 Elo、95%信頼区間は+92.7〜+161.7 Eloだった。
エンジン異常、時間切れ、着手の拒否はいずれも0件だった。実行時間は212.4秒だった。

## 再現手順

本体リポジトリのルートで測定した。対局ハーネスのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase match run --candidate commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --baseline commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --candidate-option EvalFile=data/tatara-p-repair-20261119/new-13/nnue.bin --baseline-option EvalFile=data/tatara-p-diagnosis-20261119/early-new/nnue.bin --each nodes=3000 --rules JCSA --candidate-hash 32 --baseline-hash 32 --concurrency 7 --seed 20261119490000 --run-dir data/tatara-p-repair-20261119/matches/two-epochs-teacher-new elo --pairs 200
```
