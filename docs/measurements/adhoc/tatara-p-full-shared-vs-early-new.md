# 全教師で幅16の層の学習率を半減した対局

## 目的

中将棋のA型評価関数について、幅16の層の学習率を半減した2回目の新型の棋力を通常の2回目の新型との直接対局で測った。本文中の新型はtataraのLayerStackへ移行した評価関数を指す。この実験の学習では6,104更新を1 superbatchとし、2回目は約2.07エポック、64回目は約66.3エポックに当たる。下表の候補と基準を同じ開始局面から先後を入れ替えて対局させ、2局を1ペアとして候補側から見た棋力差を推定した。

## コマンドライン

`ROOT` は本体リポジトリのルートを指す。実行時の設定は測定ディレクトリの `manifest.json` にある。対局ハーネスの版は2.6.0で、ハーネスの実行ファイルのSHA-256は `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` である。

```sh
minase match run \
  --run-dir "$ROOT/data/tatara-p-diagnosis-20261119/full-shared-vs-early-new" --seed 20261119500000 \
  --candidate "$ROOT/target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase --protocol usi" --baseline "$ROOT/target/match-cache/8b853b6655f3c2acfa41677449d2bcc8509dc66c/minase --protocol usi" \
  --candidate-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/fit-full-shared/nnue.bin" \
  --baseline-option "EvalFile=$ROOT/data/tatara-p-diagnosis-20261119/early-new/nnue.bin" \
  --candidate-hash 32 --baseline-hash 32 --rules JCSA --each nodes=3000 \
  --concurrency 5 elo --pairs 200
```

## エンジン

日本中将棋連盟の規則セット `JCSA` を使った。

| 側 | ネット | エンジンのコミット | 実行ファイルの SHA-256 | 重みファイルの SHA-256 |
|---|---|---|---|---|
| 候補 | 幅16の層の学習率を半減した2回目の新型 | 8b853b66 | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `c3b97311821b9e18e15bf611f3aed01c52eb1aa39fa152f8d36fcc8d43e99ada` |
| 基準 | 通常の2回目の新型 | 8b853b66 | `b6066b1dee6eea6d8d6d3d1740778e7a0e5c4b387e22486bd8ec3a065c32897d` | `cd96df8bc54c21175fbfed0f04b962452265b90c7b63a422e510deeb89b8b4f0` |

## 環境

Intel Core Ultra 7 265KF（物理20コア、論理20コア）で5ペアを並行して測った。
探索は双方とも1スレッドで、置換表は候補32 MB、基準32 MBとした。

## 結果

200ペアのうち有効は199ペア、1ペアは手数上限に達した局を含むため破棄した。
候補から見たペアの得点が0、0.5、1、1.5、2点だった度数は順に `[47, 5, 97, 2, 48]` だった。
棋力差は-0.9 Eloで、95%信頼区間は-34.6〜+32.9 Eloだった。
不正着手、異常終了、応答のタイムアウト、時間切れ、着手の拒否はすべて0件だった。
累計実行時間は321.6秒だった。

## 結論

区間は0 Eloを含み、この対局だけでは優劣を決められない。
関連する比較と診断用エンジンの構成は[棋力低下の調査](../../audits/tatara-p-regression-2026-11-19.md)にまとめた。
