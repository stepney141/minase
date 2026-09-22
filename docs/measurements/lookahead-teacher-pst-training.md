# 先読み教師での学習PSTの再学習と診断

## 目的

[先読み教師値](../plans/lookahead-teacher.md)の利用者の指示による候補として、段階開始版の学習PSTを先読み教師（γ=0.9、n=40）で学習し直し、学習後の診断を通して採否測定の候補を作る。

## コマンドライン

```console
tools/train/.venv/bin/python tools/train/pst/pst_workflow.py prepare --config data/strength-stage9/lookahead-pst.toml
tools/train/.venv/bin/python tools/train/pst/pst_workflow.py train --run data/strength-stage9/lookahead-pst-training
tools/train/.venv/bin/python tools/train/pst/pst_diagnostics.py \
  --data <基本の教師の11ファイル> --king-features <118列のMNKF 11ファイル> --extra-columns 0:24 \
  --lookahead-gamma 0.9 --lookahead-plies 40 \
  --base data/strength-stage9/pst-1633f53-extra24.bin \
  --candidate data/strength-stage9/lookahead-pst-training/training/pst-extra24.bin \
  --probe data/strength-stage9/lookahead-pst-training/probe/target/release/pst_probe \
  --output-dir data/strength-stage9/lookahead-pst-training/diagnostics-master --sample-size 10000 --seed 1
```

設定は、基準を段階開始版 `6c5c559`、入力を基本の教師の11ファイル、モデルを `mirrored`、出力K 1072.6529541015625、学習率0.03、駒除去差分の追加損失の係数1000、10エポック、バッチ16,384、乱数シード1、λ=0.75（来歴）、追加特徴なし、`train.lookahead = { gamma = 0.9, plies = 40 }` とし、段階7の世代2の再学習と同じ条件に先読みだけを加えた。
診断は、候補の13,680特徴の重みを追加特徴24列の重み0で拡張した写しに対して、同じ拡張をした基準と、ブランチの `pst_probe` で行った。拡張は評価値を変えない。

## エンジン

学習器はブランチ `lookahead-teacher` の `e783dc8`、候補の重みはブランチ `lookahead-pst` の `a7e32a8`（`6c5c559` に `nets/pst.bin` の差し替えだけを加えたコミット）に置いた。
重みのSHA-256は `72d7aee949e7672a2b4fb2e7b98ddaed76b352112187bfbcc7e2d772224fb67b` である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB、GeForce RTX 3060 Ti）で、2026年9月22日に実施した。学習は約27分で終わった。

## 結果

先読み教師のKは世代0から順に768.5、1068.2、758.4である（現行の教師は804.0、1099.7、777.6）。
先読み教師に対する検証損失は、初期状態（段階開始版の重み）の0.474203から、最良のエポック10で0.473405へ下がった。
損失は10エポック目まで単調に下がり続けており、収束には達していない。

学習後の診断は次のとおりである。

| 検査 | 結果 |
|---|---|
| Rustと学習器の評価値の一致 | 141,841局面で一致 |
| 量子化の前後の平均絶対誤差 | 0.59センチポーン（最大2.24） |
| 駒除去差分の符号の反転 | 0件（352件を検査） |
| 成りの評価差の一致 | 一致 |

世代と局面帯ごとの検証損失は、世代0と世代2の全帯で候補が基準より小さく、世代1では0.0001から0.0004だけ大きい。
教師探索値（先読み値）との平均絶対誤差は、世代0と世代2で候補が小さく、世代1で大きい。
除去差分から導いた序中盤の駒価値は、歩兵98、仲人115、香車368、反車377、横行492、竪行587、角行662、飛車846、龍馬1001、龍王1154であり、固定の探索用駒価値との差は最大でも5%以内である。

## 結論

候補は学習後の診断を通過し、段階開始版との差が重みファイルだけの候補 `a7e32a8` として、STC（`lookahead-teacher-pst-stc`）とLTC（`lookahead-teacher-pst-ltc`）で測る。
