# 約0.94エポックの重みから終盤の教師値を付け直し、量子化を模擬して学習した新型とmasterの標準ネットの対局

tatara方式への移行で生じたP型評価関数の棋力低下を調べるため、検証のための対局を行った。
旧型はtatara方式への移行前のP型、新型は移行後のP型を指す。教師30本はmasterの標準ネットの学習に使った30ファイルである。
候補は約0.94エポックの重みから終盤の教師値を付け直し、量子化を模擬して学習した新型、基準はmasterの標準ネットで、同じ開始局面から先後を入れ替えた2局を1ペアとした。
追加学習したモデルは、教師30本で約0.94エポック学んだ新型を出発点とし、訓練集合から抽出した262,144局面を2エポック学んだ。
終盤の教師値を付け直す条件では、駒が25枚以下の65,536局面をmaster標準ネットで各3,000ノード再探索した。

日本中将棋連盟の規則で双方に1手3,000ノードを与え、シード 20261119530000 から開始局面を生成した。
Intel Core Ultra 7 265KFで7ペアを並行実行し、両側とも探索は1スレッド、置換表は32 MBとした。

| 対局側 | エンジンのコミット | 実行ファイルのSHA-256 | 評価関数のSHA-256 |
|---|---|---|---|
| 候補 | `8b853b66` | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `bff2696610aadc680556d758cc705f0b4027efb055556f6cfebb9169cb8d81ee` |
| 基準 | `a48f03aa` | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd` |

160ペアのうち手数上限に達した対局を含む2ペアを除き、158ペアを集計した。
2局合計で候補の得点が順に0、0.5、1、1.5、2点だったペア数は `[52, 0, 87, 2, 17]` で、得点率は39.24%だった。
基準に対する候補の棋力差は-76.0 Elo、95%信頼区間は-112.5〜-41.0 Eloだった。
エンジン異常、時間切れ、着手の拒否はいずれも0件だった。実行時間は195.4秒だった。

## 再現手順

本体リポジトリのルートで測定した。対局ハーネスのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase match run --candidate commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --baseline commit:a48f03aa35cfa419353739ee001664543536dd71 --candidate-option EvalFile=data/tatara-p-repair-20261119/first-relabel-qat/nnue.bin --baseline-option EvalFile=/mnt/synology_nas/minase/nnue/nets/987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd.bin --each nodes=3000 --rules JCSA --candidate-hash 32 --baseline-hash 32 --concurrency 7 --seed 20261119530000 --run-dir data/tatara-p-repair-20261119/matches/first-relabel-qat-vs-standard elo --pairs 160
```
