# minase-zero

本ディレクトリは、AlphaZero型探索（[docs/plans/alphazero.md](../../../docs/plans/alphazero.md)）が使うニューラルネットワーク（以下ネット）を扱うPythonのツール群である。
ネットは局面を入力として、各合法手の有望さ（方策）と手番側の勝率（価値）を出力する。
本ツールは、ネットの定義、ONNX形式への出力、ONNX Runtimeによる推論速度の計測、入力平面と方策ラベルの符号化、学習データ形式MNZTの読み込み、および学習のループを持つ。
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

MNZTの読み込みと学習の試験の一部は、Rustの自己対局生成器が書いた見本のMNZTと、同じ生成で書かれたMNSDおよびその来歴ファイルを読む。
見本の経路は環境変数 `MINASE_ZERO_SAMPLE_MNZT` で指定し、指定がなければ本体リポジトリの `data/alphazero-feasibility/sample.mnzt` を使う（worktreeから実行しても本体の `data/` を指す）。
見本がない場合、これらの試験はスキップせずに失敗する。
見本は、たとえば次のコマンドで生成する。

```bash
cargo run --release --bin minase -- data selfplay generate --games 4 --nodes 300 --seed 7 \
  --random-moves 8 --output data/alphazero-feasibility/sample.mnsd \
  --mnzt-output data/alphazero-feasibility/sample.mnzt
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

### 学習

`minase_zero.train` は、MNZTの学習データからネットを学習し、ONNXへ出力する。
学習データの形式はRustの `crates/minase/src/nn/mnzt.rs` が定めており、`minase_zero.mnzt` が開く時点でヘッダ、ファイル長、および全レコードの欄を検証して、壊れたファイルを拒否する。
複数のファイルを `--data` に並べると連結して1つの学習データとして扱い、対局は（ファイル、対局番号）の組で区別する。

```bash
uv run --project tools/train/zero python -m minase_zero.train \
  --data data/zero/sl-000.mnzt data/zero/sl-001.mnzt --out data/zero/run-sl \
  --blocks 10 --channels 128 --batch-size 1024 --epochs 1 --lr 0.02 --seed 1
```

学習と検証の分割は対局単位で行い、`--val-fraction` の割合（既定5%）の対局を切り上げた局数だけ乱択して検証に回す。
同じシードと同じファイルの並びからは同じ分割になる。
学習を始めるときに、局面数、対局数、1エポックのステップ数、および総ステップ数を標準出力へ書く。
重みの更新回数が足りるかは、この値で学習の前に確かめる（[docs/lessons/check-steps-per-epoch-before-training.md](../../../docs/lessons/check-steps-per-epoch-before-training.md)）。

損失は、方策の交差エントロピーと価値の二値交差エントロピーの重みつき和である。
方策の目標は各レコードの（ラベル、確率）の組であり、組にない手（非合法手を含む）の目標は0である。
価値の目標は `(1 − λ)·結果 + λ·根の価値` であり、結果は手番側から見た勝ち1、引き分け0.5、負け0、根の価値は記録された手番側の勝率である。
最適化はSGDであり、学習率は `--lr` を初期値として、コサインで総ステップ数までに0へ下げるか（既定）、一定に保つ。

主な引数は次のとおりである。

| 引数 | 内容 |
|---|---|
| `--data`、`--out` | 学習データのMNZTと、新しく作る実行ディレクトリ。実行ディレクトリが既にあればエラーになる |
| `--lr`、`--seed` | 学習率の初期値と、初期化、分割、およびバッチの順序のシード。どちらも必須 |
| `--blocks`、`--channels` | ネットの大きさ。既定は10ブロック128チャネル |
| `--batch-size`、`--epochs` | バッチの大きさ（既定1024）とエポック数（既定1）。各エポックの端数のバッチは捨てる |
| `--lr-schedule` | `cosine`（既定）または `constant` |
| `--momentum`、`--weight-decay` | SGDのモーメンタム（既定0.9）と重み減衰（既定1e-4） |
| `--lambda` | 価値の目標で根の価値に掛ける重みλ（既定0.333） |
| `--policy-weight`、`--value-weight` | 方策と価値の損失の重み（既定はどちらも1） |
| `--val-fraction` | 検証に回す対局の割合（既定0.05） |
| `--log-interval`、`--eval-interval`、`--checkpoint-interval` | 学習損失の記録（既定100）、検証（既定1000）、およびチェックポイントの保存（既定1000）の間隔のステップ数。検証と保存は各エポックの終わりにも行う |
| `--init` | 前の実行の `checkpoint.pt` から重みだけを読み込んで始める。強化学習で前のサイクルのネットから学習を続けるときに使う |
| `--workers` | バッチを作る `DataLoader` のワーカーの数（既定4） |
| `--device` | 学習に使う装置（既定 `cuda`）。GPUのない環境では `cpu` を明示する |

実行ディレクトリには次のファイルができる。

| ファイル | 内容 |
|---|---|
| `checkpoint.pt` | 重み、最適化器の状態、ステップ数、乱数の状態、学習の設定、学習データの経路と検査和、および開始時刻。一時ファイルへ書いてから名前を変えて確定させる |
| `log.jsonl` | 学習損失（`"kind": "train"`）と検証の指標（`"kind": "validation"`）を1行ずつ追記したもの。検証の指標は、方策の交差エントロピー、方策のlogitが最大の手と教師の確率が最大の手の一致率（`policy_agreement`）、および価値の損失である |
| `net.onnx`、`net.sha256` | 学習を終えたネットと、`sha256sum` と同じ書式の検査和 |
| `provenance.json` | 学習データの経路と検査和、`--init` の重みの検査和、学習の設定、シード、PyTorchの版、および開始と終了の時刻 |

学習損失の行の `positions_per_second` は、前の行からの経過時間で求めた毎秒の学習局面数であり、その間に行った検証と保存の時間を含む。
`policy_agreement` のネット側の手は、合法手に限らず方策の出力全体の最大である。
学習器は合法手の集合を持たないからである。

中断した学習は、実行ディレクトリを `--resume` に指定して、最後に保存したステップから続ける。
学習の設定はチェックポイントから読むので指定し直さず、`--resume` と設定の引数を同時に指定すると引数のエラーで終了する。
指定できるのは、機械に依存する `--device` と `--workers`、および学習データの置き場所が変わった場合の `--data` だけである。
学習データの検査和がチェックポイントの記録と異なる場合は、エラーで終了する。
各エポックのバッチの順序はシードとエポック番号から決まり、再開はそのエポックの途中のバッチから続くので、中断しなかった学習と同じ順序で同じ局面を読む。
`log.jsonl` のうち、最後の保存より後に書かれた行は、再開の時点で除いてから書き直す。

```bash
uv run --project tools/train/zero python -m minase_zero.train --resume data/zero/run-sl
```

## コンテナイメージ

`Dockerfile` は、レンタル機と作業機で共通に使う土台のイメージを定義する。
イメージは、CUDA 13.4、cuDNN 9.26、`uv.lock` で固定したPython環境（PyTorch 2.13、ONNX Runtime 1.28、TensorRT 10.14）、Rust 1.98、およびuvを含む。
minaseのソースもバイナリも含めないので、ソースを変えてもイメージを作り直す必要はない。
作業機でのビルドは次のとおりである。

```bash
docker build -t minase-zero tools/train/zero
```

イメージの大きさは、2026年10月9日に作業機でビルドした時点で展開後24 GB（`docker image ls`の表示）であり、レジストリへ送る圧縮後は約7 GBと見込む。uvのキャッシュをイメージに残すと約12 GB増えるので、Dockerfileは`uv sync`と同じ層で`uv cache clean`を実行する。
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

## minaseのMCTSのエンジンのキャッシュ

`minase --search mcts`は、最初の`isready`への応答の前にONNX Runtimeのセッションを作り、既定のTensorRTの実行プロバイダでは、実行ファイルに埋め込んだネットからGPU向けのエンジンを作る。
エンジンの作成には数分かかることがあり、対局ハーネス`minase match run`の応答タイムアウト（`--response-timeout`、既定120秒）を超えると、そのエンジンの反則負けになる。
作ったエンジンは、USIオプション`NnCacheDir`（既定は`$XDG_CACHE_HOME/minase/tensorrt`）の下の、ネットの検査和、GPUの機種名、メモリ容量、計算能力、およびTensorRTの版から作った鍵のディレクトリに保存され、次回からは読み込むだけになる。
このため測定の前に、測定に使うバイナリを、測定と同じ`NnCacheDir`と`NnBatchSize`で1回起動し、`readyok`が返るまで待ってキャッシュを作っておく。
ONNX Runtimeの共有ライブラリは環境変数`ORT_DYLIB_PATH`で指定し、コンテナイメージの中では設定済みである。

```bash
printf 'usi\nsetoption name NnCacheDir value data/cache/trt-minase\nisready\nquit\n' \
  | target/release/minase --protocol usi --rules engine-default --search mcts
```

## ソースの構成

| ファイル | 内容 |
|---|---|
| `src/minase_zero/model.py` | ネット `ZeroNet`。入力層の3×3畳み込み、KataGo式の大域プーリングを持つ残差ブロックの列、方策の出力（249チャネルの1×1畳み込みをRust側のラベル順へ転置して平坦化）、および価値の出力（1×1畳み込み、全結合層2つ、シグモイド）からなる |
| `src/minase_zero/export.py` | ONNXへの出力と、そのコマンドライン |
| `src/minase_zero/bench.py` | 推論速度の計測のコマンドライン |
| `src/minase_zero/encoding.py` | SFENとUSI表記の着手の読み込み、入力平面の符号化、および方策ラベルの計算 |
| `src/minase_zero/mnzt.py` | MNZTの検証と、局面の表と方策の列の `memmap` による読み込み、および複数ファイルの連結 |
| `src/minase_zero/dataset.py` | 対局単位の学習と検証の分割、局面の符号化によるバッチの作成、およびエポックごとのバッチの順序 |
| `src/minase_zero/train.py` | 学習のループ、検証、チェックポイントの保存と再開、ONNXと来歴の出力、およびそのコマンドライン |
| `tests/test_encoding.py` | Rustの参照値との照合と、手計算した入力平面とラベルの検査 |
| `tests/test_model.py` | ONNXの入出力の名前と形状、方策の並び、価値の範囲、およびPyTorchとONNX Runtimeの出力の一致の検査 |
| `tests/test_mnzt.py` | 見本のMNZTと同じ生成のMNSDとの照合、合成ファイルの往復、および壊れたファイルの拒否の検査 |
| `tests/test_dataset.py` | 分割が対局単位であること、バッチの内容と価値の目標、およびバッチの順序の再現の検査 |
| `tests/test_train.py` | 学習の損失が有限であること、中断と再開が中断しない学習と一致すること、`--init`、およびONNXと来歴の出力の検査 |
| `tests/conftest.py` | 合成のMNZTの書き出しと、見本のMNZTの場所 |
| `Dockerfile`、`.dockerignore` | コンテナイメージの定義。ビルドの文脈へは `pyproject.toml` と `uv.lock` だけを取り込む |
| `run.sh` | 作業機でコンテナを起動する手順 |
