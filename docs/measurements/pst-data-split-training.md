# データ半分割の2本のPSTの学習と診断

## 目的

[世代3の計画](../plans/pst-gen3.md)のデータ半分割の診断で、採用工程の訓練データを対局単位で2つに分け、それぞれで学習したPST（AとB）を得る。
あわせて、学習後の診断と、判定の前提条件（AとBの検証損失の差の絶対値が2×10⁻⁴以下）を確かめる。

## コマンドライン

学習は `data/pst-data-split/train_halves.sh` で行った。
このスクリプトは[学習シードの散らばりの測定](pst-seed-spread-nodes.md)の学習スクリプトと同じ引数で `train_pst.py train` を呼び、乱数シードを1に固定して `--train-half` だけを0と1に変える。

```console
TRAINER_DIR=$PWD/data/worktrees/trainer-d524871 OUT=$PWD/data/pst-data-split \
  OMP_NUM_THREADS=2 OPENBLAS_NUM_THREADS=2 MKL_NUM_THREADS=2 \
  data/pst-data-split/train_halves.sh 0 1
```

学習後の診断は、各候補について次のとおり実行した（`<h>` は0または1、`<11ファイル>` は採用工程の入力）。

```console
tools/train/.venv/bin/python tools/train/pst/pst_diagnostics.py \
  --data <11ファイル> --lookahead-gamma 0.9 --lookahead-plies 40 \
  --base data/strength-stage9/lookahead-pst-training/pst-base.bin \
  --candidate data/pst-data-split/half<h>/pst.bin \
  --probe data/gen3/probe/target/release/pst_probe \
  --output-dir data/pst-data-split/half<h>/diagnostics --sample-size 10000 --seed 1
```

## エンジン

学習器と診断器は、ブランチ`pst-gen3`のコミット`d524871`に固定したworktree `data/worktrees/trainer-d524871` のものである。
このコミットの学習器は、半分割オプションを省略すると採用PSTを再現する（[再現の記録](pst-gen3-trainer-reproduction.md)、オプション追加後もテストで固定）。
診断に使った `pst_probe` は、世代3の生成の準備でmaster `44ae7cb` からビルドしたものである。
AとBの重みは、ブランチ`pst-data-split`のコミット`1c9bf35`（A）と`348bb5c`（B）に、`44ae7cb` の `nets/pst.bin` を差し替える形で置いた。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB、GeForce RTX 3060 Ti）で、2026年10月2日12時01分から12時22分に学習した。
同じ時間帯に世代3の生成（同時16対局）が走っていた。

## 結果

| 項目 | A（訓練半分0） | B（訓練半分1） |
|---|---:|---:|
| 訓練局面数 | 11,964,664 | 11,914,947 |
| 総更新回数 | 7,310 | 7,280 |
| 最良エポック | 10 | 10 |
| 最良エポックの検証損失 | 0.473761760 | 0.473752686 |
| 重みのSHA-256 | `2d7fbb8d…` | `174758fb…` |

検証の分割（1,241,072局面）、教師K（世代0から順に768.45、1068.21、758.35）、および初期状態の検証損失0.474202923は、AとBで同じだった。
S0（全訓練データで学習した採用PST）の検証損失は0.473404901であり、AとBはどちらもS0より約3.5×10⁻⁴高い。
AとBの検証損失の差の絶対値は9.1×10⁻⁶であり、前提条件（2×10⁻⁴以下）を満たす。

学習後の診断は次のとおりである。

| 検査 | A | B |
|---|---|---|
| Rustと学習器の評価値の一致 | 141,841局面で一致 | 141,841局面で一致 |
| 量子化の前後の平均絶対誤差 | 0.52 cp（最大1.99 cp） | 0.54 cp（最大2.34 cp） |
| 駒除去差分の符号の反転 | 0件（352件を検査） | 0件（352件を検査） |
| 探索用駒価値と出力K | 不変 | 不変 |

浮動小数点の重みの差は次のとおりである。

| 比較 | 序中盤の平均差（最大） | 終盤の平均差（最大） |
|---|---:|---:|
| A対B | 4.88 cp（59.07 cp） | 7.05 cp（118.67 cp） |
| A対S0 | 5.38 cp | 10.23 cp |
| B対S0 | 5.44 cp | 10.04 cp |

A対Bの差は、学習シードだけを変えた場合（平均0.06 cpと0.08 cp）の約80倍であり、[混合比λ=1.0の再学習](teacher-mixing-ratio-100-stc.md)が基点から動いた量（平均89 cpと139 cp）の約20分の1である。

## 結論

AとBは学習後の診断を通過し、検証損失の差は前提条件を満たした。
訓練データを対局単位で分けると、シードの違いよりはるかに大きく重みが変わることを確かめたので、両者を[設計書](../plans/pst-gen3.md)の対局測定にかける。
