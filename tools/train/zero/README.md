# minase-zero

本ディレクトリは、AlphaZero型探索（[docs/plans/alphazero.md](../../../docs/plans/alphazero.md)）が使うニューラルネットワーク（以下ネット）を扱うPythonのツール群である。
ネットは局面を入力として、各合法手の有望さ（方策）と手番側の勝率（価値）を出力する。
本ツールは、ネットの定義、ONNX形式への出力、ONNX Runtimeによる推論速度の計測、および入力平面と方策ラベルの符号化を持つ。
学習データ形式MNZTの読み込みと学習のループは、後続の作業で加える。
エンジン本体はRustで書かれており、学習したネットはONNXファイルを介してだけ受け渡す。
レンタル機と作業機で共通に使うコンテナイメージの定義もここに置く。

## 環境の構築

依存関係は [uv](https://docs.astral.sh/uv/) で管理し、PSTの学習ツール（`tools/train/`）とは別のプロジェクトとする。
Python 3.12以上、PyTorch 2.13、ONNX、ONNX Runtime 1.28のGPU版、およびTensorRT 10.14の版は `uv.lock` に固定されており、次のコマンドで `tools/train/zero/.venv` に同じ環境を作れる。

```bash
uv sync --project tools/train/zero
```

コマンドはすべてリポジトリのルートで実行する。
GPUの実行プロバイダ（TensorRTとCUDA）は、ONNX Runtimeの要件を満たすCUDAとcuDNNがシステムにある環境でだけ動く。
PyTorch 2.13がpipのcuDNNを9.20に固定しており、ONNX Runtime 1.28が要件とするcuDNN 9.23以上をこの環境だけでは満たせないからである。
このため、GPUを使う計測は後述のコンテナイメージの中で行い、イメージの外の `.venv` ではCPUの実行プロバイダとテストだけを使う。

## テストの実行

テストはpytestで書かれており、`tests/` にある。

```bash
uv run --project tools/train/zero pytest tools/train/zero/tests
```

`tests/test_encoding.py` の `test_fixture_matches_rust_encoding` は、Rustが書き出す参照値 `crates/minase/tests/fixtures/nn_encoding.json` を読む。
参照値がない場合、このテストはスキップせずに失敗する。
参照値は次のコマンドで生成する。

```bash
cargo test -p minase --lib nn::fixture::tests::write_nn_encoding_fixture -- --ignored --exact
```

## コマンド

### ONNXへの出力

`minase_zero.export` は、ネットを作ってONNXへ出力する。
`--weights` を省略するとランダム初期化の重みを使い、指定すると学習済みの重み（PyTorchの `state_dict`）を読み込む。

```bash
uv run --project tools/train/zero python -m minase_zero.export --blocks 10 --channels 128 --out net.onnx
```

ネットの大きさは `--blocks`（残差ブロックの数）と `--channels`（チャネル数）で与える。
設計書が比べる大きさは、10ブロック128チャネル、6ブロック96チャネル、および15ブロック192チャネルである。
10ブロック128チャネルのネットは重みが約346万個、単精度のONNXファイルで約14 MBになる。

出力したONNXの入出力は次のとおりである。
バッチの大きさNは実行時に決められる。

| 名前 | 形状 | 内容 |
|---|---|---|
| `input` | `[N, 95, 12, 12]` | 手番側から見た入力平面（float32、値は0か1） |
| `policy` | `[N, 35856]` | 方策のlogit。要素の番号はRust側のラベル `相対移動元 * 249 + 面` に一致する |
| `value` | `[N, 1]` | 手番側の勝率（0〜1） |

ONNXのopsetは18に固定している。
PyTorchのtorch.export方式の出力器が対応する最小の版であり、ONNX Runtime 1.28とTensorRT 10.14のどちらも実行できる。

### 推論速度の計測

`minase_zero.bench` は、ONNXのネットをONNX Runtimeの指定の実行プロバイダで動かし、バッチの大きさごとに毎秒の評価数（局面/秒）を測る。
実行プロバイダは `tensorrt`、`cuda`、`cpu` から選ぶ。
指定の実行プロバイダを初期化できない場合は、別の実行プロバイダへ切り替えずにエラーで終了する。

```bash
uv run --project tools/train/zero python -m minase_zero.bench \
  --model net.onnx --provider tensorrt --trt-cache-dir data/cache/trt-bench
```

主な引数は次のとおりである。

| 引数 | 内容 |
|---|---|
| `--batches` | 計測するバッチの大きさ。既定は `64,128,256,512` |
| `--warmup`、`--iterations` | 計測前の推論の回数（既定20）と、計測する推論の回数（既定100） |
| `--fp16`、`--no-fp16` | TensorRTのFP16を有効にするか。`tensorrt` だけで指定でき、既定は有効 |
| `--trt-cache-dir` | TensorRTのエンジンのキャッシュを置くディレクトリ。2回目からはエンジンの作成を省ける |
| `--verbose` | ONNX Runtimeの詳細ログとTensorRTの詳細なビルドログを標準エラーへ出す |

FP16で動いていることは、`--fp16` と `--no-fp16` の速度の差と、`--verbose` のログで確かめる。
TensorRTのエンジンは、バッチの大きさ1から `--batches` の最大値までを受け付ける1つの最適化プロファイルで作り、最適化の基準を最大値に置く。

結果は1行1条件のTSVで標準出力へ書く。

| 列 | 内容 |
|---|---|
| `provider`、`fp16` | 実行プロバイダと、TensorRTのFP16の有無（他の実行プロバイダでは `-`） |
| `batch`、`warmup`、`iterations` | バッチの大きさと推論の回数 |
| `seconds`、`positions_per_second` | 計測した推論の合計秒数と、毎秒の評価数 |
| `gpu_memory_used_mib` | そのバッチの計測を終えた時点のGPU 0のメモリの使用量（MiB） |
| `gpu_memory_baseline_mib` | セッションを作る前のGPU 0のメモリの使用量（MiB） |

GPUのメモリの使用量は `nvidia-smi --query-gpu=memory.used` で測ったGPU全体の値であり、他のプロセスの分も含む。
計測によるメモリの増分は、2つの列の差で読む。
ONNX Runtimeは確保したメモリを返さないので、1回の実行で複数のバッチを測ると、各行の値はそれまでのバッチの最大の使用量を含む。

### 入力の符号化と方策ラベル

`minase_zero.encoding` は、Rustの `crates/minase/src/nn/encoding.rs` と `nn/policy.rs` と同じ入力平面と方策ラベルをPythonで計算する。
`parse_sfen` は2欄の基本形と、先獅子の升を含む4欄または5欄の拡張SFENを読み、`encode` は局面を `[95, 12, 12]` の入力平面へ、`move_label` はUSI表記の着手を方策ラベルへ変換する。
面と方向の割当はRust側の `nn/policy.rs` の説明が定めており、両者の一致は前述の参照値のテストで確かめる。

## コンテナイメージ

`Dockerfile` は、レンタル機と作業機で共通に使う土台のイメージを定義する。
イメージは、CUDA 13.4、cuDNN 9.26、`uv.lock` で固定したPython環境（PyTorch 2.13、ONNX Runtime 1.28、TensorRT 10.14）、Rust 1.98、およびuvを含む。
minaseのソースもバイナリも含めないので、ソースを変えてもイメージを作り直す必要はない。
作業機でのビルドは次のとおりである。

```bash
docker build -t minase-zero tools/train/zero
```

イメージの大きさは、展開後で約15 GB、レジストリへ送る圧縮後で約7 GBと見込む。
内訳の見込みは、基底イメージ（CUDAとcuDNN）が展開後で約5.5 GB（圧縮後2.4 GB）、Python環境が約8.9 GB（TensorRTの共有ライブラリ3.9 GB、PyTorchが同梱するCUDAとcuDNNのライブラリ2.7 GB、PyTorch本体1.1 GBを含む）、Rustのツールチェーンが約0.6 GBである。
CUDAとcuDNNは基底イメージとPyTorchのwheelの両方に入って重複するが、ONNX Runtimeが要するcuDNN 9.23以上はPyTorchのwheelの版では満たせないので、この重複を受け入れる。

イメージの中では、ONNX Runtimeの共有ライブラリの場所を環境変数 `ORT_DYLIB_PATH` が指し、TensorRTの共有ライブラリは動的リンカの検索パスに登録されている。
Rustのminaseは、この2つによってイメージのONNX Runtimeを実行時に読み込む。
Python環境は `/opt/zero-venv` にあり、`uv run --project tools/train/zero` はこの環境を使う。

作業機では `run.sh` でコンテナを起動する。
`run.sh` は、GPUをコンテナへ渡し、リポジトリを `/work` へ、本体リポジトリの `data/` を `/work/data` へマウントし、ホストの利用者のuidで動かす。
TensorRTのエンジンのキャッシュは `data/cache/` に置かれて起動をまたいで再利用され、Cargoの成果物はホストのビルドと混ざらないよう `target/zero-image/` に分かれる。
作業機には、あらかじめNVIDIA Container Toolkitを導入しておく。

```bash
tools/train/zero/run.sh uv run --project tools/train/zero python -m minase_zero.bench \
  --model data/zero/net.onnx --provider tensorrt --trt-cache-dir data/cache/trt-bench
```

## ソースの構成

| ファイル | 内容 |
|---|---|
| `src/minase_zero/model.py` | ネット `ZeroNet`。入力層の3×3畳み込み、KataGo式の大域プーリングを持つ残差ブロックの列、方策の出力（249チャネルの1×1畳み込みをRust側のラベル順へ転置して平坦化）、および価値の出力（1×1畳み込み、全結合層2つ、シグモイド）からなる |
| `src/minase_zero/export.py` | ONNXへの出力と、そのコマンドライン |
| `src/minase_zero/bench.py` | 推論速度の計測のコマンドライン |
| `src/minase_zero/encoding.py` | SFENとUSI表記の着手の読み込み、入力平面の符号化、および方策ラベルの計算 |
| `tests/test_encoding.py` | Rustの参照値との照合と、手計算した入力平面とラベルの検査 |
| `tests/test_model.py` | ONNXの入出力の名前と形状、方策の並び、価値の範囲、およびPyTorchとONNX Runtimeの出力の一致の検査 |
| `Dockerfile`、`.dockerignore` | コンテナイメージの定義。ビルドの文脈へは `pyproject.toml` と `uv.lock` だけを取り込む |
| `run.sh` | 作業機でコンテナを起動する手順 |
