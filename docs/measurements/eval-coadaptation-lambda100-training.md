# 混合比λ=1.0の候補Pλの学習と診断

## 目的

[共適応の検証](../plans/eval-search-coadaptation.md)のフェーズ1として、採用PSTであるG23を初期値に、G23の学習条件のうち混合比だけをλ=1.0に変えて候補Pλを学習し、学習後の診断を通す。

## コマンドライン

設定ファイル`data/eval-coadaptation-lambda100.toml`は、[G23の学習](pst-gen3-training.md)の設定ファイル`data/pst-gen3-g23.toml`の写しで、`run.directory`を`data/eval-coadaptation-lambda100`に、`run.base_commit`をM（`4fb158284648c26be039544385359e5704e13cb5`）に改め、`[train]`節へ`lambda_override = 1.0`を加えたものである。
入力の10ファイル、先読み教師（γ=0.9、n=40）、鏡映共有モデル、出力K 1072.6529541015625、学習率0.03、10エポック、バッチ16,384、乱数シード1、駒除去差分の追加損失の係数1000はG23と同じである。

```console
tools/train/.venv/bin/pst-workflow prepare --config data/eval-coadaptation-lambda100.toml
tools/train/.venv/bin/pst-workflow train --run-dir data/eval-coadaptation-lambda100
tools/train/.venv/bin/pst-workflow diagnose --run-dir data/eval-coadaptation-lambda100
```

## エンジン

学習器と診断器は、M（master `4fb1582`）の`tools/train/`である。
[再現の記録](eval-coadaptation-trainer-reproduction.md)のとおり、この学習器はG23の学習条件でG23をバイト単位で再現する。
初期値は、Mの`crates/minase/nets/pst.bin`（G23、SHA-256 `e5ba5022…`）である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB、GeForce RTX 3060 Ti）で、2026年10月4日21時14分から21時40分に準備、学習、診断を行った。
学習は約22分であり、同じ時間帯に他の測定は走っていなかった。

## 結果

| 項目 | 値 |
|---|---:|
| 訓練局面数 | 27,745,843 |
| 検証局面数 | 1,462,975 |
| 総更新回数 | 16,940 |
| 最良エポック | 10 |
| 検証損失（初期状態→最良） | 0.436958→0.434934 |
| 初期局面の評価値（学習器の参照値） | 33 cp（G23は27 cp） |
| 重みのSHA-256 | `e493717c0fd46715e6d74b14368d011c428021dbd4b700c5c8d2b3afd0726ec8` |

検証損失は、λ=1.0の教師に対する値であり、λ=0.75の教師に対するG23の記録の値とは比べない。
10エポック目まで検証損失が下がり続けていた。

学習後の診断は次のとおりである。

| 検査 | 結果 |
|---|---|
| Rustと学習器の評価値の一致 | 100,000局面で一致 |
| 量子化の前後の平均絶対誤差 | 0.53 cp（最大2.20 cp、合否の基準は平均2 cp以下） |
| 駒除去差分の符号の反転 | 0件（341件を検査） |
| 探索用駒価値と出力K | 不変 |

PλとG23の量子化前の重み（`training/pst-float.npz`どうし）の差の平均絶対値と最大は、序中盤で20.4 cpと241.1 cp、終盤で30.8 cpと223.8 cpだった。
これは、G23がその前の採用PSTから動いた量（序中盤で平均19.5 cp、終盤で平均33.9 cp）と同程度である。

## 結論

Pλは学習後の診断を通過したので、設計書のとおりCλを作り、S0との400ペアの測定へ進む。
