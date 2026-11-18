# 新型の後段の量子化精度

## 目的

中将棋のP型評価関数について、後段を16ビットにした新型の棋力を8ビットの新型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/precise-3000" --seed 20261119000000 \
  --candidate "$ROOT/data/tatara-p-diagnosis-20261119/probe16 --protocol usi" --baseline "$ROOT/target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase --protocol usi" \
  --candidate-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/precise.bin" \
  --baseline-option "EvalFile=$ROOT/data/nnue-tatara-framework-and-halfkp-vm/train-after/nnue.bin" \
  --candidate-hash 32 --baseline-hash 32 --rules JCSA --each nodes=3000 \
  --concurrency 5 elo --pairs 400
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | 後段を16ビットにした新型 | 8b853b66 の診断用ビルド | `7fcda912f7fa80b7a6d5fd1532a54d6dd20c4936f14087fdf69815a6f941b36d` | `fa7cca164184981b9dfde21646462fbd880d8e168ea05f2b56c66a9c407a5e13` |
| 基準 | 8ビットの新型 | 8b853b66 | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `37054c9e49c9218bb18efdd1814fe1b298de5cbfc9c14352f6ad6eaecbb70dea` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で5ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補32 MB、基準32 MBとした。

## 結果

400ペアのうち有効は400ペア、破棄は0ペアだった。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[92, 11, 208, 6, 83]` だった。
棋力差は-10.0 Eloで、95%信頼区間は-32.9〜+12.8 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は778.7秒だった。

## 結論

区間は0 Eloを含み、この対局だけでは優劣を決められない。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
