# AlphaZero型探索のネットの推論は、ortクレートのload-dynamicでONNX Runtimeを実行時に読み込み、既定ではTensorRTのFP16で行う

## ステータス

承認済み。決定日は2026年10月27日。

## コンテキスト

学習器はPyTorchでネットを学習してONNX形式で書き出し、minaseはそれをRustから推論する。
対局ハーネスの`commit:`指定は測定のたびに`git archive`で展開したソースから`cargo build --release --bin minase`を実行し、lishogi BotのDockerイメージもGPUのない環境で同じビルドを行う。
このため、推論のための依存がビルド時にCUDA、cuDNN、TensorRT、またはONNX Runtimeを要求すると、これらのビルドが壊れる。
minaseは`unsafe_code = "forbid"`なので、自前のFFIを書けない。
ONNX Runtime 1.28のTensorRT実行プロバイダは、TensorRT 11以降に対してビルドした場合にFP16の設定を無効にする（`tensorrt_execution_provider.cc` 1651〜1659行）。
ONNX Runtime 1.28の公式のビルドは、TensorRT 10.14.1.48を使っている。
[実現性の実測](../measurements/alphazero-environment/alphazero-feasibility.md)では、ONNX Runtime 1.28.3の`IoBinding`は出力をCUDAのピン留めメモリへ束縛すると実行時にエラーを返し、ページング可能なメモリへの束縛は`session.run`より遅いか、ヒープの破壊で異常終了した。
決定の経緯は[ネットの入出力と推論の設計書](../plans/archived/alphazero-network.md)にある。

## 決定

推論は、ONNX RuntimeのRustバインディングortを`load-dynamic`機能つきで使い、ONNX Runtimeの共有ライブラリを実行時に環境変数`ORT_DYLIB_PATH`のファイルから読み込む。
`ORT_DYLIB_PATH`が設定されていなければエラーとし、ortの既定の探索パスに頼らない。
実行プロバイダはTensorRTのFP16、CUDA、およびCPUから明示的に選び、選んだものを初期化できない場合も、部分グラフをCPUで実行できない場合も、別の実行プロバイダへ切り替えずにエラーとする。
TensorRTは10系に固定し、推論は`session.run`の経路で行う。
ortの版は、2.0系のリリース候補のうち`rust-version`が1.88の`=2.0.0-rc.13`に固定する。
依存は常に含め、cargoのfeatureで切り替えない。

## 検討した代案

tchは、PyTorchのC++版ライブラリlibtorchとC++のビルドを要するので採らなかった。
candleは、CUDAのコンパイラnvccを要するので採らなかった。
どちらもGPUのない環境でのハーネスとDockerイメージのビルドを壊す。

TensorRTの直接バインディングは、保守されている安全な候補がなく、minaseは自前のFFIを書けないので採らなかった。

ortの依存をcargoのfeatureで切り替える案は採らなかった。
load-dynamicならビルド時の負担は純Rustの依存の追加だけであり、featureにするとハーネスの`commit:`ビルドへ機能名を渡す仕組みが別に要る。
ビルド時間とバイナリの大きさの増加は[ortの依存によるビルド時間と実行ファイルの増加](../measurements/alphazero-network/ort-build-cost.md)で測った。

`IoBinding`で入出力をあらかじめ束縛する経路は、上の実測のとおり、実行に失敗するか、`session.run`より遅いか、異常終了したので採らなかった。

ortのrc.14以降は、`rust-version = "1.92"`とONNX Runtime 1.31を要求するので、minaseの`rust-version`を上げずに済むrc.13にとどめた。

## 結果

minaseのビルドは、GPUのない環境でもONNX Runtimeのない環境でも通る。
推論を使う起動には、`tools/train/zero/env.sh`が設定する`ORT_DYLIB_PATH`と、TensorRT、CUDA、およびcuDNNの共有ライブラリへの`LD_LIBRARY_PATH`が要る。
ortをrc.14以降へ上げるときは、minaseの`rust-version`の引き上げと、環境定義のONNX Runtimeの更新を同じ変更で行う。
TensorRT 11以降へ移るには、単精度のONNXにFP16の設定を付ける方式をやめ、FP16で書き出したONNXを使う形に改める必要がある。
A100のColab機では、`session.run`の経路でもセッションの破棄か終了処理の時点でヒープの破壊が検出されたことがあり、作業機での終了処理は[ランダム初期化のネットの出力の照合](../measurements/alphazero-network/random-net-output-check.md)で確かめた。
