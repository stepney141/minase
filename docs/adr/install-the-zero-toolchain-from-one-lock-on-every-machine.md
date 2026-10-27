# AlphaZero型探索の実行環境は、独自のコンテナイメージを作らず、同じロックファイルと固定した版を用いてすべての機械へ導入する

## ステータス

承認済み。決定日は2026年10月27日。

## コンテキスト

AlphaZero型探索の学習と自己対局は、Colab機、vast.aiやRunpodで借りる機械、および作業機で動かす（[Colab機のADR](rent-colab-gpus-through-the-cli-and-keep-state-on-the-nas.md)）。
同じネットの出力が機械によって変わらないように、PyTorch、TensorRT、ONNX Runtime、およびRustのツールチェーンの版をそろえる必要がある。
Colab機ではコンテナを使えず、vast.aiやRunpodの機械は事業者が用意したコンテナの中で動く。
作業機（Arch Linux、glibc 2.44）でビルドしたバイナリは、Colab機（Ubuntu 24.04、glibc 2.39）より新しいglibcに依存し得る。
[実現性の実測](../measurements/alphazero-environment/alphazero-feasibility.md)では、新しいColab機での導入に81〜214秒、minaseのリリースビルドに40〜82秒かかった。

## 決定

Pythonの依存は`tools/train/zero/`のuvのロックファイルで固定し、TensorRTはNVIDIAのパッケージインデックス（pypi.nvidia.com）にある`tensorrt-cu13-libs`から取る。
Rustのツールチェーンはrustupで固定した版を入れ、ONNX RuntimeはMicrosoftのCUDA 13版の配布物を、検査和を照合してから置く。
導入は`tools/train/zero/setup.sh`が行い、vast.aiやRunpodでは事業者が公開するUbuntuとCUDAのイメージの上で同じスクリプトを実行する。
借りた機械には`git archive`で作ったソースの書庫を送り、その機械でビルドする。

## 検討した代案

独自のコンテナイメージを作る案は、Colab機で使えないので採らなかった。
機械ごとに版を決めて導入する案は、機械の間で推論の結果を比べられなくなるので採らなかった。
借りた機械でリポジトリを直接取得する案は、借りた機械にリポジトリの認証情報を置くことになるので採らなかった。
作業機でビルドしたバイナリを送る案は、glibcの版の違いで動かないおそれがあるので採らなかった。

## 結果

新しい機械を借りるたびに、導入とビルドにかかる数分がサイクルの所要時間に加わる。
GPUドライバ、OS、およびglibcは機械ごとに異なるので、その差が推論の結果に影響しないことは、PyTorch、ONNX RuntimeのCPU、およびTensorRTのFP16の出力の照合で確かめる（[ネットの入出力と推論](../plans/archived/alphazero-network.md)の検証の節）。
版を上げるときは、ロックファイルと`tools/train/zero/env.sh`の版を同じコミットで変える。
