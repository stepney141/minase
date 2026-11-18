# 同数の対局を異なる教師から学ぶ比較（シード1）

## 目的

中将棋のP型評価関数について、標準ネットと同じ出所の教師で学んだ新型の棋力を第2世代の教師で学んだ新型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/corpus-1" --seed 20261119410000 \
  --candidate "$ROOT/target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase --protocol usi" --baseline "$ROOT/target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase --protocol usi" \
  --candidate-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/fit-corpus-master-1/nnue.bin" \
  --baseline-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/fit-corpus-gen2-1/nnue.bin" \
  --candidate-hash 32 --baseline-hash 32 --rules JCSA --each nodes=3000 \
  --concurrency 3 elo --pairs 200
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | 標準ネットと同じ出所の教師で学んだ新型 | 8b853b66 | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `aeb5e73bdd345bf1c3bc32bdaf5a198098bad91baeb7db620a124a72898fdc2c` |
| 基準 | 第2世代の教師で学んだ新型 | 8b853b66 | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `af3a6bdcee59a249d02aebacc8bf6b7a43b737355b79aad81344ae4290341330` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で3ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補32 MB、基準32 MBとした。

## 結果

200ペアのうち有効は200ペア、破棄は0ペアだった。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[37, 0, 91, 0, 72]` だった。
棋力差は+61.4 Eloで、95%信頼区間は+26.3〜+97.8 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は653.0秒だった。

## 結論

この条件では標準ネットと同じ出所の教師で学んだ新型の得点率が高かった。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
