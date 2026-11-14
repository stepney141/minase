# 標準レシピv1の学習器と教師の生成器の試走

## 目的

[標準レシピv1の設計書](../../plans/nnue-standard-recipe.md)のフェーズ5として、フェーズ3で改めた学習器（学習の長さをsuperbatchで指定し、最後のネットを書き出し、勝率の差の2乗を損失とする）と、フェーズ4で改めた教師の生成器（王手の局面の除外、開始局面の選別、評価値による裁定、MNSDの版3、規則セット`lishogi`）が、生成から学習、診断、およびコミットまで通ることを確かめる。
確かめる点は次の5つである。

- 作業機で生成した教師ファイルと、それで学んだネットのヘッダの規則セットが`lishogi`（`L1,L2,P0,P3,R1,E1,E3`）であり、教師データの形式が版3である。
- 生成器が、開始局面を捨て、評価値で対局を裁定し、王手の局面を除外する。
- 連続生成スクリプト`scripts/vast/generation_chain.py`が、生成機での生成、学習、診断、およびコミットまでを新しい引数で通す。
- 生成機での学習が、`--superbatches`と`--validation-superbatches`を受け取り、`--teacher-cp-per-logit`を渡されずに初期値のネットの勝率尺度を使い、診断に`latest.npz`が渡される。
- 生成機の削除をインスタンスの一覧で確かめ、課金が止まる。

## 第0世代のコミットと実行ファイル

本書のブランチ`nnue-standard-recipe`のコミット`31d8944f`を、リモートを外した使い捨てのcloneに取り出した。
そのcloneで、改めた学習器の`minase-trainer nnue export-handcrafted --cp-per-logit 1000`で手書きの評価関数v0を書き出し、`crates/minase/nets/nnue.bin`に埋め込んで第0世代のコミット`866379ef`を作った。
勝率尺度1,000 cpは仮の値であり、探索の評価値には影響しない。
書き出したネットの重みのチェックサムは`74afe0e5…`であり、規則セットのほかは中断した系列の第0世代と同じ重みである。
作業機で本を検査する`minase`と、第1世代の勝率尺度の推定に使う`minase-trainer`は、このコミットからビルドした。

## 作業機での生成と学習

作業機はIntel Core Ultra 7 265KF（20コア、メモリ30 GB、RTX 3060 Ti）である。
第0世代のコミットで、20局を1局面500ノードで生成した。

```console
minase data selfplay generate --output-dir local-gen --chunk-games 10 --seed 9700000000 --games 20 \
  --nodes 500 --random-moves 0 --concurrency 16 --max-ply 4000 --hash-mb 16
```

`minase data selfplay inspect`の出力は、版3、規則セット`L1,L2,P0,P3,R1,E1,E3`、生成コミット`866379ef`だった。
20局のうち、評価値による裁定で終えた対局は9局、手数の上限で破棄した対局は0局で、捨てた開始局面は2個だった。
除外した局面は、詰みの帯95、王手146、戦術的な局面1,202、反復の局面118であり、記録した局面は4,011だった。

この教師で、書き出したv0を初期値として短く学んだ。

```console
minase-trainer nnue train --data local-gen/teacher.mnsd --base v0-lishogi.bin --output-dir local-train \
  --lambda 1.0 --feature-set P --width 256 --batch 1024 --superbatches 1 --batches-per-superbatch 50 \
  --validation-superbatches 1 --lr-schedule cosine:1:0.027 --seed 1 --device cuda --threads 4
```

学習は`candidate`で終わり、50回の更新を完了した。
`config.json`の教師の勝率尺度の出どころは`base`、値は1,000 cpだった。
書き出した`nnue.bin`のヘッダは規則セット`L1,L2,P0,P3,R1,E1,E3`を持ち、`L0,P0,R1,E0`を含まなかった。
同じ初期値で中断した系列の教師ファイル（版2）を与えた学習は、版の不一致で始まらなかった。
`lishogi`の初期値が`JCSA`の版3の教師を拒むことは、単体テストで確かめている。

## 生成機での1世代

連続生成スクリプトは、`--worktree`に使い捨てのclone、`--ref-prefix`に試走専用の`refs/nnue-standard-recipe-trial-p5`を指定し、`systemd-run --user`（nice値19、メモリ上限8 GB）で起動した。
本番と値を変えた引数は、`--books-per-generation 2`、`--games 40`、`--chunk-games 20`、`--nodes 500`、`--seed-base 9600000000`、`--calibration-books 1`、`--batch 1024`、`--superbatches 1`、`--validation-superbatches 1`、`--lr-schedule cosine:1:0.027`、`--train-threads 8`、`--removal-sample 100`、および生成機の選定条件と待ち時間である。
`--replay-buffer-generations 1`、`--fit no`、`--lr-linear 0.03`、`--train-place generator`は標準レシピv1と同じ値にした。

```console
python3 scripts/vast/generation_chain.py run --run-dir <実行ディレクトリ> --worktree <clone> \
  --minase-bin <cloneのminase> --trainer-bin <cloneのminase-trainer> \
  --ref-prefix refs/nnue-standard-recipe-trial-p5 --gen0-commit 866379ef23ebcfbba7d4c97fad619138ff533665 \
  --first-generation 1 --last-generation 1 --books-per-generation 2 \
  --games 40 --chunk-games 20 --nodes 500 --random-moves 0 --max-ply 4000 --hash-mb 16 \
  --seed-base 9600000000 --seed-generation-step 10000000 --seed-book-step 100000 \
  --calibration-books 1 --lambda 1.0 --feature-set P --width 256 --head pairwise-l16-l32 --batch 1024 \
  --superbatches 1 --validation-superbatches 1 --lr-schedule cosine:1:0.027 \
  --train-seed 1 --device cuda --lr-linear 0.03 --train-threads 8 --train-place generator \
  --replay-buffer-generations 1 --fit no --removal-sample 100 --removal-seed 1 --bench-depth 5 \
  --image rust:1.98.0-bookworm --disk 30 --max-price 0.15 --min-cpu-cores 24 --cpu-pattern . \
  --min-duration-days 1 --min-reliability 0.95 --min-inet-up 100 --min-disk 30 \
  --book-minutes 10 --deadline-margin 1.3 --training-wait-hours 0.5 --idle-limit-hours 0.5 \
  --poll-seconds 15 --book-stall-minutes 30 --ssh-failure-minutes 5 --offer 53170012
```

生成機はvast.aiの出品53170012（AMD Ryzen 9 3900X、24スレッド、RTX 3060 Ti、フランス、1時間0.097米ドル）である。
実行は2026年11月14日の23時43分から23時52分で、約9分だった。
クレジットは61.9500米ドルから61.9336米ドルへ減ったので、試走の費用は約0.02米ドルである。

| 確かめた条件 | 結果 |
|---|---|
| 本のヘッダ | 本0と本1（各40局）は、版3、規則セット`L1,L2,P0,P3,R1,E1,E3`、生成コミット`866379ef`、チェックサム`74afe0e5…`、およびシード9,610,000,000と9,610,100,000を持っていた。 |
| 生成器の3つの処理 | 捨てた開始局面は本0で7個、本1で5個だった。評価値による裁定で終えた対局は13局と19局、王手で除いた局面は448と380だった。手数の上限で破棄した対局は0局だった。 |
| 生成機での学習 | 学習の`--data`は第1世代の本2本だけで、`--superbatches 1`と`--validation-superbatches 1`が設定に記録され、`--teacher-cp-per-logit`は渡されなかった。教師の勝率尺度の出どころは`base`で、値は本0から推定した勝率尺度2,156.77 cpだった。重み減衰は第1層と出力部とも0だった。学習は`candidate`で終わり、予定の6,104回の更新を完了した。 |
| 診断 | 駒除去差分の診断の`--float`には`train/latest.npz`が渡された。 |
| コミット | 第1世代のコミット`f87c4127`の本文は「教師2本、16421局面、1 superbatch（6104回の更新）」であり、埋め込んだネットは`gen-1/train/nnue.bin`とバイト単位で一致した。 |
| 参照の名前空間 | 使い捨てのcloneに作られた参照は`refs/nnue-standard-recipe-trial-p5/gen-0`と`gen-1`の2つだけだった。 |
| 生成機の削除 | 連続生成スクリプトは「完了」を表示して終わり、`vastai show instances`の一覧は空だった。 |

少ない局と短い学習で試したので、ネットの値や損失に意味はない。
1局面500ノードの教師から推定した勝率尺度2,156.77 cpは、本番の2万ノードの教師の値の目安にならない。

## 結論

改めた学習器、教師の生成器、および連続生成スクリプトは、作業機と生成機の両方で、生成から学習、診断、およびコミットまで新しい引数と規則セット`lishogi`で通った。
フェーズ5の完了条件を満たした。
