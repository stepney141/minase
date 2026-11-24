# 同じ推論器で後段の精度を変えた短時間対局

## 目的

中将棋のA型評価関数について、後段を16ビットにした新型の棋力を8ビットの重みを整数倍して移した新型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/precise-control-stc" --seed 20261119200000 \
  --candidate "$ROOT/data/tatara-p-diagnosis-20261119/probe16 --protocol usi" --baseline "$ROOT/data/tatara-p-diagnosis-20261119/probe16 --protocol usi" \
  --candidate-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/precise.bin" \
  --baseline-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/control.bin" \
  --candidate-hash 256 --baseline-hash 256 --rules JCSA --each time=10000+100 \
  --concurrency 6 elo --pairs 240
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | 後段を16ビットにした新型 | 8b853b66 の診断用ビルド | `7fcda912f7fa80b7a6d5fd1532a54d6dd20c4936f14087fdf69815a6f941b36d` | `fa7cca164184981b9dfde21646462fbd880d8e168ea05f2b56c66a9c407a5e13` |
| 基準 | 8ビットの重みを整数倍して移した新型 | 8b853b66 の診断用ビルド | `7fcda912f7fa80b7a6d5fd1532a54d6dd20c4936f14087fdf69815a6f941b36d` | `2cd20f6b44f1c471226837bc29562b0e29a79ff6c2a5a1c6dbed8c1bdd4444ae` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で6ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補256 MB、基準256 MBとした。

## 結果

240ペアのうち有効は239ペア、1ペアは手数上限に達した局を含むため破棄した。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[44, 25, 112, 19, 39]` だった。
棋力差は-11.6 Eloで、95%信頼区間は-39.4〜+16.0 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
同じ診断用エンジン同士が対局し、別の固定ノード対局や学習、ビルドも並行した。保存された棋譜を `minase match report` で再集計し、Eloと異常件数を照合した。有効な478局で最初の32手の平均完了深さは候補17.28手、基準16.85手だった。累計実行時間は5,179.5秒だった。

## 結論

区間は0 Eloを含み、この対局だけでは優劣を決められない。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
