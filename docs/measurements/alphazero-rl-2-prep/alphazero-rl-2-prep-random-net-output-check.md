# 2回目の学習に使う初期ネットの出力の照合

## 目的

[2回目の実行の準備の設計書](../../plans/archived/alphazero-rl-2-prep.md)のフェーズ2として、強化学習の2回目の学習で最初に使う、ランダムに初期化した20ブロック256チャネルのネットの出力を作業機で確かめる。
PyTorchの出力を基準として、ONNX RuntimeのCPUとTensorRTのFP16の出力が、バッチの大きさ1、7、および256で許容誤差に収まるかを調べる。

## コマンドライン

測定日は2026年11月10日である。
`<wt>`はmasterのマージコミット`cd156c0ec18993bfc1a45450229f16b3f5bfa6a4`の作業ツリー、`<out>`は作業機の出力先`data/alphazero-rl-2-prep`である。

```console
cd <wt>/tools/train/zero
uv run --frozen python -m minase_zero init --blocks 20 --channels 256 --seed 20261109 \
  --out-dir <out>/cycle-0000 --trainer-commit cd156c0ec18993bfc1a45450229f16b3f5bfa6a4
uv run --frozen python -m minase_zero fixture-eval --checkpoint <out>/cycle-0000/checkpoint.pt \
  --onnx <out>/cycle-0000/model.onnx --fixture ../../../crates/minase/tests/fixtures/nn-positions.jsonl \
  --out <out>/cycle-0000/check/pytorch.json
cd <wt>
cargo build --release --bin minase
bash docs/measurements/alphazero-rl-2-prep/alphazero-rl-2-prep-random-net-output-check/check.sh \
  <out>/cycle-0000/model.onnx <out>/cycle-0000/check/pytorch.json <out>/cycle-0000/check-workstation
```

学習器`minase_zero`の`init`は、ランダムに初期化したネットのチェックポイント`checkpoint.pt`、ONNX形式のネット`model.onnx`、および集計`summary.json`を書く。
`fixture-eval`は、チェックポイントからONNXを書き出し直して`model.onnx`とバイト単位で一致することを確かめ、固定局面集`nn-positions.jsonl`の288局面に対するPyTorchの出力`pytorch.json`を書く。
[check.sh](alphazero-rl-2-prep-random-net-output-check/check.sh)は、同じ288局面を`minase dev nn-eval`のCPUとTensorRTでそれぞれバッチ1、7、256で推論し、`nn_compare.py`でPyTorchの出力と照合する。
`check.sh`の標準出力と標準エラーは、同じディレクトリの[check.log](alphazero-rl-2-prep-random-net-output-check/check.log)にある。

## エンジン

ネットは`model.onnx`（SHA-256 `8be01e2ed1608d4287853d8b43a871ce08d6f8dda4864b15ae966a62459683a6`）であり、学習器の`init`が記録したパラメータ数は、補助ヘッドを含めて28,052,008である。
チェックポイント`checkpoint.pt`のSHA-256は`02c75e68205a29e8b63edf619d5b8987d001bf88e58d613242327fa583292fff`、PyTorchの出力`pytorch.json`のSHA-256は`38f3ecbc0d92648494bd51ec459bef9b6c75855890ebceb3d577cd2e9f0c8fff`である。
minaseと学習器は、どちらもコミット`cd156c0e`でビルドした。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、NVIDIA GeForce RTX 3060 Ti、ドライバ615.78.08）で、ONNX Runtime 1.28.3とTensorRT 10.14.1を使った。
`init`と`fixture-eval`はCPUで実行した。

## 結果

許容誤差は[照合の手引き](../../guides/alphazero-network-check.md)のとおり、CPUがlogitと価値の差で1×10⁻⁴、TensorRTのFP16が合法手の確率の差で1×10⁻²、価値の差で5×10⁻²である（[FP16の照合のADR](../../adr/compare-fp16-zero-network-values-within-five-hundredths.md)）。
すべての照合がこの許容誤差に収まった。

| 出力 | バッチ | logitの差の最大 | 合法手の確率の差の最大 | 価値の差の最大 | 最大のlogitの手の不一致（局面数） |
|---|---|---|---|---|---|
| CPU | 1 | 4.82×10⁻⁷ | 9.96×10⁻⁹ | 8.79×10⁻⁸ | 0 |
| CPU | 7 | 4.82×10⁻⁷ | 9.96×10⁻⁹ | 8.79×10⁻⁸ | 0 |
| CPU | 256 | 4.82×10⁻⁷ | 9.96×10⁻⁹ | 8.79×10⁻⁸ | 0 |
| TensorRTのFP16 | 1 | 1.33×10⁻³ | 3.20×10⁻⁵ | 2.61×10⁻⁴ | 0 |
| TensorRTのFP16 | 7 | 1.49×10⁻³ | 3.21×10⁻⁵ | 2.61×10⁻⁴ | 2 |
| TensorRTのFP16 | 256 | 1.71×10⁻³ | 3.72×10⁻⁵ | 2.73×10⁻⁴ | 1 |

TensorRTで最大のlogitの手が基準と食い違った局面は、基準（PyTorch）の出力で、両者が最大とした2つの手のlogitの差が3.1×10⁻⁴以下のほぼ同点の局面であり、手引きのとおり合否には使わない。
TensorRTのエンジン（ネットをGPU向けに最適化したもの）の作成は、各バッチの最初の実行で10.8〜18.5秒かかった。
10ブロック128チャネルの初期ネットの[照合](../alphazero-restart-techniques/alphazero-restart-techniques-random-net-output-check.md)と比べると、FP16の差の最大は、logitで約2.4倍、価値で約2.0倍であった。
ただし、ランダム初期化のネットは出力の散らばりが小さいので、この差は学習したネットの差の目安にならない（[教訓](../../lessons/judge-low-precision-inference-on-trained-outputs.md)）。

照合に通ったネット、チェックポイント、`init`の集計、およびPyTorchの出力は、NASの`cycles-rl2/cycle-0000/`（PyTorchの出力は`check/pytorch.json`）に置き、検査和の一致を確かめてから読み取り専用にした。

## 結論

20ブロック256チャネルの初期ネットは、PyTorch、ONNX RuntimeのCPU、およびTensorRTのFP16の出力が許容誤差の中で一致するので、2回目の学習の初期ネットとして使える。
