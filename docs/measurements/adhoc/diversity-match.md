# 教師2ファイルから対局または局面を4分の1に絞ったP型の直接対局

候補は14,048局の適格局面を約4分の1に絞って1,324,956局面を学んだモデル、基準は対局を約4分の1の3,425局に絞り、適格な1,296,697局面をすべて学んだモデルである。同じ乱数シードで初期化し、それぞれ2,560回更新した。両者とも駒種と升を特徴とするtatara方式のP型評価関数を使う。

2026年11月19日に日本中将棋連盟ルールで対局し、1手3,000ノード、探索1スレッド、置換表32 MBとした。シード20261119950000から開始局面を96通り生成し、先後を入れ替えた2局を1組とした。Intel Core Ultra 7 265KFで3組ずつ並行実行した。

候補の得点率は66.93%、棋力差は+122.5 Eloで、95%信頼区間は［+73.1, +177.1］Eloだった。候補の2局の合計が0、0.5、1、1.5、2点だった組数は順に10、0、43、1、42だった。全96組を集計した。エンジンの異常と時間切れはいずれも0件だった。実行時間は223.2秒だった。

エンジンと対局ハーネスにはコミット `8b853b6655f3c2acfa41677449d2bcc8509dc66c` の同じ実行ファイルを使った。以下の値はファイル全体のSHA-256である。

| ファイル | SHA-256 |
|---|---|
| 実行ファイル | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` |
| 候補の評価関数 | `ef646cfc51b8fd6af646d729eb5d913ce0b8f78ae0899b3cf31efd24ae407e78` |
| 基準の評価関数 | `c6898601cfc1f9f09ee51033619f7865d40fad1fcecf9c1fb97ea8033af06743` |

## 再現手順

本体リポジトリのルートで次を実行する。教師や重みの作成方法、開始局面ごとの対局結果は[過学習の調査](tatara-p-generalization.md)にある。

```sh
target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase match run --candidate commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --baseline commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --candidate-option EvalFile=data/tatara-p-generalization-20261119/positions/nnue.bin --baseline-option EvalFile=data/tatara-p-generalization-20261119/games/nnue.bin --each nodes=3000 --candidate-hash 32 --baseline-hash 32 --rules JCSA --concurrency 3 --seed 20261119950000 --run-dir data/tatara-p-generalization-20261119/diversity-match elo --pairs 96
```
