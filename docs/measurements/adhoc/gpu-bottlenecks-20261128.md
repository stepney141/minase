# GPU計算ボトルネック調査の基礎測定（2026年11月28日）

## 目的

この記録は、`crates/minase-trainer`の差分更新型ニューラルネットワーク評価関数NNUE学習器、`crates/minase`の対局用モンテカルロ木探索PV-MCTS、および自己対局生成器`zero-selfplay`におけるGPU計算時間と飽和点を切り分けるための基礎測定をまとめたものである。

## コマンドライン

`minase-trainer`の`bench-step`計測には、構造種別`layerstack`または`simple`とデータ供給スレッド数`--threads`（1、4、8、および16）を切り替えて次のコマンドを実行した。

```text
target/release/minase-trainer nnue bench-step layerstack --batch 16384 --seed 1 --teacher-cp-per-logit 733.1159421542734 --lambda 1.0 --steps 50 --threads 16 --profile --data data/nnue-generation-iteration/gen-1/teacher/book-0.mnsd
target/release/minase-trainer nnue bench-step simple --batch 16384 --seed 1 --teacher-cp-per-logit 733.1159421542734 --lambda 1.0 --steps 20 --threads 16 --profile --data data/nnue-generation-iteration/gen-1/teacher/book-0.mnsd
```

ここで`--teacher-cp-per-logit`は教師データの勝率尺度係数`K_t`、また`--lambda`は勝率損失と勝敗結果損失の混合比を指す。
1スーパーバッチ（1チェックポイント間隔の更新まとまり）の短時間学習は、`LayerStack`で`--batches-per-superbatch 128`かつ`--threads`を4と16、`Simple`で`--batches-per-superbatch 48`かつ`--threads 16`に指定して実行した。

```text
target/release/minase-trainer nnue train layerstack --data data/nnue-generation-iteration/gen-1/teacher/book-0.mnsd --rules lishogi --teacher-cp-per-logit 733.1159421542734 --lambda 1.0 --batch 16384 --superbatches 1 --batches-per-superbatch 128 --lr-schedule cosine:1:0.027 --validation-superbatches 1 --seed 1 --device cuda --threads 16 --output-dir <OUT_DIR>
target/release/minase-trainer nnue train simple --data data/nnue-generation-iteration/gen-1/teacher/book-0.mnsd --rules lishogi --teacher-cp-per-logit 733.1159421542734 --lambda 1.0 --batch 16384 --superbatches 1 --batches-per-superbatch 48 --lr-schedule cosine:1:0.027 --validation-superbatches 1 --seed 1 --device cuda --threads 16 --output-dir <OUT_DIR>
```

カーネル単体のCUDAイベント計測には[kernel_probe.py](gpu-bottlenecks-20261128/kernel_probe.py)を用い、コンパイル指定`sm_86`および算術オプション`--fmad=false --prec-div=true --prec-sqrt=true --ftz=false`の下でバッチサイズ16,384上の各カーネル時間を記録した。

PV-MCTS用の20ブロック256チャンネル網は`minase_zero init`で初期化および書き出しを行い、推論単体計測`nn-eval`、CPUダミー探索`bench`、汎用将棋プロトコルUSI（Universal Shogi Interface）対話による固定局面探索、および`zero-selfplay`は[mcts_probe.py](gpu-bottlenecks-20261128/mcts_probe.py)へまとめた次のコマンド形式で実行した。

```text
tools/train/zero/.venv/bin/python -m minase_zero init --blocks 20 --channels 256 --seed 20261116 --out-dir <MODEL_DIR> --trainer-commit 394b5583
target/release/minase data zero-selfplay --output-dir <OUT_DIR> --nn-model <MODEL_ONNX> --nn-backend tensorrt --generation-commit <COMMIT40> --seed 9100000000 --start-position shuffled-setup --games 512 --target-positions 1000000 --chunk-games 512 --simulations 4 --resign-threshold 0 --threads 4 --devices 1 --games-per-thread 128
```

USI対局探索の条件は`RuleSet`を`lishogi`、`NnCpPerLogit`を`1072.65`とし、`NnBackend`を`cuda`または`tensorrt`、`NnBatchSize`を1、16、128、または256へ変えて`startpos`での`go nodes 256`によるウォームアップ後にフィクスチャ局面を評価した。
評価局面には`nn-fixture out`で出力した288局面JSONL（および8局面切片`20..28`、ならびに10番目以降の切片`10..40`から確定詰み等を除いた最初の非確定中盤5局面）と、`bench`内蔵の15局面を使用した。
各測定値の機械可読な控えは[results.json](gpu-bottlenecks-20261128/results.json)に保存している。

## エンジン

測定対象のコード基準は、`master`のコミット`394b5583`であり、エンジン表示版は`minase 3.1.1`である。
規則セットには`lishogi`を用い、Efficiently Updatable Neural Networkを表すNNUE学習器とPolicy-Value Monte Carlo Tree Searchを表すPV-MCTSを含む`minase`本体の双方を`--release`ビルドで実行した。

評価対象のPV-MCTS用ネットワークは20ブロック256チャンネルの残差ネットワーク`ZeroNet`である。
パラメータ数は25,182,204、推論の演算量は1局面あたり7.24 GFLOP（10⁹回の浮動小数点演算）、初期化シードは`20261116`である。
モデルのSHA-256（Secure Hash Algorithm 256-bit）ハッシュ値は`4b430a4414d250f96094c475dc489efbc865afa20e1c9bfc5b2a958cae4abf07`である。

## 環境

計算機は物理20コアかつ論理20コアおよび主記憶30 GiBを備えるIntel Core Ultra 7 265KFと、ビデオメモリVRAMを8,192 MiB備えるNVIDIA GeForce RTX 3060 Tiである。
RTX 3060 Tiの仕様はL2キャッシュ3.0 MB、メモリ帯域448 GB/s、単精度演算性能FP32が16.2 TFLOPS（テラ浮動小数点演算毎秒）、倍精度演算性能FP64が0.253 TFLOPS、および半精度FP16 Tensor Core演算性能が64.8 TFLOPSである。

ソフトウェア基盤はNVIDIAドライバ`620.73.04`、CUDA `13.5`、およびONNX Runtime `1.31.1`である。
lishogiのBotのコンテナは測定中も止めていなかった（2026年11月28日の`docker events`に停止の記録がない）。
Botは通信対局を10分ごとに確かめ、そのたびに数秒間エンジンのプロセスを使っていた（Botのログの`Process Used`と`free_process`）。
BotのエンジンはCPUで動くαβ探索なので、GPUのカーネルの時間への影響は小さいとみられるが、CPUで組む教師のバッチの供給速度には影響した可能性がある。
対局比較専用の`USI_Hash`、同時対局数、勝敗記録のペンタノミアル度数、対数尤度比LLR（Log-Likelihood Ratio）、および時間切れ数`time_forfeits`は本非対局ベンチマークの対象外とした。

## 結果

### NNUE学習器のスループットとスレッド並列性

教師データ`book-0.mnsd`（バッチサイズ16,384、シード1、尺度係数`K_t=733.1159421542734`、`lambda=1.0`）に対する`bench-step --profile`の計測値を次の表に示す。
表中の`compute_ms`はGPU計算合計時間、`fwd_ms`は順伝播、`bwd_ms`は逆伝播、`opt_ms`は最適化器、`transfer_ms`はホストおよびデバイス間の転送時間（ミリ秒）を示し、`t`はスレッド数`--threads`を表す。

| 構造 | 条件 | compute_ms（ミリ秒） | fwd_ms（ミリ秒） | bwd_ms（ミリ秒） | opt_ms（ミリ秒） | transfer_ms（ミリ秒） | ワーカー供給速度（局面毎秒） |
|---|---|---:|---:|---:|---:|---:|---:|
| LayerStack | `bench-step` `t=1` | 11.917 | 4.954 | 6.065 | 0.898 | 1.587 | 191,878 |
| LayerStack | `bench-step` `t=4` | 11.548 | 4.767 | 5.935 | 0.846 | 1.569 | 720,242 |
| LayerStack | `bench-step` `t=8` | 11.511 | 4.749 | 5.922 | 0.840 | 1.568 | 1,228,281 |
| LayerStack | `bench-step` `t=16` | 11.576 | 4.785 | 5.942 | 0.850 | 1.568 | 2,037,044 |
| Simple | `bench-step` `t=1` | 46.755 | 5.056 | 24.817 | 16.881 | 1.871 | 221,495 |
| Simple | `bench-step` `t=4` | 46.532 | 5.068 | 24.563 | 16.900 | 1.881 | 831,814 |
| Simple | `bench-step` `t=8` | 46.697 | 5.075 | 24.723 | 16.899 | 1.881 | 1,321,901 |
| Simple | `bench-step` `t=16` | 46.795 | 5.071 | 24.826 | 16.897 | 1.889 | 1,941,516 |

`nnue train`による検証集合117,493局面を含めた1スーパーバッチ実測では、128ステップの`LayerStack`が`t=4`で学習時間3.0543秒および686,613局面毎秒、`t=16`で学習時間2.0605秒および1,017,791局面毎秒であった。
1ステップあたりの平均更新時間`update_seconds`は`t=4`で0.01170秒、`t=16`で0.01149秒であった。
学習開始時`sb0`と1スーパーバッチ完了時`sb1`の検証損失は、`t=4`で0.7596と0.7594、`t=16`で0.7683と0.7792であった。

一方で48ステップおよび`t=16`における`Simple`の総パラメータ数は250,287,457、学習時デバイス占有は4,228,545,268バイトに達した。
同条件の学習時間は2.4794秒（317,181局面毎秒）および`update_seconds`が0.04795秒であり、`LayerStack`の31.2%の速度にとどまった。
`Simple`の検証損失は`sb0`で0.4379および`sb1`で0.4495であった。

1バッチ（16,384局面）あたりの入力転送バイト数は、6,841確保行を持つ`LayerStack`で38,492,408バイトであった。
これに対し、72個の王バケットと1バケットあたり13,392個の基底特徴からなる964,224行を持つ`Simple`の入力転送バイト数は46,121,556バイトであり、転送時間`transfer_ms`はいずれも1.57〜1.89ミリ秒に収まった。

### NNUE各カーネルのCUDAイベント実測

バッチサイズ16,384（32,768視点）における疎特徴統計と、各カーネルのCUDAイベント計測時間を以下に示す。
`LayerStack`の特徴量集合Aは有効6,840行と予約された番兵用最終1行の計6,841行を確保し、未使用列は番兵値で埋める構造を持つ。

特徴量集合Aでは延べ特徴量数が1,788,710件であり、1視点あたり平均54.6件と番兵割合62.3%を記録した。
同バッチの非空行は確保6,841行のうち3,972行であり、疎特徴の逆引きインデックスにおけるセグメント数は17,044個であった。

特徴量集合`HalfKP_vm`の`Simple`では延べ特徴量数が1,723,372件であり、1視点あたり平均52.6件であった。
同バッチの非空行は964,224行中42,973行（観測率4.46%かつ未観測率95.54%）であり、セグメント数は53,884個、出現行の出現回数中央値は2.0回かつ4回以下が75.4%を占めた。

| 構造 | カーネル名 | 所要時間（ミリ秒） | 内訳フェーズ |
|---|---|---:|---|
| LayerStack | `ls_forward` | 3.838 | 順伝播 |
| LayerStack | `ls_statistics` | 1.189 | 順伝播 |
| LayerStack | `ls_statistics_reduce` | 0.006 | 順伝播 |
| LayerStack | `ls_backward` | 0.362 | 逆伝播 |
| LayerStack | `ls_head_partials` | 1.549 | 逆伝播 |
| LayerStack | `ls_head_reduce` | 0.052 | 逆伝播 |
| LayerStack | `ls_sparse_partials` | 3.761 | 逆伝播 |
| LayerStack | `ls_sparse_reduce` | 0.168 | 逆伝播 |
| LayerStack | `ls_adamw` | 0.925 | 最適化器 |
| Simple | `simple_forward` | 4.368 | 順伝播 |
| Simple | `simple_statistics` | 0.860 | 順伝播 |
| Simple | `simple_statistics_reduce` | 0.006 | 順伝播 |
| Simple | `simple_backward` | 0.784 | 逆伝播 |
| Simple | `simple_head_partials` | 4.876 | 逆伝播 |
| Simple | `simple_head_reduce` | 0.048 | 逆伝播 |
| Simple | 勾配ゼロ初期化（`memset_zeros`） | 2.338 | 逆伝播 |
| Simple | `simple_sparse_reduce` | 12.606 | 逆伝播 |
| Simple | `simple_virtual_reduce` | 3.968 | 逆伝播 |
| Simple | `simple_adamw` | 16.881 | 最適化器 |

### PV-MCTSの推論とCPU木探索および自己対局

ONNX Runtime上の実行プロバイダEP（Execution Provider）別の性能を比較するため、288局面フィクスチャに対する`nn-eval`を実測した。
`nn-eval`は端数バッチを含めて推論を呼び出すため、合計時間を実呼び出し回数で割った平均時間と、合計時間を288割るバッチ幅の満杯相当呼び出し回数で割った全幅換算時間の双方を併記する。
TensorRTのバッチ幅256は未計測である。

| 対象 | EPおよび設定 | バッチ幅 | 実呼び出し回数 | 合計時間（秒） | 1回あたり平均（ミリ秒） | 全幅換算（ミリ秒） | スループット（局面毎秒） |
|---|---|---:|---:|---:|---:|---:|---:|
| `nn-eval`（288局面） | CUDA・FP32 | 1 | 288 | 0.761175 | 2.64 | 2.64 | 378.4 |
| `nn-eval`（288局面） | CUDA・FP32 | 16 | 18 | 0.166874 | 9.27 | 9.27 | 1,725.9 |
| `nn-eval`（288局面） | CUDA・FP32 | 128 | 3 | 0.171912 | 57.30 | 76.41 | 1,675.3 |
| `nn-eval`（288局面） | CUDA・FP32 | 256 | 2 | 0.165461 | 82.73 | 147.08 | 1,740.6 |
| `nn-eval`（288局面） | TensorRT・FP16 | 1 | 288 | 0.154970 | 0.54 | 0.54 | 1,858.4 |
| `nn-eval`（288局面） | TensorRT・FP16 | 16 | 18 | 0.040053 | 2.23 | 2.23 | 7,190.5 |
| `nn-eval`（288局面） | TensorRT・FP16 | 128 | 3 | 0.037267 | 12.42 | 16.56 | 7,728.0 |

CPU木探索のみのベンチマーク（15局面各800ノード、合計12,000シミュレーション）の実測は次のとおりである。

| 対象 | 評価器 | バッチ幅 | 15局面合計時間（ミリ秒） | スループット（シミュレーション毎秒） |
|---|---|---:|---:|---:|
| CPU MCTS `bench 800` | ダミー評価 | 16 | 257 | 46,580 |
| CPU MCTS `bench 800` | ダミー評価 | 128 | 225 | 53,154 |
| CPU MCTS `bench 800` | ダミー評価 | 256 | 230 | 52,136 |

非確定中盤5局面に対するTensorRT条件の対局用PV-MCTS実測は次のとおりである。

| 対象 | 設定 | バッチ幅 | 1手あたり平均時間（ミリ秒） | スループット（シミュレーション毎秒） |
|---|---|---:|---:|---:|
| 対局PV-MCTS（5局面） | TensorRT（nodes 800） | 1 | 428.0 | 1,869.1 |
| 対局PV-MCTS（5局面） | TensorRT（nodes 800） | 16 | 130.2 | 6,143.2 |
| 対局PV-MCTS（5局面） | TensorRT（nodes 800） | 128 | 118.2 | 6,769.8 |
| 対局PV-MCTS（5局面） | TensorRT（nodes 3200） | 1 | 1,639.2 | 1,952.2 |
| 対局PV-MCTS（5局面） | TensorRT（nodes 3200） | 16 | 507.6 | 6,304.1 |
| 対局PV-MCTS（5局面） | TensorRT（nodes 3200） | 128 | 444.6 | 7,197.1 |

追加で実施した8フィクスチャ（`20..28`切片、合計4,946シミュレーション、`go nodes 800`、バッチ幅16）の比較では、FP32のCUDA実行が652.8ミリ秒毎手で947.1シミュレーション毎秒であった。
これに対し、FP16のTensorRT実行は95.6ミリ秒毎手で6,468.8シミュレーション毎秒となり、6.83倍の速度を達成した。

組み替え初期配置`shuffled-setup`、シード`9100000000`、各スレッド128同時対局スロット、および4シミュレーションで`zero-selfplay`を実行した。
4シミュレーションの探索に根局面の初回評価1回を加えると1記録局面あたり最大5回のネットワーク評価が生じるため、以下では5倍を上限見積もりの換算係数に用いる。
1スレッド時は128局21,270局面を19.96秒（1,065.8局面毎秒）で生成し、上限見積もりで推論約5,329局面毎秒であった。
このときの`nvidia-smi`統計はGPU稼働率`utilization.gpu`が平均90.7%かつ最大97%、メモリ制御器使用率`utilization.memory`が平均15.8%、最大VRAM占有量が1,855 MiBであった。

同条件の4スレッド時は512局77,535局面を52.03秒（1,490.1局面毎秒）で生成し、同様の上限見積もりで推論約7,451局面毎秒に達した。
このときの`nvidia-smi`統計は`utilization.gpu`が平均98.0%かつ最大100%、`utilization.memory`が平均24.0%、最大VRAM占有量が2,157 MiBであった。

## 結論

`Simple`では`simple_adamw`（16.881ミリ秒）と`simple_sparse_reduce`がGPU時間の大半を占めて`LayerStack`でも`ls_sparse_partials`と`ls_forward`が主要な負荷となる一方、PV-MCTSのTensorRT FP16実行はバッチ幅16で7,190.5局面毎秒に達し、かつ4スレッド時の`zero-selfplay`でGPU稼働率が平均98.0%に達している。
