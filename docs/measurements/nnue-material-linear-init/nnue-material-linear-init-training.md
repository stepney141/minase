# 線形経路を探索用駒価値で初期化したNNUEの学習

## 目的

[線形経路を駒価値で初期化する計画](../../plans/archived/nnue-material-linear-init.md)のフェーズ1、3、および4として、線形経路の初期値に学習済みPSTではなく探索用駒価値を置く学習器を使い、現行のネットと同じ教師データ、引数、およびシードでネットを学習して、学習の経過と標準の3診断を現行のネットと比べる。
あわせて、初期値のほかに学習の条件が変わっていないことを、計画が定めた3つの検査で確かめる。

## コマンドライン

学習はいずれも本体のリポジトリのルートで、`systemd-run`のscope（nice値19、メモリ上限13 GB）として実行した。
`$F`は`data/nnue-lr-schedule-gen/generated-*.bin`の30ファイルを名前順に並べた配列である。

第1の検査（回帰検査）では、変更前の学習器に現行のネットの学習と同じ引数を与えて実行し、最初の途中検証（1,000更新目）が記録された時点で止めた。

```console
systemd-run --user --scope --quiet --unit=nmli-regression -p MemoryMax=13G -p MemorySwapMax=0 \
  nice -n 19 /usr/bin/time -v -o data/nnue-material-linear-init/regression.time \
  data/nnue-material-linear-init/minase-trainer-master nnue train --feature-set P --width 128 \
  --data $F \
  --init-pst data/pst-fm-joint-training/j75/pst-float.npz \
  --base crates/minase/nets/pst.bin --lambda 1.0 \
  --batch 16384 --epochs 50 --patience 5 --seed 1 --threads 4 --device cuda \
  --lr-schedule cosine:3:0.1 --teacher-cp-per-logit calibrate --validation-interval 1000 \
  --output-dir data/nnue-material-linear-init/regression
```

本学習は、変更後の学習器で、上のコマンドから`--init-pst`を外して実行した。

```console
systemd-run --user --scope --quiet --unit=nmli-train -p MemoryMax=13G -p MemorySwapMax=0 \
  nice -n 19 /usr/bin/time -v -o data/nnue-material-linear-init/train.time \
  data/nnue-material-linear-init/minase-trainer-material nnue train --feature-set P --width 128 \
  --data $F \
  --base crates/minase/nets/pst.bin --lambda 1.0 \
  --batch 16384 --epochs 50 --patience 5 --seed 1 --threads 4 --device cuda \
  --lr-schedule cosine:3:0.1 --teacher-cp-per-logit calibrate --validation-interval 1000 \
  --output-dir data/nnue-material-linear-init/train
```

診断は[学習の手引き](../../guides/nnue-training.md)の8節の3つのコマンドで、[現行のネットの学習の記録](../nnue-early-best-epoch/nnue-no-lookahead-training.md)と同じ標本の条件で行った。
評価値の分布では、現行のネットと新しいネットを同じ標本で比べた。

```console
D=data/nnue-material-linear-init
$D/minase-trainer-material diag nnue-distribution \
  --pair current crates/minase/nets/pst.bin data/nnue-early-best-epoch/no-lookahead/nnue.bin \
  --pair material crates/minase/nets/pst.bin $D/train/nnue.bin \
  --data $F --sample 20000 --seed 1 --output $D/diag/distribution.json
$D/minase-trainer-material diag nnue-removal --float $D/train/best.npz --net $D/train/nnue.bin \
  --data $F --sample 10000 --seed 1 --output $D/diag/removal.json
$D/minase-trainer-material diag nnue-moves --net $D/train/nnue.bin --base crates/minase/nets/pst.bin \
  --positions data/out-of-core-shuffle/teacher/lishogi.bin --output $D/diag/moves.json
```

非線形部分の初期値の比較では、回帰検査と本学習がそれぞれ学習前の検証の直後に保存したエポック0の`latest.npz`を複製し、線形経路の配列`weights.linear.weight`を除く`weights.*`の全配列をバイト列で比べた。

## 学習器

| 用途 | 学習器のコミット | 実行ファイルのSHA-256 |
|---|---|---|
| 回帰検査 | `b8bebfb3`（変更前のmaster） | `e4b34a746f06cff35de2664199330a093efc26f10af8068597f190f47e81e785` |
| 本学習と診断 | `7e720567`（`--init-pst`を削除し、駒価値で初期化する） | `e5c71a53b80e2aa7481b1d077fe687e1a09ed9d46fe4c2832e4964a02c300bde` |

`b8bebfb3`の学習器は、現行のネットの学習に使った`7307d3d2`の学習器に、既定で無効の`--train-subsample`を加えたものである。
入力は、現行のネットの学習と同じ30ファイル（222,636局、訓練79,458,082局面、検証4,153,303局面）と、基準の重みファイル`crates/minase/nets/pst.bin`（SHA-256 `efe2c145…`）である。
新しいネットの`nnue.bin`はコミット`6d47da2a`で`crates/minase/nets/nnue.bin`へ埋め込み、`cargo test --locked`の1,498件がすべて成功した。

## 環境

測定機（Intel Core Ultra 7 265KF、20コア、メモリ30 GB、RTX 3060 Ti）で、2026年10月25日に行った。
同じ機械でlishogiのBotが動いており、回帰検査の間は、学習器の変更と診断のスクリプトを作るcodexの2つのタスクがビルドと検査を行っていた。
本学習の経過時間は2時間3分55秒、最大常駐メモリは10.7 GBだった。

## 結果

### 学習の条件の検査

第1の検査では、変更前の学習器の1,000更新目の検証損失が0.458106442396301となり、現行のネットの学習の`validation-interval.jsonl`の1行目と浮動小数点のビット列まで一致した。
検証局面数と決着局面数（4,153,303と4,107,348）も一致した。
最初の起動は47秒で入力ファイルが見つからずに終わった。
`--data`の一覧を作るのに使った`ls`が、ファイル名にアイコンの文字を付けて出力する別のコマンドの別名になっていたためであり、globで一覧を作り直して実行し直した。

第2の検査では、本学習と回帰検査の`config.json`を現行のネットの学習のものと照合した。
`settings`、`inputs.files`、および`preprocessing`（教師の勝率尺度1,039.5786センチポーン、訓練と検証の分割、観測の集計を含む）は、本学習と回帰検査のどちらでも一致した。
本学習で異なっていたのは、`--init-pst`の引数と入力の記録がないこと、出力先、GPUメモリの使用量、および前処理の秒数だけである。

第3の検査では、回帰検査と本学習のエポック0のチェックポイントで、第1層、第1層のバイアス、出力重み、および出力バイアスを含む線形経路以外の全配列がバイト列まで一致した。

### 学習の経過

| 項目 | 現行のネット | 新しいネット |
|---|---:|---:|
| 線形経路の初期値 | 学習済みPST（J75） | 探索用駒価値 |
| エポック0の検証損失 | 0.4643992 | 0.4903012 |
| 最良の検証損失 | 0.4575547 | 0.4575860 |
| 最良エポック / 停止エポック | 17 / 22 | 19 / 24 |
| 最良エポックの訓練損失（エポック中の平均） | 0.4558313 | 0.4558634 |
| 最良エポックの符号一致率 | 0.7437 | 0.7438 |
| 途中検証の最小（更新回数、エポック） | 0.4575547（63,000、13） | 0.4575864（115,000、24） |
| 最良エポックの線形重みの移動量、序中盤（平均 / 最大、センチポーン） | 0.75 / 31.2 | 0.95 / 21.4 |
| 最良エポックの線形重みの移動量、終盤（平均 / 最大、センチポーン） | 0.92 / 25.7 | 1.26 / 31.6 |
| 最良エポックの非線形出力の標準偏差（センチポーン） | 512 | 1,693 |
| 量子化誤差の平均（上限26.82センチポーン） | 15.66 | 21.12 |
| 駒除去差分の符号反転の割合 | 0.56%（3,161件） | 0.43%（2,444件） |
| `nnue.bin`のSHA-256 | `faa5fbe2…` | `5257f374…` |

検証損失は各エポックの終了時の値である。
主なエポックの検証損失は次のとおりである。

| エポック | 1 | 2 | 3 | 5 | 10 | 15 | 17 | 19 | 22 | 24 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 現行のネット | 0.4576677 | 0.4575932 | 0.4575660 | 0.4575619 | 0.4575578 | 0.4575555 | 0.4575547 | 0.4575559 | 0.4575584 | ― |
| 新しいネット | 0.4577711 | 0.4576468 | 0.4576089 | 0.4576036 | 0.4575917 | 0.4575889 | 0.4575873 | 0.4575860 | 0.4575884 | 0.4575883 |

新しいネットの検証損失は、エポック0では現行のネットより0.026高かったが、エポック1で差が0.0001まで縮み、最良の値での差は3.1×10⁻⁵だった。
線形重みの初期値からの移動量は、平均で1センチポーン前後、最大で約30センチポーンにとどまり、現行のネットの移動量と同程度である。
探索用駒価値の初期値は升ごとの差を持たないので、新しいネットの線形経路で同じ駒状態の升どうしの差は、移動量の最大の2倍（約60センチポーン）を超えない。
一方、非線形出力の標準偏差は現行のネットの3.3倍であり、位置による評価の差は非線形部分が担っている。

### 診断

評価値の分布では、PST＋FMに対する標準偏差の比が新しいネットで1.027（判定の範囲0.9〜1.1の内側）、現行のネットで1.035であり、PST＋FMとのピアソン相関は新しいネットが0.9938、現行のネットが0.9940だった。
駒除去差分では、量子化前の重みの2つの計算経路の差の最大が3.8×10⁻¹²センチポーン（上限10⁻⁶）だった。
合法手の評価差では、47局面すべてで新しいネットとPST＋FMの合法手の集合が一致した。
どの診断にも計算経路の不一致は現れなかった。

## 結論

線形経路を探索用駒価値で初期化したネットは、現行のネットと同じ条件の学習で、検証損失の差を3.1×10⁻⁵まで縮め、量子化の検査と3つの診断に通ったので、非劣性の測定へ進めた。
