# masterの標準ネットと新型

## 目的

中将棋のA型評価関数について、masterの標準ネットの棋力を新型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `e7eea73dd6e89d9f5e2ca7710144f4ad335acc682b719f44ec10595c9890cb51` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/standard-master-vs-new" --seed 20261119080000 \
  --candidate commit:a48f03aa35cfa419353739ee001664543536dd71 --baseline commit:8b853b6655f3c2acfa41677449d2bcc8509dc66c \
  --candidate-option "EvalFile=/mnt/synology_nas/minase/nnue/nets/987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd.bin" \
  --baseline-option "EvalFile=/mnt/synology_nas/minase/nnue/nets/37054c9e49c9218bb18efdd1814fe1b298de5cbfc9c14352f6ad6eaecbb70dea.bin" \
  --candidate-hash 16 --baseline-hash 16 --rules JCSA --each nodes=3000 \
  --concurrency 3 elo --pairs 200
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | masterの標準ネット | a48f03aa | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd` |
| 基準 | 新型 | 8b853b66 | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `37054c9e49c9218bb18efdd1814fe1b298de5cbfc9c14352f6ad6eaecbb70dea` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で3ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補16 MB、基準16 MBとした。

## 結果

200ペアのうち有効は200ペア、破棄は0ペアだった。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[0, 0, 49, 0, 151]` だった。
棋力差は+342.0 Eloで、95%信頼区間は+298.2〜+396.3 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は401.0秒だった。

## 結論

この条件ではmasterの標準ネットの得点率が高かった。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
