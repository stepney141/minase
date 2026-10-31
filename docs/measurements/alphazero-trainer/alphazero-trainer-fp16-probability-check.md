# 学習したネットの合法手の確率による出力の照合

## 目的

[学習器の設計書](../../plans/archived/alphazero-trainer.md)の検証の節が求める、学習したネットのPyTorch、ONNX RuntimeのCPU、およびTensorRTのFP16の出力の一致を、2026年10月28日に決め直した照合の基準（[ADR](../../adr/archive/compare-fp16-zero-network-outputs-on-legal-move-probabilities.md)）で確かめる。
CPUは合法手のlogitと価値の差を1×10⁻⁴で、FP16は合法手の確率の差を1×10⁻²、価値の差を2×10⁻²で比べ、バッチの大きさ1、7、および256のすべてで照合が通るかを調べる。

## コマンドライン

```console
bash generate.sh                       # 初期のネットで1,000局を生成する
bash run-train.sh <data>/train-a       # 1サイクル分を学習する
bash check.sh                          # 3者の出力を照合する
```

学習の初期のネットは、`init`でシード20261028から作った10ブロック128チャネルのネット（ONNXのSHA-256 `a01d01d4…4168`）である。
[generate.sh](alphazero-trainer-fp16-probability-check/generate.sh)と[run-train.sh](alphazero-trainer-fp16-probability-check/run-train.sh)の引数は、[GPUでの学習の記録](alphazero-trainer-gpu-resume-check.md)と同じである。
前の記録のデータとネットは削除していたので、同じ手順で作り直した。
[check.sh](alphazero-trainer-fp16-probability-check/check.sh)は、`fixture-eval`でPyTorchの出力を書き出し、`minase dev nn-eval`で固定局面集の288局面をCPUとTensorRTのそれぞれでバッチの大きさ1、7、256で推論し、`nn_compare.py`で照合する。
標準出力と標準エラーは[check.log](alphazero-trainer-fp16-probability-check/check.log)にある。

## エンジン

学習器、生成器、および`nn_compare.py`は、masterのd1176351にブランチ`alphazero-fp16-check`の照合の変更を加えた作業ツリーのものである。
`crates/`はmasterと同一である。
規則セットは`engine-default`（L0＋P0＋R1＋E0）、推論はONNX Runtime 1.28.3（TensorRT 10.14.1.48）である。

## 環境

作業機（Intel Core Ultra 7 265KF、メモリ31 GB、NVIDIA GeForce RTX 3060 Ti 8 GB、ドライバ615.78.08）で実行した。
PyTorchは2.11.0（CUDA 13.0版）である。
作業機では別のプロジェクトの計算とlishogiのBotが動いており、負荷平均は約13であった。

## 結果

1,000局の生成は127秒で終わり、探索した局面は107,600であった（[集計](alphazero-trainer-fp16-probability-check/selfplay-summary.json)）。
学習の更新回数、学習曲線、および出力したネットのSHA-256（`45204684…a6bb`）は前の記録と一致し、照合したネットは前の記録で照合が通らなかったネットとバイト単位で同じである（[集計](alphazero-trainer-fp16-probability-check/train-summary.json)）。

PyTorchの出力との照合の結果は次のとおりである。
差は288局面についての差の絶対値の最大であり、確率は各局面の合法手のlogitをsoftmaxで正規化した値である。

| 候補 | 比べる量 | 方策の許容誤差 | 価値の許容誤差 | logitの差 | 確率の差 | 価値の差 | 判定 |
|---|---|---|---|---|---|---|---|
| CPU、バッチ1 | logit | 1×10⁻⁴ | 1×10⁻⁴ | 1.3×10⁻⁵ | 5.4×10⁻⁷ | 4.2×10⁻⁶ | 合格 |
| CPU、バッチ7 | logit | 1×10⁻⁴ | 1×10⁻⁴ | 1.3×10⁻⁵ | 5.9×10⁻⁷ | 4.5×10⁻⁶ | 合格 |
| CPU、バッチ256 | logit | 1×10⁻⁴ | 1×10⁻⁴ | 1.3×10⁻⁵ | 5.9×10⁻⁷ | 4.5×10⁻⁶ | 合格 |
| TensorRT FP16、バッチ1 | 確率 | 1×10⁻² | 2×10⁻² | 3.9×10⁻² | 1.6×10⁻³ | 9.7×10⁻³ | 合格 |
| TensorRT FP16、バッチ7 | 確率 | 1×10⁻² | 2×10⁻² | 3.4×10⁻² | 2.8×10⁻³ | 7.7×10⁻³ | 合格 |
| TensorRT FP16、バッチ256 | 確率 | 1×10⁻² | 2×10⁻² | 4.3×10⁻² | 2.4×10⁻³ | 1.1×10⁻² | 合格 |

TensorRTのエンジンの作成は、バッチの大きさごとに7.1〜9.5秒かかった。

## 結論

学習したネットは、CPUのlogitと価値の照合と、TensorRTのFP16の合法手の確率と価値の照合に、バッチの大きさ1、7、および256のすべてで通った。
