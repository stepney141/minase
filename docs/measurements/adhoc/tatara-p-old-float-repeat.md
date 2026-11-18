# 別の開始局面で旧型と実数計算の新型を比較した対局

## 目的

中将棋のP型評価関数について、第2世代の教師で学び直した旧型の棋力を実数で推論する新型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/old-float-repeat" --seed 20261119900000 \
  --candidate "$ROOT/target/match-cache/a48f03aa35cfa419353739ee001664543536dd71/minase --protocol usi" --baseline "$ROOT/data/tatara-p-diagnosis-20261119/probefloat --protocol usi" \
  --candidate-option "EvalFile=$ROOT/data/nnue-tatara-framework-and-halfkp-vm/train-before/nnue.bin" \
  --baseline-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/float.bin" \
  --candidate-hash 32 --baseline-hash 32 --rules JCSA --each nodes=3000 \
  --concurrency 3 elo --pairs 200
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | 第2世代の教師で学び直した旧型 | a48f03aa | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `fc86cce6b2f60c357061222fd54afdd62a5a88330b0a9207c93ec0119622ee71` |
| 基準 | 実数で推論する新型 | 8b853b66 の診断用ビルド | `8ce288cfa5ed0660545615a72d3aff974266b3638d3fbb739451c44df18bb14a` | `38e3c8a4523bddf723009e0cd2580961a6d42ce20137f03c7dffc903be6c3518` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で3ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補32 MB、基準32 MBとした。

## 結果

200ペアのうち有効は200ペア、破棄は0ペアだった。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[36, 1, 110, 1, 52]` だった。
棋力差は+27.9 Eloで、95%信頼区間は-4.0〜+60.2 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は540.2秒だった。

## 結論

区間は0 Eloを含み、この対局だけでは優劣を決められない。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
