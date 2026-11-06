# 線形の経路の学習率の比較

## 目的

[標準レシピv1の設計書](../../plans/nnue-standard-recipe.md)の測定1として、線形の経路（駒と升ごとの点数の表、PSQT）の学習率を0.03から先例にそろえた0.67903へ変えて学んだネットが、0.03で学んだネットより強いかをSTCで判定する。
`H1`なら0.67903を標準レシピv1に採り、それ以外は0.03を保つ。

## コマンドライン

学習、補正、駒除去差分の診断、比較用のコミットの作成、およびSTCは、[スクリプト](nnue-standard-recipe-lr-linear-stc/run_lr_linear.py)で順に実行した。
教師は中断した系列の第2世代の37本（`data/nnue-generation-reset/gen-2/teacher/`）である。
学習の引数は、対照のネットを学んだときの`config.json`から読み、`--lr-linear 0.67903`だけを加えた。
対照の学習は[教師の量の比較](../nnue-generation-reset/nnue-generation-reset-gen2-volume.md)の全量の学習であり、途中検証は行っていない。

```console
minase-trainer nnue train --data <第2世代の37本> --base data/nnue-generation-reset/gen-1/fit/nnue.bin \
  --lambda 1.0 --teacher-cp-per-logit calibrate --seed 1 --threads 8 --output-dir <出力先>/train \
  --feature-set P --width 256 --head pairwise-l16-l32 --batch 16384 --epochs 50 --patience 5 \
  --device cuda --lr-schedule cosine:3:0.1 --lr-linear 0.67903
minase-trainer nnue fit-linear --base <出力先>/train/nnue.bin --data <第2世代の37本> \
  --tau 10 20 40 80 --seed 20261105 --threads 12 --output-dir <出力先>/fit
minase-trainer diag nnue-removal --float <学習の出力>/best.npz --net <補正後のネット> \
  --data <第2世代の37本> --sample 10000 --seed 1 --output <出力>
minase match run --run-dir data/matches/nnue-standard-recipe-lr-linear-stc --seed 20261300100000 \
  --candidate commit:d7467a7367dc3125797f7020fc30e3910d6cd2ff \
  --baseline commit:f60eb6913d45f4c057031c4964788e1e95ea2442 \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

`<出力先>`は`data/nnue-standard-recipe/recipe/lr-linear`である。
学習器は対照と同じ実行ファイル`data/nnue-generation-reset/bin/minase-trainer`を使い、学習のコマンドを`/usr/bin/time -v`経由で実行して最大常駐メモリを測った。
対局は設計書の作業ツリー`data/worktrees/nnue-standard-recipe`から実行し、候補と対照をその作業ツリーの`target/match-cache/`で新しくビルドした（[教訓](../../lessons/do-not-mix-builds-across-cargo-config-changes.md)）。

## エンジン

| 名前 | コミット | 内容 |
|---|---|---|
| 候補 | `d7467a73`（参照`refs/nnue-standard-recipe/lr-linear-fit`） | 学習率0.67903で学んで補正したネット |
| 候補の補正前 | `c71d30aa`（参照`refs/nnue-standard-recipe/lr-linear-trained`） | 同じ学習の補正前のネット。測定2で使う。 |
| 対照 | `f60eb691`（参照`refs/nnue-generation-reset/gen-2-volume-full`） | 学習率0.03で学んで補正したネット |

3つのコミットの親はいずれも中断した系列の第2世代のコミット`c5910ea5`で、違いは`crates/minase/nets/nnue.bin`だけである。
規則セットは`L0,P0,R1,E0`である。

## 環境

2026年11月6日に、作業機（Intel Core Ultra 7 265KF、20コア、メモリ30 GB、RTX 3060 Ti）で、学習の後にSTCを実行した。
対局はエンジンごとに1スレッド、`USI_Hash`は256 MB、同時対局数は16である。
STCの間、作業機ではlishogiのBotが動いていた。
STCの途中から、測定3の候補の学習（4スレッド、nice値19、メモリ上限5 GB）と、連続生成スクリプトの[試走](nnue-standard-recipe-chain-trial.md)の作業機側の処理（ビルドは4並列、nice値19）を並行させた。

## 結果

### 学習と補正

| 項目 | 候補（0.67903） | 対照（0.03） |
|---|---:|---:|
| 最良エポック / 停止エポック | 2 / 7 | 2 / 7 |
| 最良の検証損失 | 0.521351 | 0.521349 |
| 線形の経路の移動量（最良エポック、序中盤の平均 / 最大） | 8.33 cp / 95.96 cp | 0.50 cp / 10.18 cp |
| 線形の経路の移動量（最良エポック、終盤の平均 / 最大） | 10.38 cp / 154.94 cp | 0.71 cp / 7.60 cp |
| 量子化誤差の平均 / 最大 | 4.55 cp / 72.41 cp | 4.46 cp / 74.38 cp |
| D | −0.005685 | −0.005679 |
| Dの95%区間 | [−0.007107, −0.004312] | [−0.007093, −0.004309] |
| 選ばれたτ | 80 | 80 |
| 学習の時間 | 1,399秒 | 1,355秒 |

Dは補正ありの予測の損失から補正なしの損失を引いた値で、負なら補正が予測の損失を下げている。
線形の経路の移動量は、初期値（探索用駒価値から作った表）からの、左右の鏡映で共有した6,840行の変化の大きさである（`training.jsonl`の`linear_movement`）。
学習率を約23倍にすると線形の経路は約16倍動いたが、検証損失の差は0.000002だった。
候補の学習の最大常駐メモリは12.39 GB（12,392,844 kB）で、`/usr/bin/time`が測ったプロセス全体の経過時間は23分22秒だった。

補正の駒種ごとの平均は、候補と対照で最大2.1 cpしか違わなかった（候補は竪行+384.5、龍王+378.9、飛車+368.4、龍馬+314.1、獅子−281.1、仲人−50.6 cp）。
学習中に線形の経路が動いた量（平均約8〜10 cp）は、補正の量（駒種の平均で最大約385 cp）より2桁小さい。
補正の出力は[候補](nnue-standard-recipe-lr-linear-stc/fit_report.out)にあり、対照は[教師の量の比較の出力](../nnue-generation-reset/nnue-generation-reset-gen2-volume/fit_report-full.out)である。

駒除去差分の符号反転の割合は、第2世代の教師の検証集合の10,000局面（516,352件の除去）で次のとおりである。
値はいずれも`diag nnue-removal`の値で、標本の検査和（`99b59cbe…`）は4つのネットで同じである。候補の補正前は[スクリプト](nnue-standard-recipe-lr-linear-stc/diag_trained.py)でSTCの途中に測り、対照の補正前は[第2世代の学習の記録](../nnue-generation-reset/nnue-generation-reset-gen2-training.md)の値（同じネット`08b408d9…`）である。

| ネット | 全体 | 平均の大きさ | 歩兵 | 仲人 |
|---|---:|---:|---:|---:|
| 候補の補正前 | 2.15% | 9.4 cp | 10.6% | 0.1% |
| 候補 | 4.28% | 21.2 cp | 13.8% | 55.7% |
| 対照の補正前 | 1.99% | 8.9 cp | 9.8% | 0.2% |
| 対照 | 4.21% | 21.0 cp | 13.4% | 55.9% |

### STC

| 有効ペア | 破棄ペア | 度数 | LLR | 判定 | 得点率 | 得点率から求めたElo |
|---:|---:|---|---:|---|---:|---:|
| 1,607 | 24 | [326, 91, 790, 97, 303] | −2.9481 | `H0` | 49.4% | 約−4 |

度数は、各ペアの2局で候補が得た合計点が0、0.5、1、1.5、および2点だったペアの数を、この順に並べたものである。
所要時間は14,030秒で、エンジン異常、時間切れ、および拒否着手は0件だった。
LLRは626ペアで最大の+1.53に達した後に下がり、1,607ペアで下の境界に達して`H0`となった。

## 結論

線形の経路の学習率を0.67903にしても、補正後のネットは0.03のネットより強くならなかった（`H0`）ので、標準レシピv1は0.03を保つ。
学習率を上げると学習中に線形の経路は大きく動いたが、補正の後の駒種ごとの値は0.03の場合とほとんど変わらず、補正が学習中の動きを上書きしている。
