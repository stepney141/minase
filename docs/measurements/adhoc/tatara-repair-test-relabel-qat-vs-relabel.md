# 終盤の教師値を付け直し、量子化を模擬して学習した新型と終盤の教師値を付け直して浮動小数点で学習した新型の対局

tatara方式への移行で生じたP型評価関数の棋力低下を調べるため、検証のための対局を行った。
旧型はtatara方式への移行前のP型、新型は移行後のP型を指す。教師30本はmasterの標準ネットの学習に使った30ファイルである。
候補は終盤の教師値を付け直し、量子化を模擬して学習した新型、基準は終盤の教師値を付け直して浮動小数点で学習した新型で、同じ開始局面から先後を入れ替えた2局を1ペアとした。
追加学習したモデルは、教師30本で約2.05エポック学んだ新型を出発点とし、訓練集合から抽出した262,144局面を2エポック学んだ。
終盤の教師値を付け直す条件では、駒が25枚以下の65,536局面をmaster標準ネットで各3,000ノード再探索した。

日本中将棋連盟の規則で双方に1手3,000ノードを与え、シード 20261119410000 から開始局面を生成した。
Intel Core Ultra 7 265KFで7ペアを並行実行し、両側とも探索は1スレッド、置換表は32 MBとした。

| 対局側 | エンジンのコミット | 実行ファイルのSHA-256 | 評価関数のSHA-256 |
|---|---|---|---|
| 候補 | `8b853b66` | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `1d8e4477de39e60b4c3c4b64fc83b423635b12905bd101d5842a19d99aa2bf49` |
| 基準 | `8b853b66` | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `3c771b7c52b3affae41cf469fe170de1c09339a4dd8fe7c6631b2c2afe0390f2` |

全200ペアを集計した。
2局合計で候補の得点が順に0、0.5、1、1.5、2点だったペア数は `[42, 4, 105, 2, 47]` で、得点率は51.00%だった。
基準に対する候補の棋力差は+6.9 Elo、95%信頼区間は-25.5〜+39.5 Eloだった。
エンジン異常、時間切れ、着手の拒否はいずれも0件だった。実行時間は249.1秒だった。

## 再現手順

本体リポジトリのルートで測定した。対局ハーネスのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase match run --candidate commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --baseline commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --candidate-option EvalFile=data/tatara-p-repair-20261119/relabel-qat/nnue.bin --baseline-option EvalFile=data/tatara-p-repair-20261119/relabel/nnue.bin --each nodes=3000 --rules JCSA --candidate-hash 32 --baseline-hash 32 --concurrency 7 --seed 20261119410000 --run-dir data/tatara-p-repair-20261119/matches/test-relabel-qat-vs-relabel elo --pairs 200
```
