# 入力層の重みを実数に戻した対局

## 目的

中将棋のA型評価関数について、入力層の重みを実数に戻した新型の棋力を整数化した重みで途中の切り捨てを除いた新型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/ft-vs-smooth" --seed 20261119600000 \
  --candidate "$ROOT/data/tatara-p-diagnosis-20261119/probefloat --protocol usi" --baseline "$ROOT/data/tatara-p-diagnosis-20261119/probefloat --protocol usi" \
  --candidate-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/ft.bin" \
  --baseline-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/smooth.bin" \
  --candidate-hash 32 --baseline-hash 32 --rules JCSA --each nodes=3000 \
  --concurrency 4 elo --pairs 240
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | 入力層の重みを実数に戻した新型 | 8b853b66 の診断用ビルド | `8ce288cfa5ed0660545615a72d3aff974266b3638d3fbb739451c44df18bb14a` | `3e49f5a559ab9c09855959a736bb21806aafb5d9f90ea21133842483751ce466` |
| 基準 | 整数化した重みで途中の切り捨てを除いた新型 | 8b853b66 の診断用ビルド | `8ce288cfa5ed0660545615a72d3aff974266b3638d3fbb739451c44df18bb14a` | `61836353fdee07f3140d7df8f89b9c7866af85ac0d0434ce7646b12c52b20d87` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で4ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補32 MB、基準32 MBとした。

## 結果

240ペアのうち有効は240ペア、破棄は0ペアだった。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[43, 4, 129, 5, 59]` だった。
棋力差は+23.9 Eloで、95%信頼区間は-4.9〜+53.1 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は688.5秒だった。

## 結論

区間は0 Eloを含み、この対局だけでは優劣を決められない。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
