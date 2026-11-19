# 教師30ファイルの2.83エポックと8.02エポックを別の384組で比べた直接対局

候補は検証損失が最小だった2.83エポックのモデル、基準は8.02エポックのモデルである。両者とも駒種と升を特徴とするtatara方式のP型である。先に96組で比較したときの2つの重みをそのまま使い、開始局面のシードを替え、測定前に組数を384と定めた。[最初の96組](full-vs-late.md)

2026年11月19日に日本中将棋連盟ルールで対局し、1手3,000ノード、探索1スレッド、置換表32 MBとした。シード20261119990000から開始局面を384通り生成し、先後を入れ替えた2局を1組とした。Intel Core Ultra 7 265KFで5組ずつ並行実行した。

手数上限に達した対局を含む1組を除き、383組を集計した。候補の得点率は57.38%、棋力差は+51.6 Eloで、95%信頼区間は［+28.2, +75.5］Eloだった。候補の2局の合計が0、0.5、1、1.5、2点だった組数は順に59、4、202、1、117だった。エンジンの異常と時間切れはいずれも0件だった。実行時間は1,069.0秒だった。

エンジンと対局ハーネスにはコミット `8b853b6655f3c2acfa41677449d2bcc8509dc66c` の同じ実行ファイルを使った。以下の値はファイル全体のSHA-256である。

| ファイル | SHA-256 |
|---|---|
| 実行ファイル | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` |
| 候補の評価関数 | `f065a8fcf651bc62282f3f0fa1ca62f81e358d682d23a1b918e72f5d41234767` |
| 基準の評価関数 | `8b68eaabed8ea29e0a6c0eead808c0822846198600bcc49fa7bf397d6284afe7` |

## 再現手順

本体リポジトリのルートで次を実行する。教師や重みの作成方法、開始局面ごとの対局結果は[過学習の調査](tatara-p-generalization.md)にある。

```sh
target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase match run --candidate commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --baseline commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --candidate-option EvalFile=data/tatara-p-generalization-20261119/full-selected/nnue.bin --baseline-option EvalFile=data/tatara-p-repair-20261119/new-51/nnue.bin --each nodes=3000 --candidate-hash 32 --baseline-hash 32 --rules JCSA --concurrency 5 --seed 20261119990000 --run-dir data/tatara-p-generalization-20261119/full-vs-late-confirm elo --pairs 384
```
