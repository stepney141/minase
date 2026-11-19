# masterの標準ネットと同じ教師で学び直した旧型

## 目的

中将棋のP型評価関数について、masterの標準ネットの棋力を同じ教師で学び直した旧型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `e7eea73dd6e89d9f5e2ca7710144f4ad335acc682b719f44ec10595c9890cb51` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/standard-master-vs-retrained" --seed 20261119080000 \
  --candidate commit:a48f03aa35cfa419353739ee001664543536dd71 --baseline commit:a48f03aa35cfa419353739ee001664543536dd71 \
  --candidate-option "EvalFile=/mnt/synology_nas/minase/nnue/nets/987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd.bin" \
  --baseline-option "EvalFile=/mnt/synology_nas/minase/nnue/nets/fc86cce6b2f60c357061222fd54afdd62a5a88330b0a9207c93ec0119622ee71.bin" \
  --candidate-hash 16 --baseline-hash 16 --rules JCSA --each nodes=3000 \
  --concurrency 3 elo --pairs 200
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | masterの標準ネット | a48f03aa | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `987f060d545c09f0a42b4c668ea8b24cbf83c1770d11d663a94e4b82601274cd` |
| 基準 | 同じ教師で学び直した旧型 | a48f03aa | `9aee18e42ed2b2a8f17d980e90152e83a94cab99a7e4ba5952bd5e803a333237` | `fc86cce6b2f60c357061222fd54afdd62a5a88330b0a9207c93ec0119622ee71` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で3ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補16 MB、基準16 MBとした。

## 結果

200ペアのうち有効は200ペア、破棄は0ペアだった。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[0, 0, 40, 1, 159]` だった。
棋力差は+379.3 Eloで、95%信頼区間は+331.8〜+440.2 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は550.8秒だった。

## 結論

この条件ではmasterの標準ネットの得点率が高かった。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
