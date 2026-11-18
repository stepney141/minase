# 実数評価と後段16ビット評価

## 目的

中将棋のP型評価関数について、実数で推論する新型の棋力を後段を16ビットにした新型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/float-precise-3000" --seed 20261119000000 \
  --candidate "$ROOT/data/tatara-p-diagnosis-20261119/probefloat --protocol usi" --baseline "$ROOT/data/tatara-p-diagnosis-20261119/probe16 --protocol usi" \
  --candidate-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/float.bin" \
  --baseline-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/precise.bin" \
  --candidate-hash 32 --baseline-hash 32 --rules JCSA --each nodes=3000 \
  --concurrency 4 elo --pairs 320
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | 実数で推論する新型 | 8b853b66 の診断用ビルド | `8ce288cfa5ed0660545615a72d3aff974266b3638d3fbb739451c44df18bb14a` | `38e3c8a4523bddf723009e0cd2580961a6d42ce20137f03c7dffc903be6c3518` |
| 基準 | 後段を16ビットにした新型 | 8b853b66 の診断用ビルド | `7fcda912f7fa80b7a6d5fd1532a54d6dd20c4936f14087fdf69815a6f941b36d` | `fa7cca164184981b9dfde21646462fbd880d8e168ea05f2b56c66a9c407a5e13` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で4ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補32 MB、基準32 MBとした。

## 結果

320ペアのうち有効は320ペア、破棄は0ペアだった。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[69, 11, 177, 7, 56]` だった。
棋力差は-16.3 Eloで、95%信頼区間は-40.6〜+7.9 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は1,059.3秒だった。

## 結論

区間は0 Eloを含み、この対局だけでは優劣を決められない。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
