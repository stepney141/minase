# 標準ネットで探索用の駒価値を交換した対局

## 目的

中将棋のA型評価関数について、masterの標準ネットの棋力をv0の探索用駒価値を移した標準ネットとの直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/standard-with-v0-table" --seed 20261119080000 \
  --candidate commit:a48f03aa35cfa419353739ee001664543536dd71 --baseline commit:a48f03aa35cfa419353739ee001664543536dd71 \
  --candidate-option "EvalFile=/mnt/synology_nas/minase/nnue/nets/987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd.bin" \
  --baseline-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/games-analysis/../standard-with-v0-table.bin" \
  --candidate-hash 16 --baseline-hash 16 --rules JCSA --each nodes=3000 \
  --concurrency 2 elo --pairs 200
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | masterの標準ネット | a48f03aa | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd` |
| 基準 | v0の探索用駒価値を移した標準ネット | a48f03aa | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `d37a576746adb6183151e309ec7a89f52a4dd00d5d274b780855eb8c076b1167` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で2ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補16 MB、基準16 MBとした。

## 結果

200ペアのうち有効は198ペア、2ペアは手数上限に達した局を含むため破棄した。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[35, 5, 108, 6, 44]` だった。
棋力差は+16.7 Eloで、95%信頼区間は-14.3〜+48.0 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は1,143.7秒だった。

## 結論

区間は0 Eloを含み、この対局だけでは優劣を決められない。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
