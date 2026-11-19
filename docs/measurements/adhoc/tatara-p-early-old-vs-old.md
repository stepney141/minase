# 全教師で旧型の学習期間を変えた対局

## 目的

中将棋のP型評価関数について、2回目のsuperbatchの旧型の棋力を64回目の旧型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。この実験の学習では6,104更新を1 superbatchとし、2回目は約2.07エポック、64回目は約66.3エポックに当たる。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/early-old-vs-old" --seed 20261119500000 \
  --candidate "$ROOT/target/match-cache/a48f03aa35cfa419353739ee001664543536dd71/minase --protocol usi" --baseline "$ROOT/target/match-cache/a48f03aa35cfa419353739ee001664543536dd71/minase --protocol usi" \
  --candidate-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/early-old/nnue.bin" \
  --baseline-option "EvalFile=$ROOT/data/nnue-tatara-framework-and-halfkp-vm/train-before/nnue.bin" \
  --candidate-hash 32 --baseline-hash 32 --rules JCSA --each nodes=3000 \
  --concurrency 3 elo --pairs 200
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | 2回目のsuperbatchの旧型 | a48f03aa | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `bf7a986252b880959139856e273517041e20f2a0fb8099acf0a9112081652f09` |
| 基準 | 64回目の旧型 | a48f03aa | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `fc86cce6b2f60c357061222fd54afdd62a5a88330b0a9207c93ec0119622ee71` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で3ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補32 MB、基準32 MBとした。

## 結果

200ペアのうち有効は200ペア、破棄は0ペアだった。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[18, 1, 91, 2, 88]` だった。
棋力差は+128.0 Eloで、95%信頼区間は+93.9〜+164.6 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は495.6秒だった。

## 結論

この条件では2回目のsuperbatchの旧型の得点率が高かった。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
