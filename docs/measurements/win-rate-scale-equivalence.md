# 勝率尺度の改名と較正コマンドの前後で学習結果が一致することの照合記録

## 目的

[勝率尺度の1定数化と教師の尺度補正係数の明示（Kの改名）の設計書](../plans/win-rate-scale.md)は、学習器が「K」と呼んでいた2つの勝率尺度（ネットの出力をロジットへ換算する尺度と、教師の探索値を勝率へ換算する尺度）の識別子と記録のキーを改め、教師の勝率尺度を必須の引数`--teacher-cp-per-logit`で与える形にする。
この変更は計算する値を変えてはならないので、採否は棋力の測定ではなく、変更前の学習器との学習結果の一致で判定する。
本書は、変更前の学習器で取った基準の結果と、改名後（フェーズ1）および較正コマンドの追加後（フェーズ2）の学習器で同じ条件の学習を行った結果を記録し、両者が文字列として一致することを確かめる。

## 環境

測定機はIntel Core Ultra 7 265KF（20コア、ハイパースレッディングなし）、メモリ30 GiBである。
学習はすべてCPUで行い、同時にlishogiのBot用のエンジンが2プロセス待機していたが、棋力測定は走らせていない。
変更前の学習器は、着手時点のmasterのコミットd13f8ac1を`cargo build --release -p minase-trainer`でビルドしたもので、実行ファイルのSHA-256は`3c055003bb346cdbb02beff54100827ab73eb83487725442b6a82cd62f7dcbfb`である。
同じコミットのエンジン`minase`のSHA-256は`8199259e9641dfe263aea33d52e650edf51de9998fc69691ce9e76d7e0d171c0`である。

## 照合の条件

入力には、評価関数がNNUEになったmasterで生成した乱数開始の自己対局のデータから次の2ファイルを使い、この順に`--data`へ与える。
どちらも教師の重みの検査和`3684a5ef…98c32`、生成コミット31c2a46、1局面2万ノード、規則セット`L0,P0,R1,E0`で生成されている。

| ファイル | バイト数 | 項目数（ヘッダの`entry_count`） | SHA-256 |
|---|---|---|---|
| `data/nnue-lr-schedule-gen/generated-7100000.bin` | 25,546,960 | 3,479,022 | `6397daa8f1feabea6eb65deb1b2d98614f20930d2b49ba37958edfe98cb63c07` |
| `data/nnue-lr-schedule-gen/generated-7200000.bin` | 25,140,197 | 3,420,982 | `cd5350a90345c192a1e6d9d78fab72089eeb0546f7ba5c45e69ea7db54eb53d5` |

初期の重みは次の3ファイルである。
NNUEの線形経路の初期値は同時学習`j75`の量子化前の重み、NNUEの勝率尺度と探索用駒価値の基準は置き換え前のPST＋FMの重み、PSTとFMの初期値はエンジンに組み込まれている初期PSTである。

| 用途 | ファイル | SHA-256 |
|---|---|---|
| NNUEの`--init-pst` | `data/pst-fm-joint-training/j75/pst-float.npz` | `828f8c0064e6bface1b85324cdc47bb0b26f037fa8b2f89eb97dbc2949dfebdd` |
| NNUEの`--base` | `crates/minase/nets/pst.bin` | `efe2c1455f19c1b019051d672d37204e714ed1043daec88b44422c3a7f780c53` |
| PSTの`--init`、診断の`--base` | `crates/minase/nets/pst-init.bin` | `8e82a245009b08ed81c149e26fa8fe42e2c7eb82a5d324047b57bc53972f444f` |

FMの`--init`にはPSTの学習が書き出した`pst.bin`を与える。
同時学習の`--base`と`--fm-reference`にはFMの学習が書き出した`fm.bin`を、`--pst-init`と`--fm-init`にはそれぞれの`pst-float.npz`と`fm-float.npz`を与える。
診断の`--candidate`と`--candidate-float`にはPSTの学習が書き出した`pst.bin`と`pst-float.npz`を与える。
変更後の照合では、変更前の学習器が書き出したこれらのファイルをそのまま入力に使う。

学習はいずれも2エポックで、スレッド数は4である。
変更前の学習器のコマンドは次のとおりである。
`$D`は`data/nnue-lr-schedule-gen`、`$ROOT`はリポジトリのルート、`$OUT`は出力ディレクトリを置く親ディレクトリである。
変更後の学習器では、キー名の変更のほか、フェーズ2から必須になった`--teacher-cp-per-logit calibrate`を加え、較正ファイルを使う照合ではその値を較正ファイルのパスに置き換える。

```sh
minase-trainer nnue train --device cpu --threads 4 --feature-set P --width 16 \
  --data $D/generated-7100000.bin $D/generated-7200000.bin \
  --init-pst $ROOT/data/pst-fm-joint-training/j75/pst-float.npz \
  --base $ROOT/crates/minase/nets/pst.bin \
  --lambda 1.0 --batch 16384 --epochs 2 --seed 1 --output-dir $OUT/nnue-1

minase-trainer pst train --threads 4 \
  --data $D/generated-7100000.bin $D/generated-7200000.bin \
  --init $ROOT/crates/minase/nets/pst-init.bin \
  --removal-reference init --lambda 0.75 \
  --lr 0.03 --batch 16384 --epochs 2 --patience 5 --seed 1 --output-dir $OUT/pst-1

minase-trainer fm train --threads 4 \
  --data $D/generated-7100000.bin $D/generated-7200000.bin \
  --init $OUT/pst-1/pst.bin --epochs 2 --patience 5 --lambda 1.0 \
  --lookahead-gamma 0.9 --lookahead-plies 40 --seed 1 --output-dir $OUT/fm-1

minase-trainer joint train --threads 4 \
  --data $D/generated-7100000.bin $D/generated-7200000.bin \
  --base $OUT/fm-1/fm.bin --pst-init $OUT/pst-1/pst-float.npz --fm-init $OUT/fm-1/fm-float.npz \
  --fm-reference $OUT/fm-1/fm.bin --epochs 2 --patience 5 --lambda 0.75 --seed 1 --output-dir $OUT/joint-1

minase-trainer diag pst-compare \
  --data $D/generated-7100000.bin $D/generated-7200000.bin \
  --lambda 0.75 --lookahead-gamma 0.9 --lookahead-plies 40 \
  --base $ROOT/crates/minase/nets/pst-init.bin \
  --candidate $OUT/pst-1/pst.bin --candidate-float $OUT/pst-1/pst-float.npz \
  --output-dir $OUT/diag-1 --sample-size 10000 --seed 1
```

比べる値は、`config.json`の`preprocessing`にある訓練局面数、検証局面数、分類ごとの教師の勝率尺度、訓練と検証の通し番号の列のSHA-256、`training.jsonl`の各エポックの訓練損失、補助損失、総損失、検証損失、および書き出した重み（`nnue.bin`、`pst.bin`、`fm.bin`、`pst-float.npz`、`fm-float.npz`）のSHA-256である。
診断では、`report.json`の教師の勝率尺度と、キー名を読み替えた`report.json`全体、および標本と代表局面のファイルのSHA-256を比べる。
浮動小数点の値はJSONに書かれた最短の10進表現のまま比べ、キー名は改名前後の対応表（`teacher_ks`→`teacher_cp_per_logit`、`output_k`と`k`→`cp_per_logit`、`teacher_k_source`→`teacher_cp_per_logit_source`、`nodes`→`search_budget`、`teacher_scale`→`teacher_search_budget`）で読み替える。

## 変更前の基準

変更前の学習器が同じ条件を決定的に再現することを先に確かめるため、NNUE、PST、FM、同時学習、および診断をそれぞれ2回ずつ実行し、2回の値がすべて一致することを確認した。
1回の所要時間は、NNUEが約200秒、PSTが約70秒、FMが約100秒、同時学習が約110秒である。

前処理の値は、先読みなしのNNUE、PST、同時学習で共通である。
FMと診断は先読み（γ=0.9、n=40）を使うので、教師の勝率尺度が異なる。
診断は探索値をf64で計算し直してから較正するので、FMの値ともわずかに異なる。

| 項目 | 値 |
|---|---|
| 訓練局面数 | 5,292,624 |
| 検証局面数 | 272,515 |
| 教師の勝率尺度（先読みなし） | 1031.3988510648821 |
| 教師の勝率尺度（FM、先読みあり） | 998.5442036123447 |
| 教師の勝率尺度（診断、先読みあり、f64） | 998.5442711418555 |
| 勝率尺度（NNUE、`pst.bin`） | 1072.6529541015625 |
| 勝率尺度（PST、FM、同時学習、診断、`pst-init.bin`） | 807.0253295898438 |
| 訓練の通し番号のSHA-256 | `02889b80215f385e1164e60252c81e7b694c20793a368b73dc18f50317dbfafb` |
| 検証の通し番号のSHA-256 | `79fd6b93b2b99a47df43d9e50572c4046635eeae1aba133b7adaa5d01a469ff3` |

NNUE（λ=1.0）の各エポックの損失は次のとおりである。
エポック0は学習前の検証である。

| エポック | 訓練損失 | 補助損失 | 総損失 | 検証損失 |
|---|---|---|---|---|
| 0 | | | | 0.4527878163079818 |
| 1 | 0.46030053031615376 | 0.0006784607158297866 | 0.46097899103198353 | 0.44879804867581097 |
| 2 | 0.4588308250308973 | 8.448641803086421e-6 | 0.4588392736727004 | 0.44851169274749597 |

PST（λ=0.75）の各エポックの損失は次のとおりである。

| エポック | 訓練損失 | 補助損失 | 総損失 | 検証損失 |
|---|---|---|---|---|
| 0 | | | | 0.47672316060422404 |
| 1 | 0.4906799430788697 | 0.0 | 0.4906799430788697 | 0.47424008570891124 |
| 2 | 0.4883192642557047 | 0.0 | 0.4883192642557047 | 0.4723289013254462 |

FM（λ=1.0、先読みあり）の各エポックの損失は次のとおりである。
FMは補助損失を持たず、総損失は正則化項を含む。

| エポック | 訓練損失 | 総損失 | 検証損失 |
|---|---|---|---|
| 0 | | | 0.467108321827707 |
| 1 | 0.4605778141135798 | 0.46062594962010045 | 0.44400413537980304 |
| 2 | 0.4527132626138989 | 0.45281845678526844 | 0.4421697488660077 |

同時学習（λ=0.75）の各エポックの損失は次のとおりである。

| エポック | 訓練損失 | 補助損失 | 総損失 | 検証損失 |
|---|---|---|---|---|
| 0 | | | | 0.44866812786102783 |
| 1 | 0.46062541380940936 | 2.843523801903488e-6 | 0.463554424256475 | 0.4488325200956792 |
| 2 | 0.45933159122560485 | 2.0111913898845768e-7 | 0.45962345808030186 | 0.4486596412632948 |

書き出した重みのSHA-256は次のとおりである。

| 出力 | SHA-256 |
|---|---|
| NNUEの`nnue.bin` | `31ca7e7f2955a76852c47789d49ce4431b86c9a81b8d8ac7b760e98cad878c3f` |
| PSTの`pst.bin` | `03c770a0187ece4da7b7c411947551396b4d3304a5f8aa49a510fc086823bff0` |
| PSTの`pst-float.npz` | `7dac7857458fb67346cba51c6a556ef9f247a487dd2d0c7b472bd4e5c968d6a7` |
| FMの`fm.bin` | `e42843c89c275ddce06ae47af6cf59607af39f02f34ef6b5ee3b711740daca4c` |
| FMの`fm-float.npz` | `433422543fe57ad07a7f79a0ad6954942916e94b6b15faaa3a3c4e547f7633f7` |
| 同時学習の`pst.bin` | `a7b1010c40c222287d1a1713976b276fb2add2c5baa0e99d95300d905649aea6` |
| 同時学習の`pst-float.npz` | `104d185c9c60660022e825292bd08574b4d2c01170db514c3d65b0c5b2d165ad` |

## 照合1（改名）

改名後の学習器（実行ファイルのSHA-256は`51a6ad8c87f24a0e0e5d8e6ac79ac42aff64a373b9eee58333da5baec174cbcf`）で、同じ条件の学習と診断をそれぞれ2回ずつ行った。
NNUE、PST、FM、同時学習、および診断のすべてで、前処理の値、各エポックの損失、書き出した重みのSHA-256、およびキー名を読み替えた`report.json`が、変更前の基準と文字列として一致した。

同じ学習器で、NNUEとPSTを1エポックで止めて`--epochs 2 --resume`で再開した学習も、前処理の値と各エポックの損失が基準と一致し、最終の重みは中断せずに2エポックを続けた学習の出力とバイト単位で一致した。

エンジン側の変更は重みファイルのヘッダのアクセサ名の改名だけなので、`minase dev bench --depth 7`の15局面の最善手、評価値、ノード数が変更前（総ノード数1,754,128）と一致することを確かめた。
重みファイルの読み込み時の検査（ヘッダの値の範囲と本体の検査和）は、エンジンの形式のテストが改名後も通ることで確かめた。

## 照合2（較正コマンド）

較正コマンドを加えた学習器（実行ファイルのSHA-256は`ed3a91b7227ce0b906374396380a0f905fc97767a40eec6a344aec4988f125d4`）で、同じ2ファイルから先読みなしと先読みあり（γ=0.9、n=40）の較正ファイルを作った。
先読みなしの較正ファイルの教師の勝率尺度は1031.3988510648821、先読みありは998.5442036123447であり、どちらも変更前の学習器が`config.json`に記録した値と文字列として一致した。
訓練局面数は5,292,624で、推定値は探索範囲の端に達していない。

## 照合3（2つの経路での学習）

同じ学習器で、`--teacher-cp-per-logit calibrate`を指定した学習と、照合2の較正ファイルのパスを指定した学習を、NNUE、PST、FM、同時学習のそれぞれで行った。
8つの学習のすべてで、前処理の値、各エポックの損失、および書き出した重みのSHA-256が変更前の基準と文字列として一致した。
較正ファイルを経由した学習の`config.json`は、教師の勝率尺度として較正ファイルの値をそのまま持ち、尺度補正係数（勝率尺度を教師の勝率尺度で割った値）は、NNUEでは1072.6529541015625 ÷ 1031.3988510648821 = 1.0399982053442147である。
`calibrate`を指定した学習の`config.json`も同じ値を持つ。

診断`diag pst-compare`は、`calibrate`指定では報告の全項目が基準と一致し、較正ファイル指定では教師の勝率尺度が較正ファイルの値（f32で保持した探索値からの推定値998.5442036123447）になるので、f64で計算し直した探索値から推定する診断自身の値998.5442711418555と異なり、教師値を使う損失の項目が小数点以下7桁目から異なる。
設計書が定めるとおり、診断の照合は`calibrate`指定で行い、較正ファイル指定の値の一致は求めない。

分類が2つになる入力として、masterのコミットd13f8ac1のエンジンで新たに生成した297局（1局面2万ノード、シード9900000、`data/win-rate-scale/second-gen/generated-9900000.bin`、SHA-256は`7c7e68bc3ead4cdd1101bddabfccd80a9940d381acebbe640dfb896229601a56`）を上の2ファイルに加え、3ファイルの較正ファイルを作った。
較正ファイルは2つの分類を持ち、教師の勝率尺度は生成コミット31c2a46の分類が1031.3988510648821、d13f8ac1の分類が1063.13915608192（訓練局面数106,852）である。
この較正ファイルを与えて、`--data`の順序を「2ファイル、新ファイル」と「新ファイル、2ファイル」に変えてPSTを1エポック学習したところ、`config.json`の分類の順序は入力順に入れ替わり、各分類の教師の勝率尺度と尺度補正係数は分類の属性に従って同じ値を保った。

## 引数の拒否と再開

同じ`calibrate`指定でNNUEを1エポックで止めて再開した学習は、前処理の値と各エポックの損失が中断しない学習と一致した。
改名後（フェーズ1）の学習器が書いた実行ディレクトリからの再開と、`calibrate`で始めた学習を較正ファイルの指定で再開する操作は、どちらも`checkpoint preprocessing mismatch`で拒否された。
`--teacher-cp-per-logit`を省略した学習は引数のエラーで終了し、先読みありの較正ファイルを先読みなしの学習に与えると`calibration entry 0 has no matching teacher class`で終了した。
形式識別子の不一致、分類に合わない項目、同じ分類の重複、0以下または非有限の値、探索値がすべて0の標本、λが0の分類の扱い、較正ファイルのf64の往復、および再開の拒否は、学習器のテスト`crates/minase-trainer/tests/calibration.rs`で固定した。
