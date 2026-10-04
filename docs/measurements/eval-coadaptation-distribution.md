# eval-coadaptation-distribution

## 目的

[共適応の検証](../plans/eval-search-coadaptation.md)の「追加診断」の「評価値の分布の比較」として、λ=1.0で学び直したPSTが、本書の仮説の前提である評価値の散らばりの尺度を変えていたかを、対局をせずに同じ局面の集合で調べる。
比較のため、λ=0.75のまま同じ追加学習をした対照Pcと、過去のλ=1.0の重みも並べる。

## コマンドライン

```console
OPENBLAS_NUM_THREADS=1 OMP_NUM_THREADS=1 nice -n 19 tools/train/.venv/bin/python \
  docs/measurements/eval-coadaptation-distribution/analyze_distribution.py \
  --data data/strength-stage7/gen2/generated-{600000,700000,800000,900000,1000000}.bin \
         data/gen3/generated-{1100000,1200000,1300000,1400000,1500000}.bin \
  --sample 100000 --seed 20261005 \
  --pair g23-plambda crates/minase/nets/pst.bin data/eval-coadaptation-lambda100/training/pst.bin \
  --pair g23-pc crates/minase/nets/pst.bin data/eval-coadaptation-lambda075/training/pst.bin \
  --pair old-plambda <44ae7cbのnets/pst.bin> <16a2257のnets/pst.bin> \
  --output docs/measurements/eval-coadaptation-distribution/result.json
```

スクリプトは[analyze_distribution.py](eval-coadaptation-distribution/analyze_distribution.py)、出力は[result.json](eval-coadaptation-distribution/result.json)である。
評価値は、学習ツールの`integer_evaluate`による手番側から見た量子化後の整数評価値であり、初期局面でG23が27 cp、Pλが33 cpとなって、Rustの評価と一致する。

## エンジン

比べた重みは次の5つである。

| 名前 | 重みのSHA-256 | 来歴 |
|---|---|---|
| G23 | `e5ba5022…` | 採用PST（M `4fb1582`の`crates/minase/nets/pst.bin`） |
| Pλ | `e493717c…` | [G23からλ=1.0で10エポック](eval-coadaptation-lambda100-training.md) |
| Pc | `a5bc54f1…` | [G23からλ=0.75で10エポック](eval-coadaptation-lambda075-training.md) |
| 過去の基点 | `72d7aee9…` | `44ae7cb`の`nets/pst.bin`（先読み教師での再学習） |
| 過去のλ=1.0 | `c6d6ef46…` | ブランチ`teacher-mixing-ratio-100-candidate`の`16a2257` |

探索用の歩兵の価値は、いずれも100 cpである。

## 環境

学習ツールの環境（`tools/train/.venv`）で、2026年10月5日に1スレッドで実行した。所要時間は約12秒である。
同じ時間帯に[Cc対S0の対局](eval-coadaptation-lambda075-stc400.md)（同時16局）が走っていたので、`nice -n 19`で優先度を下げた。

## 結果

局面は、世代2と世代3の10ファイルの29,208,818局面から、乱数シード20261005で100,000局面を非復元に一様抽出した。
学習データの局面は形勢の大きく傾いたものを多く含み、G23の評価値の絶対値の中央値は1,119 cpである。

全局面の比較は次のとおりである。
差は候補から基点を引いた値であり、残差は回帰式「候補 = a + b × 基点 + 残差」の残差である。

| 組 | 標準偏差の比 | Spearmanの順位相関 | 残差の標準偏差 | 差の絶対値の中央値と90%点 | 差の絶対値が100 cpを超える割合 |
|---|---:|---:|---:|---|---:|
| G23→Pλ | 1.042 | 0.9974 | 100.4 cp | 91 cp、255 cp | 46.0% |
| G23→Pc | 1.032 | 0.9998 | 36.5 cp | 43 cp、161 cp | 24.4% |
| 過去の基点→過去のλ=1.0 | 1.021 | 0.9965 | 108.4 cp | 75 cp、199 cp | 37.7% |

基点の評価値がほぼ互角の帯（絶対値0〜99 cp）では、差は次のとおりである。

| 組 | 局面数 | Spearmanの順位相関 | 残差の標準偏差 | 差の絶対値の中央値と90%点 | 差の絶対値が100 cpを超える割合 |
|---|---:|---:|---:|---|---:|
| G23→Pλ | 12,230 | 0.640 | 78.4 cp | 47 cp、131 cp | 18.6% |
| G23→Pc | 12,230 | 0.948 | 20.7 cp | 9 cp、32 cp | 0.3% |
| 過去の基点→過去のλ=1.0 | 12,899 | 0.618 | 84.4 cp | 50 cp、139 cp | 21.4% |

局面の進行度別では、駒の多い序盤寄りの区間（位相の分子q = 60〜90、40,580局面）で、残差の標準偏差がPλは88.7 cp、Pcは20.3 cp、過去のλ=1.0は97.1 cpだった。
基点の値で帯を区切ると、帯の中の基点の散らばりが小さくなるので、帯別の標準偏差の比は1より大きく出やすい（互角の帯でPλは1.80、Pcは1.07）。このため、帯別の比は尺度の変化として読まず、残差と順位相関を局面ごとの対応の変化として読む。
区間ごとの全指標は`result.json`にある。

## 結論

事前に固定した読み方では、3組とも全局面の標準偏差の比が0.9〜1.1の内にあり、「全体の尺度はほとんど変えていない」である。
したがって、Pλも過去のλ=1.0の重みも、評価値の散らばりの尺度を変える候補ではなかった。探索係数の調整で負けが縮まなかったことは、この結果と矛盾しない。
3つの候補の違いは尺度ではなく局面ごとの対応に現れた。
λ=1.0の2つの重みは、どちらも互角に近い局面の評価の並びを大きく入れ替え（順位相関0.62〜0.64）、同じ追加学習をλ=0.75で行ったPcの変化（0.95）より、局面ごとの変化が残差の標準偏差で約3〜4倍大きい。
この比較は静的評価値の分布だけを測るものであり、探索の余裕値との不整合の有無そのものは判定しない。
