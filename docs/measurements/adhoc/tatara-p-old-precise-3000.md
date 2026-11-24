# 旧型と後段を16ビットにした新型

## 目的

中将棋のA型評価関数について、第2世代の教師で学び直した旧型の棋力を後段を16ビットにした新型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/old-precise-3000" --seed 20261119000000 \
  --candidate "$ROOT/target/match-cache/a48f03aa35cfa419353739ee001664543536dd71/minase --protocol usi" --baseline "$ROOT/data/tatara-p-diagnosis-20261119/probe16 --protocol usi" \
  --candidate-option "EvalFile=$ROOT/data/nnue-tatara-framework-and-halfkp-vm/train-before/nnue.bin" \
  --baseline-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/precise.bin" \
  --candidate-hash 32 --baseline-hash 32 --rules JCSA --each nodes=3000 \
  --concurrency 3 elo --pairs 200
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | 第2世代の教師で学び直した旧型 | a48f03aa | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `fc86cce6b2f60c357061222fd54afdd62a5a88330b0a9207c93ec0119622ee71` |
| 基準 | 後段を16ビットにした新型 | 8b853b66 の診断用ビルド | `7fcda912f7fa80b7a6d5fd1532a54d6dd20c4936f14087fdf69815a6f941b36d` | `fa7cca164184981b9dfde21646462fbd880d8e168ea05f2b56c66a9c407a5e13` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で3ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補32 MB、基準32 MBとした。

## 結果

200ペアのうち有効は200ペア、破棄は0ペアだった。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[49, 0, 105, 1, 45]` だった。
棋力差は-6.1 Eloで、95%信頼区間は-39.3〜+27.0 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は604.5秒だった。

## 結論

区間は0 Eloを含み、この対局だけでは優劣を決められない。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
