# 2回目の学習に使う、AlphaZeroの形の初期ネットの出力の照合

## 目的

[学習レシピを先行実装に揃える設計書](../../plans/archived/zero-recipe-precedents.md)のフェーズ4として、ランダムに初期化した20ブロック256チャネルのAlphaZeroの形のネットを2回目の学習の初期ネットとして作り、PyTorchの出力を基準として、ONNX RuntimeのCPUとTensorRTのFP16の出力がバッチの大きさ1、7、および256で許容誤差に収まるかを作業機で確かめる。

## コマンドライン

測定日は2026年11月16日である。
`<wt>`はブランチ`zero-recipe-precedents`のコミット`94aa98a1db3c5665fac1353ee533fa7608614331`の作業ツリー、`<out>`は作業機の一時の出力先である。
作業機では持ち時間つきの対局の測定が並行して走っていたので、すべて`nice -n 19`で実行し、学習器には`OMP_NUM_THREADS=4`を与えた。

```console
cd <wt>/tools/train/zero
uv run --frozen python -m minase_zero init --blocks 20 --channels 256 --seed 20261116 \
  --out-dir <out>/cycle-0000 --trainer-commit 94aa98a1db3c5665fac1353ee533fa7608614331
uv run --frozen python -m minase_zero fixture-eval --checkpoint <out>/cycle-0000/checkpoint.pt \
  --onnx <out>/cycle-0000/model.onnx --fixture ../../../crates/minase/tests/fixtures/nn-positions.jsonl \
  --out <out>/cycle-0000/check/pytorch.json
cd <wt>
bash docs/measurements/zero-recipe-precedents/zero-recipe-precedents-initial-net-check/check.sh \
  <out>/cycle-0000/model.onnx <out>/cycle-0000/check/pytorch.json <out>/cycle-0000/check-workstation
```

[check.sh](zero-recipe-precedents-initial-net-check/check.sh)は、[出力の照合の手引き](../../guides/alphazero-network-check.md)の3節のUSIの`nn-eval`で、固定局面集の288局面をCPUとTensorRTのバッチ1、7、256で推論し、5節の`nn_compare.py`と許容誤差でPyTorchの出力と照合する。
2回目の学習の準備で前の初期ネットを照合したスクリプトは、廃止したサブコマンド`minase dev nn-eval`を使っていたので、USIの`nn-eval`を使う形に書き直した。
`check.sh`の標準出力と標準エラーは、同じディレクトリの[check.log](zero-recipe-precedents-initial-net-check/check.log)にある。

照合の後、`<out>/cycle-0000/`の`checkpoint.pt`、`model.onnx`、`summary.json`、`check/pytorch.json`、および`check-workstation/`の6つの出力を、NASの`/mnt/synology_nas/minase/alphazero/cycles-rl2-az/cycle-0000/`へ複写し、SHA-256が元と一致することを確かめて、最初の3つを読み取り専用にした。

## エンジン

ネットは`model.onnx`（SHA-256 `4b430a4414d250f96094c475dc489efbc865afa20e1c9bfc5b2a958cae4abf07`）であり、学習器の`init`が記録したパラメータ数は25,182,204である。
チェックポイント`checkpoint.pt`のSHA-256は`56761465cf8bdddfaa523237ed868a356d8b0a0874505ef05bd12fa0e00b3973`、集計`summary.json`のSHA-256は`aaf42b3d777251284a6fb498192ca4acb6454372a9e855eb6cf6d890ce082891`、PyTorchの出力`pytorch.json`のSHA-256は`e16b7bd9e704b96f70db9983507f8b17c28957a7d35c99095d0485131b798e22`である。
minaseは同じ作業ツリーのRustのコードからビルドした。
規則セットは`JCSA`である。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、NVIDIA GeForce RTX 3060 Ti、ドライバ615.78.08）で、ONNX Runtime 1.31.1とTensorRT 10.14.1を使った。
`init`と`fixture-eval`はCPUで実行した。

## 結果

許容誤差は照合の手引きのとおり、CPUがlogitと価値の差で1×10⁻⁴、TensorRTのFP16が合法手の確率の差で1×10⁻²、価値の差で5×10⁻²である。
すべての照合がこの許容誤差に収まった。

| 出力 | バッチ | logitの差の最大 | 合法手の確率の差の最大 | 価値の差の最大 | 最大のlogitの手の不一致（局面数） |
|---|---|---|---|---|---|
| CPU | 1 | 3.04×10⁻⁵ | 7.94×10⁻⁷ | 8.87×10⁻⁸ | 1 |
| CPU | 7 | 3.04×10⁻⁵ | 7.94×10⁻⁷ | 8.87×10⁻⁸ | 1 |
| CPU | 256 | 3.04×10⁻⁵ | 7.94×10⁻⁷ | 8.87×10⁻⁸ | 1 |
| TensorRTのFP16 | 1 | 4.82×10⁻⁴ | 1.28×10⁻⁵ | 2.49×10⁻⁴ | 2 |
| TensorRTのFP16 | 7 | 5.84×10⁻⁴ | 1.12×10⁻⁵ | 2.52×10⁻⁴ | 4 |
| TensorRTのFP16 | 256 | 5.10×10⁻⁴ | 1.11×10⁻⁵ | 2.63×10⁻⁴ | 4 |

CPUのlogitの差の最大は、手引きのとおり丸める前のPyTorchの出力との差であり、FP16への丸めの1目盛りを引いて判定した値は許容誤差の内側である。
最大のlogitの手が基準と食い違った局面は、基準とminaseがそれぞれ最大とした2つの手の、基準の出力でのlogitの差が2.1×10⁻⁴以下のほぼ同点の局面であり、この食い違いは手引きのとおり合否の判定には使わない。
TensorRTの推論の時間は、エンジンを作った後の288局面で0.04〜0.17秒であった。
ネットの構成と初期化のシードはフェーズ1のGPUでの確認（[記録](zero-recipe-precedents-phase1-gpu-check.md)）と同じであり、`model.onnx`のSHA-256も一致した。
チェックポイントのSHA-256が異なるのは、記録した学習器のコミットが違うからである。
ランダム初期化のネットは出力の散らばりが小さいので、この差は学習したネットの差の目安にならない（[教訓](../../lessons/judge-low-precision-inference-on-trained-outputs.md)）。

## 結論

初期ネットは出力の照合に通ったので、NASの系列`cycles-rl2-az`の`cycle-0000/`に置き、2回目の学習のサイクル1の生成と学習に使う。
