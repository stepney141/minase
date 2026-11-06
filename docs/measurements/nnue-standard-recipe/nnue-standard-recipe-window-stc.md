# 直前の世代の教師を併用する学習の比較

## 目的

[標準レシピv1の設計書](../../plans/nnue-standard-recipe.md)の測定3として、中断した系列の第1世代の教師26本と第2世代の教師37本の合わせて63本で学んだネット（窓2）が、第2世代の37本だけで学んだネット（窓1）より強いかをSTCで判定する。
`H1`なら標準レシピv1は窓2を採り、それ以外は窓1を保つ。
学習率は[測定1](nnue-standard-recipe-lr-linear-stc.md)で選んだ0.03とし、補正は[測定2](nnue-standard-recipe-fit-stc.md)の結論に従って行わない。

## コマンドライン

学習、補正、駒除去差分の診断、比較用のコミットの作成、およびSTCは、[スクリプト](nnue-standard-recipe-window-stc/run_window.py)で実行した。
学習の引数は測定1の対照と同じで、`--data`、`--threads`、および`--buckets`の3つだけが違う。
`--data`には第1世代の本0〜25、第2世代の本0〜36をこの順に与えた。

```console
minase-trainer nnue train --data <第1世代の26本> <第2世代の37本> --base data/nnue-generation-reset/gen-1/fit/nnue.bin \
  --lambda 1.0 --teacher-cp-per-logit calibrate --seed 1 --threads 4 --output-dir <出力先>/train \
  --feature-set P --width 256 --head pairwise-l16-l32 --batch 16384 --epochs 50 --patience 5 \
  --device cuda --lr-schedule cosine:3:0.1 --lr-linear 0.03 \
  --buckets 16 --bucket-positions 10474793 --scratch-dir <出力先>/scratch
minase-trainer nnue fit-linear --base <出力先>/train/nnue.bin --data <第2世代の37本> \
  --tau 10 20 40 80 --seed 20261105 --threads 4 --output-dir <出力先>/fit
minase-trainer diag nnue-removal --float <出力先>/train/best.npz --net <出力先>/train/nnue.bin \
  --data <第2世代の37本> --sample 10000 --seed 1 --output <出力>
minase match run --run-dir data/matches/nnue-standard-recipe-window-stc --seed 20261300300000 \
  --candidate commit:2f5f4b008b2e11b36ce52d3377bc6f287c833588 \
  --baseline commit:f29aa0c2a1b5a67a4b8d5522926d46ff4fe30569 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

`<出力先>`は`data/nnue-standard-recipe/recipe/window-lr0.03`である。
学習器は測定1と同じ実行ファイル`data/nnue-generation-reset/bin/minase-trainer`である。
63本の局面数は37本の約1.65倍で、測定1の学習の最大常駐メモリ12.39 GBから見込むと約20 GBになり、作業機の空きに収まらない。
そこで`--buckets`を指定して学んだ。
`--buckets`の学習も毎エポック訓練集合全体を一様に並べ替えるので（[ADR](../../adr/shuffle-the-whole-training-set-per-epoch-with-rao-sandelius-buckets.md)）、学習の分布は変わらない。
学習は測定1と測定2のSTCと並行させたので、STCの空きコア数に合わせて`--threads 4`、nice値19、メモリ上限5 GB（スワップなし）とし、bucket数は、1つあたり約1.5 GBとなる16とした。
`--bucket-positions`は入力の総局面数N=167,286,270（63本を`minase data selfplay inspect`で数えた学習に使える局面の和で、訓練局面と検証局面の和に等しい）から、N/16+6√(N/16)で求めた。
補正は棋力の比較には使わないが、Dと補正量を記録するために行い、第2世代の37本だけを与えた。
駒除去差分の診断はメモリ上限5 GBを超えたので、STCの前に上限20 GBで行った。
測定1の結論が出る前に、学習率0.67903でも同じ63本を同じ条件で学んだ（`data/nnue-standard-recipe/recipe/window-lr0.67903`、最良エポック3）。測定1が`H0`だったので、このネットは比較に使わない。

## エンジン

| 名前 | コミット | 内容 |
|---|---|---|
| 候補 | `2f5f4b00`（参照`refs/nnue-standard-recipe/window-trained`） | 63本で学んだ補正前のネット |
| 対照 | `f29aa0c2`（参照`refs/nnue-standard-recipe/lr-linear-0.03-trained`） | 第2世代の37本で学んだ補正前のネット（測定2の候補） |

2つのコミットの親はどちらも中断した系列の第2世代のコミット`c5910ea5`で、違いは`crates/minase/nets/nnue.bin`だけである。
規則セットは`L0,P0,R1,E0`である。

## 環境

学習は2026年11月7日の1時8分から3時11分に、STCはその後に、作業機（Intel Core Ultra 7 265KF、20コア、メモリ30 GB、RTX 3060 Ti）で実行した。
対局はエンジンごとに1スレッド、`USI_Hash`は256 MB、同時対局数は16である。
STCの間、作業機ではlishogiのBotが動いていた。

## 結果

### 学習と補正

| 項目 | 候補（63本） | 対照（37本） |
|---|---:|---:|
| 訓練局面 / 検証局面 | 158,810,877 / 8,475,393 | 96,462,343 / 5,162,867 |
| 教師の勝率尺度 | 第1世代の本670.91 cp、第2世代の本707.88 cp | 707.88 cp |
| 最良エポック / 停止エポック | 2 / 7 | 2 / 7 |
| 総更新回数 | 67,858 | 41,216 |
| 最良の検証損失 | 0.505129 | 0.521349 |
| 線形の経路の移動量（最良エポック、序中盤の平均 / 最大） | 0.82 cp / 17.31 cp | 0.50 cp / 10.18 cp |
| 量子化誤差の平均 / 最大 | 4.05 cp / 68.88 cp | 4.46 cp / 74.38 cp |
| D（記録のために行った補正の値） | −0.005614 | −0.005679 |
| Dの95%区間 | [−0.006979, −0.004334] | [−0.007093, −0.004309] |
| 学習の時間 | 6,257秒 | 1,355秒 |

検証集合は教師ごとに違うので、検証損失は比べない。
学習器は生成コミットの異なる第1世代の本（`6f874692`）と第2世代の本（`7c2cfe4e`）を別の分類とし、教師の勝率尺度をそれぞれ推定した。
候補の学習の最大常駐メモリは2.24 GBで、`/usr/bin/time`で計測したプロセス全体の経過時間は1時間44分21秒だった。
学習の時間は並行したSTCと4スレッドの制限を含むので、対照と比べられない。
補正の出力は[候補](nnue-standard-recipe-window-stc/fit_report.out)にある。
補正の駒種ごとの平均は対照とほとんど同じで、竪行+384.3、龍王+374.4、飛車+366.6、龍馬+310.0 cpだった。

駒除去差分の符号反転の割合は、第2世代の教師の検証集合の10,000局面（516,352件の除去）で次のとおりである。
標本の検査和（`99b59cbe…`）は測定1と測定2の診断と同じである。

| ネット | 全体 | 平均の大きさ | 歩兵 | 仲人 |
|---|---:|---:|---:|---:|
| 候補 | 1.27% | 10.4 cp | 6.1% | 1.4% |
| 対照 | 1.99% | 8.9 cp | 9.8% | 0.2% |

### STC

| 有効ペア | 破棄ペア | 度数 | LLR | 判定 | 得点率 | 得点率から求めたElo |
|---:|---:|---|---:|---|---:|---:|
| 149 | 1 | [49, 6, 81, 3, 10] | −2.9456 | `H0` | 36.4% | 約−97 |

度数は、各ペアの2局で候補が得た合計点が0、0.5、1、1.5、および2点だったペアの数を、この順に並べたものである。
所要時間は1,134秒で、エンジン異常、時間切れ、および拒否着手は0件だった。
逐次検定は判定が出た時点で止まるので、得点率から求めたEloは真の差より大きく出やすい。

## 結論

第1世代の教師を併用して学んだネットは、第2世代の教師だけで学んだネットより強くならず、得点率36.4%で大きく負けた（`H0`）ので、標準レシピv1は窓1を保つ。
併用した第1世代の教師は、手書きの評価関数v0が生成したものである。
v0より強いネットが生成した教師どうしを併用した場合の効果は、この測定からは分からない。
