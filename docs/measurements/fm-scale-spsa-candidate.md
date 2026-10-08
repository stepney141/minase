# fm-scale-spsa-candidate

## 目的

[FMの補正の倍率をSPSAで調整する設計書](../plans/fm-scale-spsa.md)のフェーズ3として、[SPSAのセッション](fm-scale-spsa.md)で得た`FmScale`=611（倍率b≈0.597）を候補J75の重みへ焼き込み、候補の正しさの検査、補正の大きさ、着手差のFM分、および探索速度を、master（M）とJ75と並べて記録する。
いずれの値も採否の条件にせず、採否は[STC](fm-scale-spsa-stc.md)の段階ゲートで決める。

## コマンドライン

焼き込みと検証標本の報告は、ブランチ`fm-scale-spsa`の`train-fm scale`で行った。
入力はJ75の重みファイルと整数化前の重み、`--data`は同時学習と同じ世代2の5ファイル（`data/strength-stage7/gen2/generated-600000.bin`から`generated-1000000.bin`）と世代3の5ファイル（`data/gen3/generated-1100000.bin`から`generated-1500000.bin`）である。

```console
uv run --project tools/train train-fm scale \
  --input data/pst-fm-joint-training/j75/pst.bin \
  --float data/pst-fm-joint-training/j75/pst-float.npz \
  --fm-scale 611 --output data/fm-scale-spsa/pst-611.bin \
  --data <10ファイル>
```

Rustの評価との照合、着手差のFM分、および探索速度は次のコマンドで測った。
`minase-p1`はフェーズ1のコミット`5202775`の通常ビルドであり、プローブは重みファイルを引数で受け取るので、3つの重みの評価に同じ実行ファイルを使った。
`train-joint move-diffs`は、ブランチ`pst-fm-joint-training`（`d8447f1`）の学習ツールにある。
1,000局面のMNSDは、10万局面の検査用MNSD`diagnostic-samples.bin`から乱数シード1で無作為に選んで`data/fm-scale-spsa/move-diff-1000.bin`に保存した。

```console
fm-diagnostics --pst-changed --base crates/minase/nets/pst.bin \
  --candidate data/fm-scale-spsa/pst-611.bin \
  --positions data/pst-longer-training/diagnostics/diagnostic-samples.bin \
  --output data/fm-scale-spsa/evaluation-check-611.json \
  --probe-command minase-p1 dev pst-probe
train-joint move-diffs --weights <重みファイル> \
  --positions <representatives.bin | move-diff-1000.bin> \
  --probe-command minase-p1 dev pst-probe --output <JSON>
taskset -c 3 minase-<m|j75|candidate> dev bench --depth 9 --repetitions 3
```

## エンジン

Mはmaster `203ba2a`であり、その評価関数は候補LのPSTと候補Faの補正1/4のFMからなる（重みファイルのSHA-256 `85e4e455…`）。
J75は`data/pst-fm-joint-training/j75/pst.bin`（SHA-256 `a28d9e82…`、`4aad6df`の重みと同じ）である。
候補は、J75のFMの出力係数を611/1024倍して整数化し直した重み`data/fm-scale-spsa/pst-611.bin`（SHA-256 `efe2c145…`）であり、ブランチ`fm-scale-spsa`のコミット`f965ba2`に埋め込んだ。
候補とMの`crates/`の差は、重みファイル、初期局面の評価値のテスト、J75の候補コミット`4aad6df`と同じ探索のテストの局面の変更、調整用ビルドだけが参照する係数`FmScale`、`spsa apply`のパーサーの属性行の扱い、調整用ビルドの静止探索テストの局面、および自己対局の決定性テストの手数上限（候補の重みでは100ノードの対局が600手で終局せず記録が残らないため、600から1,200へ変更）である。
通常ビルドの評価式と探索は変わらない。

## 環境

測定機はIntel Core Ultra 7 265KF（性能コア8、高効率コア12、実メモリ32 GB）である。
焼き込みと検査は2026年10月8日20時から21時に行い、探索速度は21時15分に測った。

## 結果

### 焼き込みと整数化

| 項目 | 値 |
|---|---:|
| `FmScale` | 611（b = 0.5967） |
| FMの指数（前→後） | 9→9 |
| 埋め込みの最大絶対値 | 10,870 |
| 浮動小数点の評価と整数の評価の平均絶対誤差／最大（検証の1万局面、シード1） | 0.51／1.43 cp |
| 焼き込んだ補正と調整中の補正trunc(N·611/(1024·D))の差の平均絶対値／最大 | 0.13／1 cp |
| 補正の大きさ（焼き込んだ補正の標準偏差） | 168.2 cp |
| 調整中の補正の標準偏差 | 168.2 cp |

浮動小数点の評価との平均絶対誤差は設計書の上限2 cpを下回り、調整中の補正との差の平均絶対値も2 cpを下回ったので、整数化の扱いを見直さずに測定へ進む。
`train-fm scale`は、整数化前の重みの両端点とFMがJ75の重みファイルに整数化し直して一致することを確かめてから書き出した。
出力の両端点、探索用駒価値、および出力Kは入力のJ75と同じであり、探索用駒価値と出力KはMの重みファイルとも一致した（`fm-diagnostics --pst-changed`の`validate_search_constants`）。

### 点検

| 検査 | 候補 |
|---|---|
| Rustと学習ツールの評価値の一致（`fm-diagnostics --pst-changed`、10万局面） | 一致 |
| 探索用駒価値と出力Kの不変 | 不変 |
| 初期局面の評価値 | 53 cp（Mは41 cp、J75は69 cp） |
| `cargo test`（通常ビルドと調整用ビルド）、clippy、fmt | すべて通過 |

### 補正の大きさと着手差のFM分

補正の大きさは、検証集合から乱数シード1で選んだ1万局面での整数の補正の標準偏差である。
着手差のFM分は、合法手ごとの着手前後の評価差のうちFMの補正分を、代表6局面（394手）と1,000局面（77,759手）で測った。

| 重み | 補正の大きさ | 代表6局面の平均絶対値（歩兵との比） | 同、最大絶対値 | 1,000局面の平均絶対値（歩兵との比） | 同、最大絶対値 |
|---|---:|---:|---:|---:|---:|
| M | 68 cp | 37.2 cp（0.37） | 156 cp | 26.4 cp（0.26） | 452 cp |
| J75 | 282 cp | 153.7 cp（1.54） | 702 cp | 112.5 cp（1.12） | 2,018 cp |
| 候補 | 168 cp | 91.5 cp（0.92） | 419 cp | 67.0 cp（0.67） | 1,204 cp |

候補の補正の大きさと着手差のFM分はJ75の約0.6倍で、倍率0.597とほぼ比例しており、Mの約2.5倍である。

### 検証損失

同時学習と同じ教師（混合比λ=0.75、先読みγ=0.9、40手）で、検証集合の1,462,975局面について、整数評価を出力Kで割ったロジットの二値交差エントロピーを測った。
教師Kは訓練集合から推定した世代2と世代3の値（758.35と765.95）である。

| 重み | 検証損失 |
|---|---:|
| M | 0.430394 |
| J75 | 0.429550 |
| 候補 | 0.429709 |

J75の値は[同時学習の記録](pst-fm-joint-training-training.md)の最良エポックの検証損失0.429552と整数化の誤差の範囲で一致する。
候補の検証損失はJ75より大きく、損失が縮めない補正を好むという設計書の「目的」の節の観察と同じ向きである。

### 探索速度

M、J75、候補のビルドを性能コアのCPU 3へ固定し、3構成を交互に3巡実行して（各巡は`--repetitions 3`の中央値）、巡の中央値を比べた。
Mのビルドはmaster `203ba2a`のrunner、J75のビルドは`4aad6df`、候補のビルドは`f965ba2`である。
その間に他の対局は走っていなかった。

| 構成 | NPS（3巡の中央値） | Mとの比 | 深さ9までのノード数 | 深さ9までの時間 |
|---|---:|---:|---:|---:|
| M | 1,859,692 | 1.000 | 5,711,806 | 3.07秒 |
| J75 | 1,802,474 | 0.969 | 7,728,300 | 4.29秒 |
| 候補 | 1,775,736 | 0.955 | 7,115,381 | 4.01秒 |

評価関数の計算はMと同じなので、ノード数の差は重みの違いで探索木が変わったことによる。
深さ9までに要するノード数はMの1.25倍であり、J75の1.35倍より小さい。

## 結論

候補は整数化の誤差の上限と、Rustと学習ツールの評価の一致の検査に合格したので、設計書のとおり[STC](fm-scale-spsa-stc.md)の段階ゲートへ進める。
補正の大きさ（168 cp）と着手差のFM分はJ75の約0.6倍でMの約2.5倍、深さ9までのノード数はMの1.25倍、検証損失はJ75とMの間にある。
いずれも採否の条件にしない。
