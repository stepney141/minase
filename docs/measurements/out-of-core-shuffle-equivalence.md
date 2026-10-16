# 一様シャッフルの実装の前後でB=1の学習結果が一致することの照合記録

## 目的

[メモリより大きい教師データの一様シャッフルの設計書](../plans/archived/out-of-core-shuffle.md)では、学習器の読み込みを1局ずつの逐次処理に変え、NNUEの学習にB≥2のbucketによる一様シャッフルの経路を加えた。
B=1の経路は、変更前と同じ順列と同じ学習結果を出さなければならない（設計書の「検証」の1）。
本書は、変更前のmasterの学習器で記録した基準と、変更後の学習器で同じ入力に対して行った学習と診断の結果を照合した記録である。
本書は棋力測定の記録ではないので、ペンタノミアル度数やLLRは記録しない。

## 環境

測定機はIntel Core Ultra 7 265KF（20コア）、NVIDIA GeForce RTX 3060 Ti（8 GB）、メモリ30 GBである。
基準の記録と照合の間、同じ機械で別の棋力測定（同時対局数16のLTC）とlishogiのBotが動いていたので、学習と診断は`--threads 3`で行い、所要時間は本書では比べない。
NNUEの学習はGPUで、PST、FM、同時学習、および診断はCPUで行った。
変更前の学習器はmasterのコミット8539c4c0を、変更後の学習器はブランチ`out-of-core-shuffle`のコミット67779260を、いずれも`cargo build --release -p minase-trainer`でビルドしたものであり、実行ファイルのSHA-256の先頭は順に`7435b6c6`と`431d2b5b`である。
以下のコマンドでは、入力と出力を置いた`data/out-of-core-shuffle/`を`$D`、リポジトリのルートを`$ROOT`と書く。

## 入力

[学習時の局面の並べ方の予備測定](shuffle-order-preliminary.md)に使った版2の4ファイルは記録の後に削除していたので、同じ条件（1局面1,000ノード、7,500局、乱数開始の自己対局、同時対局数は3）で、コミット8539c4c0の`minase data selfplay generate`により生成し直した。
生成コミットが異なるので、ファイルの内容は予備測定の4ファイルとは一致しない。
PST、FM、同時学習、および診断の照合には、複数の教師の分類、λが0になる実戦対局、および先読みの有無を含めるため、[教師データ形式の統一の照合記録](unified-teacher-format-equivalence.md)と同じ構成の小さな4ファイル（乱数開始、乱数注入、実戦開始の自己対局と、lishogiの棋譜600局の取り込み）を1局面1,000ノードで生成した。

```sh
minase data selfplay generate --output $D/teacher/measure-<seed>.bin --games 7500 --seed <seed> \
  --nodes 1000 --random-moves 0 --concurrency 3 --hash-mb 16 --max-ply 1000   # seed = 4000000, 5000000, 6000000, 7000000
minase data lishogi openings --input $D/teacher/lishogi-sample.ndjson --output $D/teacher/openings.txt \
  --report $D/teacher/openings.report.json --nodes 1000 --concurrency 3 --hash-mb 16
minase data lishogi games --input $D/teacher/lishogi-sample.ndjson --output $D/teacher/lishogi.bin \
  --report $D/teacher/lishogi.report.json --nodes 1000 --concurrency 3 --hash-mb 16 --seed 7000001
minase data selfplay generate --output $D/teacher/random.bin --games 300 --seed 1000000 --nodes 1000 --random-moves 0 --concurrency 3 --hash-mb 16 --max-ply 1000
minase data selfplay generate --output $D/teacher/inject.bin --games 200 --seed 2000000 --nodes 1000 --random-moves 8 --concurrency 3 --hash-mb 16 --max-ply 1000
minase data selfplay generate --output $D/teacher/opening.bin --openings $D/teacher/openings.txt --seed 3000000 --nodes 1000 --random-moves 0 --concurrency 3 --hash-mb 16 --max-ply 1000
```

`lishogi-sample.ndjson`は`data/lishogi/games.ndjson`の先頭600行（SHA-256 `64fda8bc…2e02`）であり、開始局面の一覧は486局を保持した。

| ファイル | 対局数 | 項目数 | 学習器が読む局面 | バイト数 | SHA-256 |
|---|---|---|---|---|---|
| `measure-4000000.bin` | 7,465 | 2,635,508 | 2,060,908 | 19,650,573 | `e014a63c…` |
| `measure-5000000.bin` | 7,447 | 2,629,802 | 2,056,881 | 19,607,733 | `7284f773…` |
| `measure-6000000.bin` | 7,450 | 2,632,402 | 2,058,567 | 19,626,416 | `a9c460da…` |
| `measure-7000000.bin` | 7,456 | 2,626,869 | 2,054,039 | 19,588,651 | `4e96b931…` |
| `random.bin` | 297 | 102,669 | 80,228 | 766,652 | `905cc08e…` |
| `inject.bin` | 200 | 71,047 | 46,057 | 529,681 | `68f19afe…` |
| `opening.bin` | 482 | 134,475 | 103,335 | 1,019,079 | `53bdd8d3…` |
| `lishogi.bin` | 47 | 11,199 | 8,390 | 123,112 | `70970589…` |

4ファイルの学習の入力は合わせて8,230,395局面で、訓練集合が7,822,295局面、検証集合が408,100局面である。
小さな4ファイルの入力は238,010局面で、訓練集合が229,145局面、検証集合が8,865局面であり、教師の分類は乱数開始の自己対局（`random`と`inject`）、実戦開始の自己対局、実戦対局の3つである。

## 照合の条件

変更前後で同じ次のコマンドを、変更前は`$D/phase0/`、変更後は`$D/final-b1/`へ出力して実行した。
NNUEは2つの入力の特徴集合で2エポック、PSTは先読みあり（補助損失の参照は初期値）と先読みなし（参照はv0、シード2）の2条件、FMと同時学習は手引きの手順どおりに2エポックで学習し、診断はPSTの比較と、NNUEの3つの診断を実行した。

```sh
minase-trainer nnue train --feature-set <P または HalfKP-hm-b8> --width 128 --data $D/teacher/measure-*.bin \
  --init-pst $ROOT/data/pst-fm-joint-training/j75/pst-float.npz --base $ROOT/crates/minase/nets/pst.bin \
  --lambda 1.0 --teacher-cp-per-logit calibrate --lookahead-gamma 0.9 --lookahead-plies 40 \
  --batch 16384 --epochs 2 --seed 1 --threads 3 --device cuda --output-dir <出力先>
minase-trainer pst train --data <小さな4ファイル> --init $ROOT/data/pst-fm-joint-training/j75/pst.bin --output-dir <出力先> \
  --lr 0.03 --epochs 2 --patience 5 --batch 4096 --seed 1 --removal-penalty 1000 --removal-reference init \
  --lambda 0.75 --teacher-cp-per-logit calibrate --lookahead-gamma 0.9 --lookahead-plies 40 --threads 3
minase-trainer pst train --data <小さな4ファイル> --init $ROOT/data/pst-fm-joint-training/j75/pst.bin --output-dir <出力先> \
  --lr 0.03 --epochs 2 --patience 5 --batch 4096 --seed 2 --removal-penalty 1000 --removal-reference v0 \
  --lambda 0.75 --teacher-cp-per-logit calibrate --threads 3
minase-trainer fm train --data <小さな4ファイル> --init <PSTの出力>/pst.bin --output-dir <出力先> \
  --epochs 2 --patience 3 --lambda 1.0 --teacher-cp-per-logit calibrate --lookahead-gamma 0.9 --lookahead-plies 40 --threads 3
minase-trainer fm quarter --input <FMの出力>/fm.bin --output <出力先>/fm-quarter.bin
minase-trainer joint train --data <小さな4ファイル> --base <出力先>/fm-quarter.bin --pst-init <PSTの出力>/pst-float.npz \
  --fm-init <FMの出力>/fm-float.npz --fm-reference <FMの出力>/fm.bin --output-dir <出力先> \
  --epochs 2 --patience 5 --lambda 0.75 --teacher-cp-per-logit calibrate --lookahead-gamma 0.9 --lookahead-plies 40 --threads 3
minase-trainer fm scale --input $ROOT/data/pst-fm-joint-training/j75/pst.bin --float $ROOT/data/pst-fm-joint-training/j75/pst-float.npz \
  --fm-scale 0 --output <出力先>/pst-base.bin --data <小さな4ファイル>
minase-trainer diag pst-compare --data <小さな4ファイル> --lambda 0.75 --teacher-cp-per-logit calibrate --lookahead-gamma 0.9 --lookahead-plies 40 \
  --base <出力先>/pst-base.bin --candidate <PSTの出力>/pst.bin --candidate-float <PSTの出力>/pst-float.npz --output-dir <出力先> --sample-size 10000 --seed 1
minase-trainer diag nnue-removal --float <NNUE Pの出力>/best.npz --net <NNUE Pの出力>/nnue.bin --data $D/teacher/measure-*.bin --sample 10000 --seed 1 --output <報告>
minase-trainer diag nnue-distribution --pair base $ROOT/crates/minase/nets/pst.bin <NNUE Pの出力>/nnue.bin --data <小さな4ファイル> --sample 5000 --seed 1 --output <報告>
minase-trainer diag nnue-moves --net <NNUE Pの出力>/nnue.bin --base $ROOT/crates/minase/nets/pst.bin --positions $D/teacher/lishogi.bin --output <報告>
```

照合する値は、使い捨てのスクリプトで出力から抜き出した。
各学習の`config.json`の`preprocessing`（局面数、教師の分類、教師の勝率尺度、訓練と検証の通し番号の列のSHA-256、観測数のSHA-256）、`training.jsonl`の各エポックの損失と統計、`summary.json`、書き出した重み（`nnue.bin`、`pst.bin`、`fm.bin`、`pst-float.npz`、`fm-float.npz`）のSHA-256、チェックポイント`best.npz`と`latest.npz`の`meta`以外の全配列のSHA-256、および診断の報告と付随ファイル（標本、代表局面、駒を除いた局面）である。
経過時間、スレッド数、GPUのメモリ、出力先のパス、引数の一覧、および実行ファイルの版の欄は比較から除いた。
浮動小数点の値はJSONに書かれた最短の10進表現のまま比べた。

## 基準（変更前）

| 学習 | 訓練局面 | 教師の勝率尺度（分類順） | 検証損失（エポック0、1、2） | 書き出した重みのSHA-256 |
|---|---|---|---|---|
| NNUE、P | 7,822,295 | 2000.73997460196 | 0.568566220937345、0.5368826676978432、0.536676699950834 | `ff581375…` |
| NNUE、HalfKP-hm-b8 | 7,822,295 | 2000.73997460196 | 0.568566220937345、0.537597398098705、0.5375488022552427 | `dc6cb68d…` |
| PST（先読みあり） | 229,145 | 2257.6687870305573、2028.841203308032、推定なし | 0.5620318742207471、0.5617148631974233、0.5614693620557376 | `d79e25e8…` |
| PST（先読みなし） | 229,145 | 2292.6267357100455、2104.476792097284、推定なし | 0.5722830737227533、0.5719846806672348、0.5717588451354809 | — |
| FM | 229,145 | 2257.6687870305573、2028.841203308032、推定なし | 0.5812846676384875、0.5807905881903817、0.5797951814421396 | `d4d2d9b3…` |
| 同時学習 | 229,145 | 2257.6687870305573、2028.841203308032、推定なし | 0.5610121595691713、0.5614939577622491、0.5622069033665366 | `3443bb1d…` |

NNUEの訓練と検証の通し番号の列のSHA-256は`2cdd100f…`と`5b0014c9…`、量子化の検査の標本（10,000局面）のSHA-256は`90d79f6e…`であり、量子化誤差の平均はPで17.10 cp、HalfKP-hm-b8で16.30 cp、符号反転の割合は順に0.0413と0.0622だった。
実戦対局の分類はλが0なので教師の勝率尺度を推定しない。
抜き出した全欄の値（約293 KB）は、照合の作業データとして`$D/phase0/values.json`に保存した。

## 照合（変更後）

変更後の学習器で同じコマンドを実行し、同じスクリプトで抜き出した値を比べると、異なる欄は1つだけだった。
`diag nnue-removal`の報告が`--float`の入力の識別として記録する`best.npz`のバイト長が、変更前の10,758,842バイトから変更後の10,758,854バイトへ12バイト増えていた。
`best.npz`の`meta`に収める設定（`settings`）に、新しい引数`buckets`の値が加わったためである。
`best.npz`と`latest.npz`の`meta`を除く全配列のSHA-256は一致しており、差分は入力の識別の欄だけで、学習結果の数値には影響しない。
それ以外の値、すなわちNNUEの2つの特徴集合、PSTの2条件、FM、同時学習の局面数、教師の勝率尺度、通し番号と観測数のSHA-256、各エポックの損失と統計、終了報告、書き出した重みのSHA-256、チェックポイントの配列、およびPSTの比較診断とNNUEの3つの診断の報告と付随ファイルは、すべてバイト単位で一致した。

## 結論

逐次読み込みへの変更とB≥2の経路の追加の後も、`--buckets 1`（既定）の学習器は、NNUE、PST、FM、同時学習、および診断のすべてで変更前とビット単位で同じ結果を出す。
以上により、設計書の「検証」の1が満たされていることを確認した。
