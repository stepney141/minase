# 余弦減衰の期間を変えたA型の直接対局

候補は余弦減衰の期間を2,560更新に設定したモデル、基準は減衰期間を390,656更新に設定したモデルである。どちらも教師2ファイルの14,048局から採った同じ1,296,697局面を使い、初期学習率0.001、下限倍率0.027で2,560回更新した。両者とも駒種と升を特徴とするtatara方式のA型評価関数を使う。

2026年11月19日に日本中将棋連盟ルールで対局し、1手3,000ノード、探索1スレッド、置換表32 MBとした。シード20261119950000から開始局面を96通り生成し、先後を入れ替えた2局を1組とした。Intel Core Ultra 7 265KFで3組ずつ並行実行した。

候補の得点率は54.43%、棋力差は+30.8 Eloで、95%信頼区間は［-14.9, +77.7］Eloだった。候補の2局の合計が0、0.5、1、1.5、2点だった組数は順に17、0、53、1、25だった。全96組を集計した。エンジンの異常と時間切れはいずれも0件だった。実行時間は258.6秒だった。

エンジンと対局ハーネスにはコミット `8b853b6655f3c2acfa41677449d2bcc8509dc66c` の同じ実行ファイルを使った。以下の値はファイル全体のSHA-256である。

| ファイル | SHA-256 |
|---|---|
| 実行ファイル | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` |
| 候補の評価関数 | `5faf586b394be95d99bf7c77de78f93a40779ff24a53e4e3cd80ce3074a5914d` |
| 基準の評価関数 | `894e2aaec705a13c3dac84e0dc11d03fa85fc123b945095d5e5cc86bd201e117` |

## 再現手順

本体リポジトリのルートで次を実行する。教師や重みの作成方法、開始局面ごとの対局結果は[過学習の調査](tatara-p-generalization.md)にある。

```sh
target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase match run --candidate commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --baseline commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c --candidate-option EvalFile=data/tatara-p-generalization-20261119/short-cosine/nnue.bin --baseline-option EvalFile=data/tatara-p-generalization-20261119/matched-positions/nnue.bin --each nodes=3000 --candidate-hash 32 --baseline-hash 32 --rules JCSA --concurrency 3 --seed 20261119950000 --run-dir data/tatara-p-generalization-20261119/schedule-match elo --pairs 96
```
