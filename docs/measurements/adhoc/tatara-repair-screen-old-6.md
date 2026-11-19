# 教師30本で約0.94エポック学んだ旧型とmasterの標準ネットの対局

tatara方式への移行で生じたP型評価関数の棋力低下を調べるため、候補選びのための対局を行った。
旧型はtatara方式への移行前のP型、新型は移行後のP型を指す。教師30本はmasterの標準ネットの学習に使った30ファイルである。
候補は教師30本で約0.94エポック学んだ旧型、基準はmasterの標準ネットで、同じ開始局面から先後を入れ替えた2局を1ペアとした。

日本中将棋連盟の規則で双方に1手3,000ノードを与え、シード 20261119400000 から開始局面を生成した。
Intel Core Ultra 7 265KFで7ペアを並行実行し、両側とも探索は1スレッド、置換表は32 MBとした。

| 対局側 | エンジンのコミット | 実行ファイルのSHA-256 | 評価関数のSHA-256 |
|---|---|---|---|
| 候補 | `a48f03aa` | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `ad68aa62c0fb8b5b35067dfa6591a5fccbceaf26fd1a4bab54a0462fcbb8cfde` |
| 基準 | `a48f03aa` | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd` |

全96ペアを集計した。
2局合計で候補の得点が順に0、0.5、1、1.5、2点だったペア数は `[39, 2, 45, 0, 10]` で、得点率は34.38%だった。
基準に対する候補の棋力差は-112.3 Elo、95%信頼区間は-164.9〜-64.4 Eloだった。
エンジン異常、時間切れ、着手の拒否はいずれも0件だった。実行時間は110.0秒だった。

## 再現手順

本体リポジトリのルートで測定した。対局ハーネスのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase match run --candidate commit:a48f03aa35cfa419353739ee001664543536dd71 --baseline commit:a48f03aa35cfa419353739ee001664543536dd71 --candidate-option EvalFile=data/tatara-p-repair-20261119/old-6/nnue.bin --baseline-option EvalFile=/mnt/synology_nas/minase/nnue/nets/987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd.bin --each nodes=3000 --rules JCSA --candidate-hash 32 --baseline-hash 32 --concurrency 7 --seed 20261119400000 --run-dir data/tatara-p-repair-20261119/matches/screen-old-6 elo --pairs 96
```
