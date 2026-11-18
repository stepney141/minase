# 後段の第1層の学習率を半減した新型の縮小実験（シード2）

## 目的

中将棋のP型評価関数について、後段の第1層の学習率を半減した新型の棋力を同じシードで学んだ新型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/match-shared-2" --seed 20261119120000 \
  --candidate "$ROOT/target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase --protocol usi" --baseline "$ROOT/target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase --protocol usi" \
  --candidate-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/fit-shared-2/nnue.bin" \
  --baseline-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/fit-production-2/nnue.bin" \
  --candidate-hash 32 --baseline-hash 32 --rules JCSA --each nodes=3000 \
  --concurrency 3 elo --pairs 160
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | 後段の第1層の学習率を半減した新型 | 8b853b66 | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `996cca7e47e3041388a5aaa37e2710e0a6849676e194a72fb5da4eb04b5248ee` |
| 基準 | 同じシードで学んだ新型 | 8b853b66 | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `c7bda0e9e8ac17aeaad7da09912545b9becb680a254062f8bde72c2dffdc83f6` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で3ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補32 MB、基準32 MBとした。

## 結果

160ペアのうち有効は159ペア、1ペアは手数上限に達した局を含むため破棄した。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[32, 0, 75, 3, 49]` だった。
棋力差は+40.6 Eloで、95%信頼区間は+2.2〜+80.0 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
実行スクリプトの変更と終了処理が重なり、最終集計の標準出力が上書きされた。保存された全ペアの棋譜から、対局ハーネスと同じ式で得点、信頼区間、終了理由を再集計した。累計実行時間は548.6秒だった。

## 結論

この条件では後段の第1層の学習率を半減した新型の得点率が高かった。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
