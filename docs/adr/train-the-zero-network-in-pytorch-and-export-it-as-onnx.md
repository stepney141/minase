# AlphaZero型探索のネットはPyTorchで学習し、ONNXへ出力してminaseで推論する

## ステータス

承認済み。決定日は2026年10月26日。

## コンテキスト

AlphaZero型探索の強化学習（RL）では、自己対局のデータでネットを学習し直して次のサイクルの生成に使う。
ネットは大域プーリングを入れた10ブロック128チャネルのResNetから始め（[ネットの構成のADR](start-the-zero-network-as-a-10-block-128-channel-resnet-with-global-pooling-and-a-32-channel-value-head.md)）、minaseはONNXのファイルをONNX Runtimeで推論する（[推論のADR](run-zero-network-inference-through-ort-load-dynamic.md)）。
minaseの学習器`crates/minase-trainer`はRustで書かれ、NNUE、PST、およびFMを評価関数ごとのパイプラインで学習し、NNUEのGPUカーネルをCUDA C++の実行時コンパイル（NVRTC）で実装している（[Rust製NNUE学習器](../plans/archived/nnue-trainer-rust.md)）。
ただし、ResNetの学習に要る畳み込み、バッチ正規化、大域プーリング、およびそれらの逆伝播のカーネルは持たない。
NNUEの学習器がRustで得た速さは、疎な入力特徴の収集と逆索引の設計に由来し（[調査](../research/nnue-trainer-speed.md)）、入力が密なResNetには当てはまらない。
エンジンの開発者は、[学習器の設計書](../plans/archived/alphazero-trainer.md)でこの決定を下した。

## 決定

ネットはPyTorchで学習し、学習したネットをONNXへ出力して、minaseがそれを推論する。
学習器はPythonのパッケージ`tools/train/zero/minase_zero/`とし、依存は環境定義`tools/train/zero/`のuvのロックファイルで固定する（[実行環境の導入のADR](install-the-zero-toolchain-from-one-lock-on-every-machine.md)）。
ネットの定義とONNXへの出力の設定は`minase_zero.net`の1か所に置き、実現性の測定に使ったスクリプト（`feasibility/make_nets.py`）と参照推論のスクリプト（`nn_torch_reference.py`）もそこから読み込む。

## 検討した代案

`minase-trainer`にResNetの学習を加え、畳み込みなどのカーネルをNVRTCで自作する案は採らなかった。
カーネルの実装と検証が本計画の範囲を超え、cuDNNを通じてこれらの演算を備えるPyTorchで済むからである。

JAXで書く案は採らなかった。
参照した将棋系の実装に採用例がなく、dlshogiと技巧はPyTorchからONNXまたはTensorRTへ出力する経路を実運用している（[調査](../research/alphazero-prior-art.md)）。

lc0の学習器lczero-trainingを使う案は採らなかった。
TensorFlowで書かれ、lc0に固有の学習データの形式に結びついているからである。

## 結果

学習器はPythonとPyTorchに依存するが、minaseのビルドはPyTorchに依存しない。
ネットの入力の符号化と方策のラベルは、Rustの`nn`とPythonの`minase_zero.encoding`の両方に実装され、両者の一致は固定局面集`tools/train/zero/fixtures/nn-positions.jsonl`による試験で確かめる。
学習したネットは、使う前に、PyTorch、ONNX RuntimeのCPU、およびTensorRTのFP16の出力を[ネットの出力の照合の手引き](../guides/alphazero-network-check.md)の手順で照合する。
撤去した以前のPython学習器（[撤去の設計書](../plans/archived/python-trainer-retirement.md)）とは別の学習器であり、NNUE、PST、およびFMの学習は引き続き`minase-trainer`が担う。
