# 最初のサイクルのネットの作業機での照合

## 目的

[強化学習の実行の設計書](../../plans/alphazero-rl.md)のフェーズ2が求めるとおり、サイクル1で学習したネットを次のサイクルの生成に使う前に、作業機のRTX 3060 Tiで、ONNX RuntimeのCPUとTensorRTのFP16の出力が、PyTorchの出力と[ネットの出力の照合の手引き](../../guides/alphazero-network-check.md)の許容誤差に収まることを確かめる。
学習に使ったColab機のL4での同じ照合は、実行器`minase_zero run-cycles`がサイクルの手順の中で行い、2つの実行プロバイダと3つのバッチの大きさの組合せの6通りとも合格した（`cycles.jsonl`のサイクル1の行の`checks`）。

## コマンドライン

照合は[check.sh](alphazero-rl-cycle1-workstation-check/check.sh)で行った。

```console
XDG_CACHE_HOME=$HOME/.cache bash check.sh 0001
```

`check.sh`は、NASの`cycles/cycle-0001/train/model.onnx`を、`minase dev nn-eval`でCPUとTensorRTのそれぞれ、バッチの大きさ1、7、256で推論し、`nn_compare.py`で基準と比べる。
基準は、学習したColab機で学習器の`fixture-eval`が書き出したPyTorchの出力`cycles/cycle-0001/check/pytorch.json`である。
許容誤差は、CPUの出力ではlogitと価値のどちらも1×10⁻⁴、TensorRTの出力では合法手の確率が1×10⁻²、価値が2×10⁻²である。

当初は作業機でも`fixture-eval`を実行してPyTorchの出力を作り直す手順にしたが、`fixture-eval`は「チェックポイントから書き出し直したONNXのバイト列が`--onnx`のファイルと一致しない」として失敗した。
原因は「結果」の節で述べる。

## エンジン

`minase`は、ブランチ`alphazero-rl`のworktreeで、masterのコミットd786c5e1と同じ`crates/`からリリースビルドしたものである。
ネットは、実行器がサイクル1で学習した10ブロック128チャネルのネット（SHA-256 `f72e32f8fe8f85b896d3e008b22db602611375403eaddf49bdbbef1890a9e8c3`）である。
推論は、ONNX Runtime 1.28.3のCPUとTensorRTの実行プロバイダ（TensorRT 10.14.1.48、FP16）で行った。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、GPUはNVIDIA GeForce RTX 3060 Ti 8 GB、ドライバ615.78.08）で、2026年10月29日1時33分（日本時間）に実行した。
直前の負荷平均は約18であり、lishogiのBotと別の作業がCPUを使っていたが、GPUは空いていた。

## 結果

### 照合

| 実行プロバイダ | バッチ | logitの差の最大 | 合法手の確率の差の最大 | 価値の差の最大 | 最大の手の不一致 | 判定 |
|---|---|---|---|---|---|---|
| CPU | 1 | 7.3×10⁻⁶ | 3.0×10⁻⁷ | 1.1×10⁻⁶ | 0 | 合格 |
| CPU | 7 | 8.0×10⁻⁶ | 3.0×10⁻⁷ | 1.1×10⁻⁶ | 0 | 合格 |
| CPU | 256 | 8.0×10⁻⁶ | 3.0×10⁻⁷ | 1.1×10⁻⁶ | 0 | 合格 |
| TensorRT | 1 | 3.9×10⁻² | 1.1×10⁻³ | 9.4×10⁻³ | 7 | 合格 |
| TensorRT | 7 | 3.3×10⁻² | 1.6×10⁻³ | 7.9×10⁻³ | 8 | 合格 |
| TensorRT | 256 | 3.7×10⁻² | 1.9×10⁻³ | 9.9×10⁻³ | 5 | 合格 |

288局面すべてで照合し、TensorRTのエンジンの作成を含む全体の所要時間は53秒であった（[ログ](alphazero-rl-cycle1-workstation-check/check.log)）。
最大のlogitを持つ手（最大の手）の不一致は、合否の判定に使わない（手引きの5節）。

### 作業機での書き出し直しの不一致

作業機で同じチェックポイントからONNXを書き出し直すと（[onnx_diff.py](alphazero-rl-cycle1-workstation-check/onnx_diff.py)、[出力](alphazero-rl-cycle1-workstation-check/onnx_diff.out)）、計算グラフの151個の節点と70個の初期値（ONNXのinitializer。重みとバイアスの定数）の名前はColab機の`model.onnx`と一致したが、28個の初期値の数値が最大4.8×10⁻⁷だけ異なった。
最も大きく異なったのは、大きさ128の畳み込みのバイアスであった。
PyTorchのONNXへの書き出しは、評価モードのネットのBatchNormを、定数畳み込みで直前の畳み込みの重みとバイアスへ融合する。
この単精度の計算の丸めがCPUによって変わるためとみられる（Colab機はIntel Xeon、作業機はIntel Core Ultra 7 265KFで、作業機のCPUはAVX-512を持たない）。
Colab機では、学習の直後に同じ機械で`fixture-eval`がバイト列の一致を確かめており、チェックポイントと`model.onnx`の対応はそこで確認できている。
作業機での照合の目的は、GPUの機種ごとに作られるTensorRTのエンジンを確かめることなので、作業機では書き出し直さずに、Colab機のPyTorchの出力を基準にした。

## 結論

サイクル1のネットは、作業機でもONNX RuntimeのCPUとTensorRTのFP16の出力がPyTorchの出力の許容誤差に収まったので、次のサイクルの生成と、作業機での対局に使える。
ONNXの書き出しのバイト列は機械によって変わるので、`fixture-eval`は学習した機械で実行し、ほかの機械ではその出力を基準に推論だけを照合する。
