# 追加学習の対照Pcの学習と診断

## 目的

[共適応の検証](../plans/eval-search-coadaptation.md)の「追加診断」の「追加学習の対照」として、[Pλ](eval-coadaptation-lambda100-training.md)と同じ設定から混合比の置き換えだけを除き、採用PSTであるG23の上へλ=0.75のまま10エポックを重ねた対照Pcを学習し、学習後の診断を通す。

## コマンドライン

設定ファイル`data/eval-coadaptation-lambda075.toml`は、Pλの設定ファイル`data/eval-coadaptation-lambda100.toml`から`run.directory`を`data/eval-coadaptation-lambda075`に改め、`[train]`節の`lambda_override = 1.0`を削除したものである。
入力の10ファイル、初期値（G23）、先読み教師（γ=0.9、n=40）、鏡映共有モデル、出力K 1072.6529541015625、学習率0.03、10エポック、バッチ16,384、乱数シード1、駒除去差分の追加損失の係数1000、および基点コミットM（`4fb1582`）はPλと同じである。

```console
tools/train/.venv/bin/pst-workflow prepare --config data/eval-coadaptation-lambda075.toml
tools/train/.venv/bin/pst-workflow train --run-dir data/eval-coadaptation-lambda075
tools/train/.venv/bin/pst-workflow diagnose --run-dir data/eval-coadaptation-lambda075
```

## エンジン

学習器と診断器は、M（master `4fb1582`）の`tools/train/`である。

## 環境

測定機はIntel Core Ultra 7 265KF（物理20コア、論理20コア、実メモリ32 GB、GeForce RTX 3060 Ti）で、2026年10月5日8時10分から8時34分に準備、学習、診断を行った。
同じ時間帯に対局は走っていなかった。

## 結果

| 項目 | 値 |
|---|---:|
| 訓練局面数 | 27,745,843 |
| 総更新回数 | 16,940 |
| 最良エポック | 10 |
| 検証損失（初期状態→最良） | 0.436731→0.435451 |
| 初期局面の評価値（学習器の参照値） | 26 cp（G23は27 cp） |
| 重みのSHA-256 | `a5bc54f1444ee364325797c1e4877690be41790e87731cd6947bc1e67b253134` |

初期状態の検証損失は、同じ教師に対するG23の値であり、[G23の学習](pst-gen3-training.md)の最良値0.436731と一致する。
10エポック目まで検証損失が下がり続けていた。

学習後の診断は次のとおりである。

| 検査 | 結果 |
|---|---|
| Rustと学習器の評価値の一致 | 100,000局面で一致 |
| 量子化の前後の平均絶対誤差 | 0.50 cp（最大1.75 cp） |
| 駒除去差分の符号の反転 | 0件（341件を検査） |
| 探索用駒価値と出力K | 不変 |

PcとG23の量子化前の重みの差の平均絶対値と最大は、序中盤で15.8 cpと145.5 cp、終盤で28.4 cpと162.5 cpだった（Pλは20.4 cpと241.1 cp、30.8 cpと223.8 cp）。

## 結論

Pcは学習後の診断を通過したので、Mの`crates/minase/nets/pst.bin`をPcへ差し替えたCc（ブランチ`eval-coadaptation-lambda075`の`4114723`）を作り、S0との400ペアの測定へ進む。
Ccでは、重みに依存する2つのテストを改めた。初期局面の参照値を26 cpにし、開局からの自己対局のテストは200ノードでは手数上限に達したので400ノードにした。
