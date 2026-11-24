# 終盤の勝敗を混ぜたA型と探索値だけを使ったA型の直接対局

候補は盤上の残り駒が25枚以下の局面だけ教師に終局得点を25%混ぜたモデル、基準は全局面の教師を探索勝率としたモデルである。教師30ファイルを2.05エポック学んだ重みから、両者に同じ262,144局面を同じ順で2エポック追加学習させた。両者とも駒種と升を特徴とするtatara方式のA型評価関数を使う。

2026年11月19日に日本中将棋連盟ルールで対局し、1手3,000ノード、探索1スレッド、置換表32 MBとした。シード20261119950000から開始局面を96通り生成し、先後を入れ替えた2局を1組とした。Intel Core Ultra 7 265KFで3組ずつ並行実行した。

候補の得点率は47.92%、棋力差は-14.5 Eloで、95%信頼区間は［-62.1, +32.6］Eloだった。候補の2局の合計が0、0.5、1、1.5、2点だった組数は順に23、3、49、1、20だった。全96組を集計した。エンジンの異常と時間切れはいずれも0件だった。実行時間は277.4秒だった。

エンジンと対局ハーネスにはコミット `8b853b6655f3c2acfa41677449d2bcc8509dc66c` の同じ実行ファイルを使った。以下の値はファイル全体のSHA-256である。

| ファイル | SHA-256 |
|---|---|
| 実行ファイル | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` |
| 候補の評価関数 | `7b59183215796c68f1ba8e721e3e3fe14a7b0f06a30687a93c21d13894defb49` |
| 基準の評価関数 | `adef4587501dcbaec9d63251a31aecb10262b00d0909a2fde2fa26ff5462f3a3` |

## 再現手順

本体リポジトリのルートで次を実行する。教師や重みの作成方法、開始局面ごとの対局結果は[過学習の調査](tatara-p-generalization.md)にある。

```sh
target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase match run --candidate commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --baseline commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --candidate-option EvalFile=data/tatara-p-generalization-20261119/endgame/nnue.bin --baseline-option EvalFile=data/tatara-p-repair-20261119/fp/nnue.bin --each nodes=3000 --candidate-hash 32 --baseline-hash 32 --rules JCSA --concurrency 3 --seed 20261119950000 --run-dir data/tatara-p-generalization-20261119/ending-match elo --pairs 96
```
