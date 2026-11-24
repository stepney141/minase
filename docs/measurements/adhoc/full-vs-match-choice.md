# 教師30ファイルの検証損失で選んだA型と対局で選んだA型の直接対局

候補は検証損失が最小だった2.83エポックのモデル、基準は標準ネットとの対局の得点率で選んだ2.05エポックのモデルである。同じ教師30ファイルを同じ学習率と局面順序で学び、それぞれの時点で重みを保存した。両者とも駒種と升を特徴とするtatara方式のA型評価関数を使う。

2026年11月19日に日本中将棋連盟ルールで対局し、1手3,000ノード、探索1スレッド、置換表32 MBとした。シード20261119950000から開始局面を96通り生成し、先後を入れ替えた2局を1組とした。Intel Core Ultra 7 265KFで3組ずつ並行実行した。

候補の得点率は49.22%、棋力差は-5.4 Eloで、95%信頼区間は［-54.9, +43.8］Eloだった。候補の2局の合計が0、0.5、1、1.5、2点だった組数は順に24、2、46、1、23だった。全96組を集計した。エンジンの異常と時間切れはいずれも0件だった。実行時間は263.2秒だった。

エンジンと対局ハーネスにはコミット `8b853b6655f3c2acfa41677449d2bcc8509dc66c` の同じ実行ファイルを使った。以下の値はファイル全体のSHA-256である。

| ファイル | SHA-256 |
|---|---|
| 実行ファイル | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` |
| 候補の評価関数 | `f065a8fcf651bc62282f3f0fa1ca62f81e358d682d23a1b918e72f5d41234767` |
| 基準の評価関数 | `70fa0052c20c57ff3e0e5515a27c59b788f311a551238b90a259d39a809cf258` |

## 再現手順

本体リポジトリのルートで次を実行する。教師や重みの作成方法、開始局面ごとの対局結果は[過学習の調査](tatara-p-generalization.md)にある。

```sh
target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase match run --candidate commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --baseline commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --candidate-option EvalFile=data/tatara-p-generalization-20261119/full-selected/nnue.bin --baseline-option EvalFile=data/tatara-p-repair-20261119/new-13/nnue.bin --each nodes=3000 --candidate-hash 32 --baseline-hash 32 --rules JCSA --concurrency 3 --seed 20261119950000 --run-dir data/tatara-p-generalization-20261119/full-vs-match-choice elo --pairs 96
```
