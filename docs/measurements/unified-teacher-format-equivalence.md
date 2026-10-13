# 教師データ形式の統一の前後で教師と学習結果が一致することの照合記録

## 目的

[教師データ形式の統一の設計書](../plans/unified-teacher-format.md)は、教師データMNSDを局面ごとの版1から対局単位の版2へ置き換える。
この変更は、生成器が書き出す内容と学習器の読み方を変えるだけであり、探索も評価関数も変えない。
したがって、同じ条件で生成した版2のファイルから学習器が取り出す局面の列は、版1のファイルのレコードの列と順序まで一致し、その局面で学習した結果もビット単位で一致しなければならない。
本書は、変更前の生成器と学習器で取った基準と、変更後の生成器と学習器で同じ条件の生成と学習を行った結果を記録し、両者を照合する。

## 環境

測定機はIntel Core Ultra 7 265KF（20コア）とNVIDIA GeForce RTX 3060 Tiであり、基準の取得中に他の測定は走らせていない。
NNUEの学習はGPU、PSTの学習と比較診断はCPUで行った。
変更前の生成器と学習器は、着手時点のコミット1977052（masterの551f3e0に設計書の状態の更新だけを加えたもの）を`cargo build --release -p minase -p minase-trainer`でビルドしたものであり、実行ファイルのSHA-256は次のとおりである。

| 実行ファイル | SHA-256 |
|---|---|
| `minase` | `b516d5c08c7d18319d13367d0b90ab5e9036fb7aee83af851203dde5a289f8a5` |
| `minase-trainer` | `a26d4fc1c2db8a89ac549bd2ae17a9e348774d01328a1b184dc0afb741e2d317` |

基準の実行ファイル、生成したファイル、および学習の出力は、照合が終わるまで`data/unified-teacher-format/phase0/`に保存する。
以下のコマンドでは、このディレクトリを`$P0`、リポジトリのルートを`$ROOT`と書く。

## 照合の条件

教師データは、自己対局の3条件とlishogiの棋譜の取り込みの、計4ファイルを生成した。
自己対局はいずれも1局面5,000ノード、手数の上限1,000、同時対局数16、置換表16 MBであり、規則セットはエンジンの既定の`L0,P0,R1,E0`、教師の評価関数は埋め込みのNNUE（検査和`3684a5ef…8c32`）である。
lishogiの棋譜は、`data/lishogi/games.ndjson`の先頭600行を`$P0/lishogi-sample.ndjson`（SHA-256 `64fda8bc…2e02`）として切り出して使った。
実戦開始の自己対局の開始局面一覧`$P0/openings.txt`（488行、SHA-256 `2d0ff6f9…952c`）は、同じ標本から`data lishogi openings`で作った。

```sh
minase data lishogi openings --input $P0/lishogi-sample.ndjson --output $P0/openings.txt \
  --report $P0/openings.report.json --nodes 5000 --concurrency 16 --hash-mb 16
minase data lishogi games --input $P0/lishogi-sample.ndjson --output $P0/lishogi.bin \
  --report $P0/lishogi.report.json --nodes 5000 --concurrency 16 --hash-mb 16 --seed 7000001
minase data selfplay generate --output $P0/random.bin --games 300 --seed 1000000 \
  --nodes 5000 --random-moves 0 --concurrency 16 --hash-mb 16 --max-ply 1000
minase data selfplay generate --output $P0/inject.bin --games 200 --seed 2000000 \
  --nodes 5000 --random-moves 8 --concurrency 16 --hash-mb 16 --max-ply 1000
minase data selfplay generate --output $P0/opening.bin --openings $P0/openings.txt --seed 3000000 \
  --nodes 5000 --random-moves 0 --concurrency 16 --hash-mb 16 --max-ply 1000
```

学習は、4ファイルを`random`、`inject`、`opening`、`lishogi`の順に`--data`へ与え、NNUEの2つの入力の特徴集合とPSTをそれぞれ2エポック行った。
PSTの比較診断では、同時学習J75の重みからFMを除いた`$P0/pst-base.bin`（`fm scale --fm-scale 0`で作成、SHA-256 `3c2c033f…3dd8`）を基準、PSTの学習の出力を候補にした。

```sh
minase-trainer nnue train --feature-set <P または HalfKP-hm-b8> --width 128 \
  --data $P0/random.bin $P0/inject.bin $P0/opening.bin $P0/lishogi.bin \
  --init-pst $ROOT/data/pst-fm-joint-training/j75/pst-float.npz --base $ROOT/crates/minase/nets/pst.bin \
  --lambda 1.0 --lookahead-gamma 0.9 --lookahead-plies 40 --batch 4096 --epochs 2 --seed 1 \
  --output-dir <出力先>
minase-trainer pst train --data <同じ4ファイル> --init $ROOT/data/pst-fm-joint-training/j75/pst.bin \
  --output-dir <出力先> --lr 0.03 --epochs 2 --patience 5 --batch 4096 --seed 1 \
  --removal-penalty 1000 --removal-reference init --lambda 0.75 --lookahead-gamma 0.9 --lookahead-plies 40
minase-trainer diag pst-compare --data <同じ4ファイル> --lambda 0.75 --lookahead-gamma 0.9 --lookahead-plies 40 \
  --base $P0/pst-base.bin --candidate <PSTの学習の出力>/pst.bin --candidate-float <PSTの学習の出力>/pst-float.npz \
  --output-dir <出力先> --sample-size 10000 --seed 1
```

照合する値は2種類である。
1つ目は教師データの局面の列であり、版1は`minase data selfplay inspect --dump`が書き出す全レコードの局面（SFEN）、手番、先獅子の升、麒麟成りの印、探索値、結果、対局番号、手数、および由来を、版2は除外の理由も価値がない印もない局面の同じ欄を比べる。
2つ目は学習の結果であり、`config.json`の`preprocessing`（局面数、教師用K、訓練と検証の通し番号の列のSHA-256、教師の分類。分類の生成コミットは版によって必ず異なるので除く）、`training.jsonl`の各エポックの損失と標本数、`summary.json`の時間以外の欄、書き出した重みファイルのSHA-256、およびPSTの比較診断の`report.json`（入力の識別と生成コミットを除く）と標本の通し番号のファイルを比べる。
2つ目の値の抽出には`data/unified-teacher-format/summarize_training.py`を使い、浮動小数点の値はJSONに書かれた最短の10進表現のまま比べる。

## 変更前の基準

生成した4ファイルは次のとおりである。
自己対局の各条件では手数の上限に達した対局があり、版1の生成器はその対局を書かない。

| ファイル | 対局数 | 上限で破棄した対局 | 記録数 | バイト数 | SHA-256 |
|---|---|---|---|---|---|
| `random.bin` | 300 | 3 | 95,524 | 15,283,976 | `1f6a6440…a07a` |
| `inject.bin` | 200 | 3 | 54,883 | 8,781,416 | `7d422e4a…8c0e` |
| `opening.bin` | 488 | 10 | 115,452 | 18,472,456 | `f1669220…6aa0` |
| `lishogi.bin` | — | — | 8,423 | 1,347,816 | `37b22e3d…ae5a` |

自己対局の生成器がログに出した除外の理由ごとの件数は、破棄した対局の局面を含めて次のとおりである。

| ファイル | 詰みの帯 | 戦術的な局面 | 反復の局面 |
|---|---|---|---|
| `random.bin` | 854 | 20,912 | 80 |
| `inject.bin` | 570 | 13,176 | 98 |
| `opening.bin` | 1,368 | 31,524 | 199 |

学習の入力は合わせて274,282局面であり、訓練集合が263,735局面、検証集合が10,547局面である。
訓練と検証の通し番号の列のSHA-256は、それぞれ`a1bfd1c6…0b88`と`845e524d…f4db`である。
教師の分類は、乱数開始の自己対局（`random`と`inject`）、実戦開始の自己対局、実戦対局の3つである。
教師用Kは、NNUEとPSTのどちらでも前の2分類で1576.9591549051001と1281.118458338129であり、実戦対局はλが0なので推定しない。

| 学習 | 書き出した重みのSHA-256 | 検証損失（エポック0、1、2） |
|---|---|---|
| NNUE、P | `9324b93d…cb9fe` | 0.5121930923093332、0.5076595624334446、0.5084699097038211 |
| NNUE、HalfKP-hm-b8 | `5ff797d8…a954e` | 0.5121930923093332、0.5104286893583697、0.5134635908749254 |
| PST | `d99c8429…58280` | 0.5029170934140164、0.5027091754306101、0.5025355164263542 |

PSTの比較診断の`report.json`から入力の識別と生成コミットを除いた内容のSHA-256は`4a99ff87…dfd7`である。
全欄の値は`$P0/baseline-summary.json`に保存した。

同じ実行ファイルで生成と学習をもう1回ずつ行うと、生成した4ファイルはバイト単位で一致し、学習と比較診断の照合値も`baseline-summary.json`と完全に一致した。
したがって、基準は同じ条件で再現でき、変更後に照合値が異なれば、その差は形式の変更による。

## 変更後の照合

（フェーズ2の完了後に記録する。）
