# 新しい構成のランダム初期化のネットの出力の照合

## 目的

[学習技法の導入の設計書](../../plans/alphazero-restart-techniques.md)のフェーズ4として、入力を159面にし、価値ヘッドを32チャネルに広げ、補助ヘッドを加えた10ブロック128チャネルのネットをランダムに初期化し、PyTorchの出力を基準に、ONNX RuntimeのCPUとTensorRTのFP16の出力がバッチの大きさ1、7、および256で一致するかを作業機で確かめる。

## コマンドライン

測定日は2026年10月31日である。

```console
# <wt> はブランチ alphazero-restart-techniques の worktree（コミット d84bedf8）、<out-dir> は出力先
cargo build --release --bin minase
bash docs/measurements/alphazero-restart-techniques/alphazero-restart-techniques-random-net-output-check/run.sh <wt> <out-dir>
```

[run.sh](alphazero-restart-techniques-random-net-output-check/run.sh)は、`tools/train/zero/env.sh`で環境を設定し、学習器の`minase_zero init --blocks 10 --channels 128 --seed 20261031`でネットを作る。
続いて、学習器の`fixture-eval`でPyTorchの出力を書き出し、固定局面集の288局面を`minase dev nn-eval`のCPUとTensorRTでそれぞれバッチ1、7、256で推論し、`nn_compare.py`で照合する。
許容誤差は[照合の手引き](../../guides/alphazero-network-check.md)のとおり、CPUがlogitと価値の差で1×10⁻⁴、TensorRTのFP16が合法手の確率の差で1×10⁻²、価値の差で5×10⁻²である。
標準出力と標準エラーは同じディレクトリの[run.log](alphazero-restart-techniques-random-net-output-check/run.log)にある。

## エンジン

ネットは`model.onnx`（SHA-256 `b2de222cdc9d013814d928bd507d71c502d541fdd25616c80c69491d3e527c48`）であり、学習器の`init`が記録したパラメータ数は、補助ヘッドを含めて4,780,200である。
ONNXの出力は、今までどおり方策のlogit（35,856）と主の価値（1）の2つだけである。
minaseと学習器はコミットd84bedf8でビルドした。

## 環境

作業機（Intel Core Ultra 7 265KF、NVIDIA GeForce RTX 3060 Ti、ドライバ615.78.08）で、ONNX Runtime 1.28.3とTensorRT 10.14.1を使った。

## 結果

すべての照合が許容誤差に収まった。

| 出力 | バッチ | logitの差の最大 | 合法手の確率の差の最大 | 価値の差の最大 | 最大の手の不一致 |
|---|---|---|---|---|---|
| CPU | 1 | 2.31×10⁻⁷ | 4.69×10⁻⁹ | 6.43×10⁻⁸ | 0 |
| CPU | 7 | 2.42×10⁻⁷ | 4.29×10⁻⁹ | 6.43×10⁻⁸ | 0 |
| CPU | 256 | 2.42×10⁻⁷ | 4.29×10⁻⁹ | 6.43×10⁻⁸ | 0 |
| TensorRTのFP16 | 1 | 6.44×10⁻⁴ | 1.38×10⁻⁵ | 1.35×10⁻⁴ | 2 |
| TensorRTのFP16 | 7 | 5.48×10⁻⁴ | 1.45×10⁻⁵ | 1.37×10⁻⁴ | 1 |
| TensorRTのFP16 | 256 | 7.13×10⁻⁴ | 1.19×10⁻⁵ | 1.33×10⁻⁴ | 2 |

TensorRTの最大の手の不一致は、基準のlogitの差が2.4×10⁻⁴以下のほぼ同点の局面で起きており、手引きのとおり合否には使わない。
TensorRTのエンジンの作成は、最初の実行の各バッチで7.7〜10.0秒かかった。
`run.log`は同じ出力先で実行し直した2回目のものであり、エンジンはキャッシュから0.39〜0.47秒で読み込んだ。照合の値は2回とも同じであった。

## 結論

新しい構成のネットは、159面の入力と方策・主の価値の2出力で、PyTorch、ONNX RuntimeのCPU、およびTensorRTのFP16の出力が一致するので、自己対局の推論に使える。
