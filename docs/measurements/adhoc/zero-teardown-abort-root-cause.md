# 終了処理での異常終了の原因の調査

## 目的

AlphaZero型探索の推論を使う`minase dev nn-eval`と自己対局の生成器が、出力を書き終えた後の終了処理で`corrupted double-linked list`を出して異常終了することがある（[ONNX Runtimeの版のADR](../../adr/load-onnx-runtime-1-31-1-through-ort-rc13-for-zero-inference.md)）。
本記録は、この異常終了を作業機で再現し、ヒープを壊している書き込みとその呼び出し元をgdbで特定する。
あわせて、異常終了が1回も観測されていなかった作業機の現行の構成で、同じ書き込みが起きているかを確かめる。

調査は2026年11月11日に行った。どの設計書も計画していない調査なので、本記録は`adhoc/`に置く。

## コマンドライン

再現用のスクリプトは[zero-teardown-abort-root-cause/](zero-teardown-abort-root-cause/)にある。
`<zero>`は導入スクリプト`setup.sh`で準備した`tools/train/zero`、`<旧>`と`<現行>`は次節の2つの実行ファイル、`<新形式>`と`<旧形式>`は次節の2つのネットである。

終了コードを数える実行は次の形である。`--backend`、`--batch`、ONNX Runtimeの版、実行ファイル、およびネットを、結果の表のとおりに替えた。

```console
python3 repeat.py --zero-dir <zero> --ort-version 1.28.3 --runs 12 -- \
  <旧> dev nn-eval --nn-model <旧形式> --backend cuda --batch 7 --out out.json
```

gdbでの調べは、次の4つである。

1. 異常終了の時点のスタックと、glibcが壊れていると判定した領域の中身を表示する（出力は[gdb-corrupted-chunk.txt](zero-teardown-abort-root-cause/gdb-corrupted-chunk.txt)）。
2. `exit`に入った時点で、壊れる8バイトにハードウェア監視点（`watch *(long*)0x555556be3fc8`）を置き、書き込みのたびにスタックを記録する（抜粋は[gdb-watchpoint-excerpt.txt](zero-teardown-abort-root-cause/gdb-watchpoint-excerpt.txt)）。
3. `exit`に入った時点で、gdbから`ort::environment::release_env_on_exit`を呼んでから続行する。
4. 2で見つけた関数の入口で、その関数が手放そうとする領域がすでに解放済みかを読む。

4は、次の2つのコマンドで行う。1つ目が、ONNX RuntimeのCUDAの実行プロバイダの共有ライブラリから関数と静的変数の番地を探し、2つ目がその番地をgdbの下で調べる。

```console
python3 find_registry_reset.py <zero>/.runtime/onnxruntime-linux-x64-gpu_cuda13-1.31.1/lib/libonnxruntime_providers_cuda.so
python3 probe.py --zero-dir <zero> --ort-version 1.31.1 --function 0x3cfdc0 --static 0x10edb8c0 -- \
  <現行> dev nn-eval --nn-model <新形式> --backend cuda --batch 7 --out out.json
```

## エンジン

| 呼び名 | コミット | 内容 |
|---|---|---|
| 現行 | `645d37c4`（調査の時点のmaster） | 入力がuint8、方策が16,108ラベルのFP16で、出力を`IoBinding`で受け取る |
| 旧 | `982d772d`（2026年10月27日） | 推論を初めて入れたコミット。入力は95面のfloat32で、`session.run`で出力を受け取る |

どちらも`cargo build --release --bin minase`に、デバッグ情報を付ける`CARGO_PROFILE_RELEASE_DEBUG=1`を加えてビルドした。
ONNX RuntimeのRustバインディングであるortクレートは、どちらも`=2.0.0-rc.13`である。

ネットは2つ使った。
新形式は、2回目の強化学習の初期ネット（20ブロック256チャネル、SHA-256の先頭は`7a9f3ec7`）である。
旧形式は、同じ重みを159面のfloat32の入力と35,856ラベルのfloat32の方策で書き出したもの（SHA-256の先頭は`8be01e2e`）である。

旧のコードは入力が95面のネットを前提とするので、旧形式のネットも入出力の不一致として拒む。
このとき旧のコードは、CUDAのセッションを作り、入出力の形を検査し、セッションを破棄して、推論を1回もせずに終了コード1で終わる。
本記録はこの経路を「推論なしの経路」と呼ぶ。現行のコードに旧形式のネットを渡したときも、同じ経路を通る。

## 環境

作業機（Intel Core Ultra 7 265KF、NVIDIA GeForce RTX 3060 Ti 8 GB、ドライバ615.78.08、Arch Linux、glibc 2.44）で、gdb 18.1、ONNX Runtime 1.28.3と1.31.1のCUDA 13版、およびTensorRT 10.14.1を使った。
ドライバとglibcは2026年10月21日から同じ版である。借りた機械は使っていない。

## 結果

### 作業機での再現

gdbを使わない実行の終了コードは次のとおりであった。異常終了は、どれも標準エラーに`corrupted double-linked list`を出してSIGABRTで終わった。

| 実行ファイル | ONNX Runtime | 実行 | 回数 | 異常終了 |
|---|---|---|---|---|
| 現行 | 1.28.3 | 新形式のネットの推論（CUDAのバッチ1、7、256と、エンジンを新しく作るTensorRTのバッチ1） | 28 | 0 |
| 現行 | 1.28.3 | 推論なしの経路（CUDA） | 12 | 0 |
| 現行 | 1.31.1 | 推論なしの経路（CUDA） | 12 | 0 |
| 旧 | 1.28.3 | 推論なしの経路（CUDA） | 12 | 8 |
| 旧 | 1.31.1 | 推論なしの経路（CUDA） | 12 | 3 |

旧のコードでは、ONNX Runtime 1.31.1でも、ドライバ615.78.08の作業機で異常終了が起きた。
推論を1回もしない経路で起きるので、推論の実行は異常終了の条件ではない。

gdbの下では、旧のコードとONNX Runtime 1.28.3の推論なしの経路は、最後まで走らせた6回のすべてで異常終了し、glibcが壊れていると判定した領域の番地は6回とも`0x555556be3fb0`であった。
gdbはアドレス空間の配置のランダム化を切るので、ヒープの配置が毎回同じになる。

### 壊れ方

glibcは、解放済みの領域を大きさごとの双方向リストでつなぎ、リストから外すときに前後のポインタの整合を検査する。
異常終了の時点で、番地`0x555556be3fb0`の領域（大きさ80バイト）の後ろ向きのポインタは`0x55555628ae7f`であった。
領域は16バイト境界に並ぶので、正しいポインタの下位4ビットは0である。この値は、正しい値`0x55555628ae80`から1を引いたものである。

### 書き込んだコード

監視点は31,068回の書き込みを記録し、glibcの外からの書き込みは最初と最後の2回だけであった。

`exit`に入った時点では、この領域はC++の`std::shared_ptr`の管理ブロックとして生きており、参照カウントは強い参照が1、弱い参照が1であった。
1回目の書き込みは、`exit`が終了時の処理を順に呼ぶ関数（`__run_exit_handlers`）から直接呼ばれた`libonnxruntime_providers_cuda.so`の中のコードで、参照カウントを0にした。その直後にglibcがこの領域を解放した。
最後の書き込みは、同じ`libonnxruntime_providers_cuda.so`の別の関数で、解放済みの領域の8バイト目を1減らした。
この関数の呼び出し元は、新しい順に、`libonnxruntime.so`、ortクレートの`Environment`の破棄、`ort::environment::release_env_on_exit`、動的リンカの終了処理（`_dl_fini`）、`__run_exit_handlers`であった。
この減算の後、`libcuda.so.1`の終了処理が呼んだ`free`が、隣の領域をまとめるためにこの領域をリストから外そうとして、不整合を検出した。
これまでの記録のスタックに出ていた`libcuda.so.1`は、ヒープを壊した側ではなく、検出の引き金になった側である。

最後の書き込みをした関数は、静的な`shared_ptr`の16バイトを0で埋め、その管理ブロックの参照カウントを減らすだけの関数である。
ONNX Runtime 1.31.1のソースでは、CUDAの実行プロバイダが、演算の実装の登録表を静的変数`static std::shared_ptr<KernelRegistry> s_kernel_registry;`に持つ（[`cuda_execution_provider.cc`](https://github.com/microsoft/onnxruntime/blob/v1.31.1/onnxruntime/core/providers/cuda/cuda_execution_provider.cc) 3237行）。
`DeleteRegistry()`がこれを`reset()`し（同3247〜3249行）、プロバイダの`Shutdown()`が`DeleteRegistry()`を呼ぶ（[`cuda_provider_factory.cc`](https://github.com/microsoft/onnxruntime/blob/v1.31.1/onnxruntime/core/providers/cuda/cuda_provider_factory.cc) 302〜304行）。
`Shutdown()`は、環境の破棄（`OrtEnv::~OrtEnv`、[`ort_env.cc`](https://github.com/microsoft/onnxruntime/blob/v1.31.1/onnxruntime/core/session/ort_env.cc) 38〜42行）が呼ぶ`UnloadSharedProviders()`から、`ProviderLibrary::Unload()`を通って呼ばれる（[`provider_bridge_ort.cc`](https://github.com/microsoft/onnxruntime/blob/v1.31.1/onnxruntime/core/session/provider_bridge_ort.cc) 2060〜2080行、2139〜2151行）。
ONNX RuntimeのライセンスはMITである。

### 原因

原因は、プロセスの終了時に、登録表の`shared_ptr`が2回手放されることである。

glibcの`exit`は、`atexit`と`__cxa_atexit`で登録された処理を、登録の逆順に呼ぶ。C++の静的変数のデストラクタは、その変数の初期化のときに`__cxa_atexit`で登録される。
実行ファイルと共有ライブラリの`.fini_array`を呼ぶ`_dl_fini`は、プロセスの起動時、`main`が始まる前に登録される。
minaseは、ONNX Runtimeの共有ライブラリを推論器の初期化のときに読み込み、ONNX RuntimeはCUDAの実行プロバイダの共有ライブラリをセッションの作成のときに読み込む。
これらのライブラリの静的変数のデストラクタは`_dl_fini`より後に登録されるので、`_dl_fini`より先に走る。
このため、実行ファイルの`.fini_array`の関数は、実行中に読み込んだこれらのライブラリの静的デストラクタより後に走る。プロセスの起動時に読み込まれる共有ライブラリの静的デストラクタは、この順序に当てはまらない。

ortクレートのrc.13は、ONNX Runtimeの環境を解放する`ReleaseEnv`を、実行ファイルの`.fini_array`に置いた関数`release_env_on_exit`から呼ぶ（`src/environment.rs` 75〜91行）。
その結果、終了時の処理は次の順になる。

1. `s_kernel_registry`の静的デストラクタが、登録表を解放する。デストラクタはポインタを0に戻さないので、静的変数は解放済みの管理ブロックを指したまま残る。
2. `.fini_array`の`release_env_on_exit`が`ReleaseEnv`を呼び、`OrtEnv`の破棄がCUDAの実行プロバイダの`Shutdown()`を呼ぶ。
3. `Shutdown()`が`s_kernel_registry.reset()`を実行し、解放済みの管理ブロックの参照カウントを1減らす。

ortクレートのソースのコメントは、`.fini_array`に置けばC++のデストラクタより前に走ると説明しているが、実行中に読み込んだライブラリについては、glibcでの順序は逆である。
ONNX Runtimeのソースにも、環境がほかのグローバル変数より後に破棄されると`ProviderLibrary::Unload()`で異常終了するという趣旨のコメントがある（`provider_bridge_ort.cc` 2061〜2062行）。

### 環境を先に解放する介入

異常終了する構成（旧のコード、ONNX Runtime 1.28.3、推論なしの経路、gdbの下）で、`exit`に入った時点でgdbから`release_env_on_exit`を呼び、環境を静的デストラクタより先に解放させた。
6回のすべてが、異常終了せずに終了コード1で終わった。介入をしない同じ構成は、6回のすべてで異常終了している。

### 現行の構成で起きていること

現行のコード、ONNX Runtime 1.31.1、および新形式のネットの推論で、登録表を手放す関数（番地`0x3cfdc0`）の入口の状態を読んだ。
CUDAとTensorRT（保存済みのエンジンを読む実行）のどちらでも、この関数は`exit`に入った後に呼ばれ、その時点で静的変数は`exit`に入った時点と同じ管理ブロックを指していた。
管理ブロックの中身は、`exit`の時点では参照カウントが強い参照1と弱い参照1であったが、関数の入口ではヒープの番地に変わっており、領域はすでに解放されていた。
どちらの実行も、終了コード0で正常に終わった。

したがって、解放済みの領域への書き込みは、異常終了が観測されていない作業機の現行の構成でも、実行のたびに起きている。

### 異常終了が起きる条件

書き込みが異常終了になるかどうかは、書き込みの時点で解放済みの領域がglibcのどこにあるかで決まる。
glibcは、解放された小さな領域を、まずスレッドごとのキャッシュ（tcache）か単方向のリスト（fastbin）に置く。これらの領域では8バイト目は後ろ向きのポインタではないので、1を引かれても検査にかからない。
領域が双方向リストに移っていると、8バイト目は後ろ向きのポインタであり、1を引かれるとリストが壊れる。その後の`free`か`malloc`がこの領域に触れたときに、異常終了になる。

領域がどこにあるかは、プロセスがそれまでに行ったメモリの確保と解放の列で決まり、ネットの大きさ、推論のコード、GPUの枚数、およびライブラリの版で変わる。
同じ作業機、同じドライバ、同じONNX Runtimeで、旧のコードだけが異常終了したのは、このためである。

## 結論

終了処理での異常終了の原因は、ortクレートのrc.13が`ReleaseEnv`を`.fini_array`から呼ぶために、実行中に読み込まれたONNX RuntimeのCUDAの実行プロバイダの登録表が、静的デストラクタで解放された後にもう一度手放されることである。
NVIDIAのドライバは原因ではなく、作業機で異常終了が観測されなかったのは、同じ書き込みが検出されないヒープの配置になっていたからである。
借りた機械での異常終了が同じ原因であることは、メッセージ、検出された箇所、およびライブラリの構成の一致から推定したものであり、借りた機械では確かめていない。
