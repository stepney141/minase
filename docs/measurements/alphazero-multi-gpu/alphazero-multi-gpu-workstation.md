# 作業機での複数GPU化の生成器と学習器の確認

## 目的

[複数GPU化の設計書](../../plans/archived/alphazero-multi-gpu.md)のフェーズ1とフェーズ4のうち、作業機（GPU 1枚）で行う確認の記録である。
確かめたのは次の5点である。

1. 生成器に`--devices 1`を与えた生成が、改める前の生成器と同じ塊を書くこと。
2. 全体のバッチ1,024をミクロバッチ256の4回の勾配累積で学ぶ学習が、最後まで動くこと。
3. `torch.compile`の採否の3つ目の条件として、コンパイルの有無による勾配の相対誤差が、bf16とfp32の間の相対誤差より小さいこと。
4. 採否の2つ目の条件として、同じNのまま強制終了して再開した学習が、空のコンパイルのキャッシュで、途中で止めない学習と出力のバイト列まで一致すること。
5. ONNX Runtimeを1.31.1へ上げた環境で、実行器の照合と同じ6つの比較が許容誤差に収まり、CUDAの実行プロバイダが終了処理で異常終了しないこと。

3と4は、初めの実装では満たさなかった。
原因を調べてInductorの設定を2つ加え、加えた後の学習器で測り直した（「結果」の節）。

## コマンドライン

測定日は2026年11月10日である。
`<wt>`はブランチ`alphazero-multi-gpu`の作業ツリー、`<S>`は作業機の一時ディレクトリである。
`<S>`には、2回目の学習の初期ネット（NASの`cycles-rl2/cycle-0000/`の`model.onnx`と`checkpoint.pt`。`checkpoint.pt`は`init-checkpoint.pt`として置いた）と、1回目の学習のサイクル32の生成ファイルのうち塊0の2つ（`chunk-00000.mnsd`と`chunk-00000.resigned.mnsd`）を写した。

生成器の比較は、[selfplay.sh](alphazero-multi-gpu-workstation/selfplay.sh)で、組み替えた初期配置から8局を、1スレッド、同時4局、探索32回で生成した。
同じ基本シードと同じ`--generation-commit`の文字列を、改める前の生成器と改めた生成器の両方に与えた。

```console
S=<S> bash selfplay.sh <masterのminase> <S>/master-a 1
S=<S> bash selfplay.sh <masterのminase> <S>/master-b 1
S=<S> bash selfplay.sh <ブランチのminase> <S>/branch-1 1 --devices 1
S=<S> bash selfplay.sh <ブランチのminase> <S>/branch-2 2 --devices 1
```

学習の確認では、塊0の2ファイルをリプレイバッファと最新のサイクルの両方に与え、全体のバッチ1,024、ミクロバッチ256、学習率0.04、bf16で、初期ネットから1サイクルを学んだ。
抽出数は194,288、更新は189回である。
コンパイルを入れる前の学習器は[train.sh](alphazero-multi-gpu-workstation/train.sh)で動かした。

```console
S=<S> bash train.sh <wt>/tools/train/zero <S>/w1 <コミット> 50
```

勾配の比較は[compile-gradients.py](compile-gradients.py)で、上の学習で189回学んだ重み（`<S>/w1/checkpoint.pt`）を使い、抽出のシードを3通りに変えて、ミクロバッチ1つの勾配を求めた。

```console
cd <wt>/tools/train/zero
uv run --frozen python <wt>/docs/measurements/alphazero-multi-gpu/compile-gradients.py \
  --checkpoint <S>/w1/checkpoint.pt --data <S>/data/chunk-00000.mnsd <S>/data/chunk-00000.resigned.mnsd \
  --micro-batch-size 256 --seed <20261101、20261102、20261103> --device cuda --out <結果>
```

再開の確認は[resume-check.py](resume-check.py)で、100回目の保存の後に強制終了した。
3つの学習のプロセスには、それぞれ空の`TORCHINDUCTOR_CACHE_DIR`と`TRITON_CACHE_DIR`を与えた。

```console
cd <wt>/tools/train/zero
uv run --frozen python <wt>/docs/measurements/alphazero-multi-gpu/resume-check.py --out <S>/resume-final \
  --commit <コミット> --init <S>/init-checkpoint.pt --data-list <塊0の2ファイルのJSON配列> \
  --nproc 1 --replay-threads 8 --checkpoint-steps 50 --kill-after 100
```

vast.aiの測定に使う[load-check.py](load-check.py)、[compare-outputs.py](compare-outputs.py)、および[compare-validation.py](compare-validation.py)も、同じデータで1回ずつ動かした。

ONNX Runtime 1.31.1の確認では、`env.sh`を1.31.1にしてから`tools/train/zero/setup.sh`で導入した。
vast.aiの機械Bで学んだサイクル1のネットについて、実行器の照合と同じ`minase dev nn-eval`をCPUとTensorRT（新しいキャッシュ）のバッチ1、7、256で実行し、機械Bの照合が書いた`pytorch.json`と`nn_compare.py`で比べた。
作業機の`fixture-eval`は、ONNXに書き出し直したバイト列が機械Bの`model.onnx`と一致しないとして止まった。
書き出しの定数畳み込みの丸めがCPUによって変わるためで、[教訓](../../lessons/compare-reexported-onnx-bytes-only-on-the-exporting-machine.md)のとおり、参照には機械Bの照合が書いた出力を使った。
続けて、CUDAの実行プロバイダのバッチ7を4回実行した。

```console
cd <wt>/tools/train/zero && source env.sh
<wt>/target/release/minase dev nn-eval --nn-model <ネット> --backend <cpu|tensorrt|cuda> --batch <1、7、256> \
  --out <結果>.json [--cache-dir <新しいキャッシュ>]
uv run --frozen python nn_compare.py --reference <機械Bのpytorch.json> --candidate <結果>.json \
  --policy <logits|probabilities> --policy-tolerance <1e-4|1e-2> --value-tolerance <1e-4|5e-2>
```

## エンジン

生成器は、改める前がmasterのコミット`2cb25ec4`、改めた後がコミット`22818b7a`（`--devices`の追加）である。
学習器のコミットは次のとおりである。

| コミット | 内容 |
|---|---|
| `e8af4b65` | Lightning FabricのDDPと勾配累積。コンパイルなし |
| `d9e50c11` | 学習の順伝播の`torch.compile`（設計書のとおり） |
| `336b89d2` | Inductorの`emulate_precision_casts`をプロセス全体で有効にした |
| `8130d270` | Inductorの`deterministic`を有効にし、記録用の方策の損失と診断の合法手の確率の和を、足す順序が決まった計算にした |

初期ネットの`model.onnx`のSHA-256は`8be01e2e…`である（[2回目の初期ネットの照合](../alphazero-rl-2-prep/alphazero-rl-2-prep-random-net-output-check.md)）。

## 環境

作業機（Intel Core Ultra 7 265KF、20コア20スレッド、NVIDIA GeForce RTX 3060 Ti 8 GB、ドライバ615.78.08）で、PyTorch 2.11.0（CUDA 13.0）、lightning-fabric 2.6.6、ONNX Runtime 1.28.3、およびTensorRT 10.14.1を使った。
学習器は`torchrun --standalone --nproc-per-node 1`で起動し、再生のスレッド数は8とした。

## 結果

### 生成器

改める前の生成器の2回の生成と、改めた生成器の`--devices 1`の生成は、2つの塊の3種類のファイル（`.mnsd`、`.resigned.mnsd`、`.json`）と`config.json`が、すべてバイト単位で一致した。
8局で269局面である。
改めた生成器は、2スレッドの`--devices 1`でも最後まで動いた。

GPUが1枚の作業機で、`--devices 2`はnvidia-smiの枚数との不一致として、`CUDA_VISIBLE_DEVICES`を`0`と空の文字列に設定した起動はどちらも設定の拒否として、`--threads 3 --devices 2`は倍数の検査で、出力先を作る前にエラーで止まった。

### 4回の勾配累積の学習

コンパイルを入れる前の学習器（`e8af4b65`）で、189回の更新が最後まで動いた。
学んだ局面は193,536で、端数の752局面は使わなかった。
学習の時間は214.6秒（毎秒902局面）、GPUのメモリの使用量の最大は3,527 MiB、読み込みの最大常駐メモリは1.71 GBであった。

### コンパイルの勾配（3つ目の条件）

設計書のとおりにコンパイルした学習器（`d9e50c11`）では、コンパイルしたbf16の勾配が、28,052,008個のパラメータのうち25,885,184個でNaNになった。
順伝播の出力と損失は有限で、コンパイルしないbf16、コンパイルしたfp32、およびInductorを通さない`aot_eager`のbf16の勾配は、すべて有限であった。
BatchNormを推論の扱いにしてもNaNになり、`backward_pass_autocast`を`off`にしても`same_as_forward`にしても、NaNの数は同じであった。
NaNは最後の残差ブロックの中で始まり、そのブロックの`pool_bias`の重みの勾配は有限であった。
大域プーリングの`amax`の逆伝播は最大値と等しい位置へ勾配を分けるので、Inductorが融合した演算の間のbf16の丸めを省き、計算し直した値が保存したbf16の最大値と一致しなくなったと考えられる（推論）。
Inductorの`emulate_precision_casts`（融合した演算の間でもeagerと同じ丸めを残す設定）を有効にすると、勾配はすべて有限になった。

最終の学習器（`8130d270`）での相対誤差は次のとおりである。
相対誤差は、全パラメータのクリップする前の勾配を並べたベクトルの差のノルムを、比べる基準の勾配のノルムで割った値である。

| 抽出のシード | コンパイルの有無（bf16） | bf16とfp32（コンパイルなし） | コンパイルの有無（fp32） | 条件 |
|---|---|---|---|---|
| 20261101 | 0.2102 | 0.2333 | 0.0549 | 満たす |
| 20261102 | 0.2136 | 0.2333 | 0.0567 | 満たす |
| 20261103 | 0.2132 | 0.2371 | 0.0558 | 満たす |

3通りとも、コンパイルの有無による相対誤差は、bf16とfp32の間の相対誤差より小さかった。
fp32どうしでもコンパイルの有無で約0.055の差が出たのは、GPUのfp32の畳み込みが既定でTF32（仮数10ビットの形式）で計算されるからと考えられる。
同じ重みとミクロバッチで、コンパイルしないfp32の勾配を、TF32を使う既定の計算とTF32を切った計算で比べると、相対誤差は0.065であった。
TF32を切った計算を2回繰り返したときの相対誤差は2.2×10⁻⁸であった。

### 再開の一致（2つ目の条件）

| 学習器 | 一致しなかった項目 |
|---|---|
| `336b89d2`（コンパイル、`deterministic`なし） | 重み、最適化器の状態、損失の累計、学習曲線、診断、`model.onnx` |
| `e8af4b65`（コンパイルなし） | 学習曲線の1つの値（150回目の区間の方策の損失、6.139072895と6.139072418） |
| `8130d270`（最終） | なし |

読んだ順序のハッシュ、更新回数、および累計標本数は、どの学習器でも一致した。
コンパイルした学習器の食い違いは、縮約のカーネルの構成を実行時の計測で選ぶInductorの既定によるものと考えられ、Inductorの`deterministic`（数値に影響する計測を省く設定）を有効にすると、すべての項目が一致した。
コンパイルしない学習器の1桁の食い違いは、記録にだけ使う方策の損失を局面ごとに足すGPUの`scatter_add_`の加算の順序によると考えられる（推論）。
コンパイルしない学習器の途中で止めない学習を2回行うと、学習曲線まで一致した。

最終の学習器の途中で止めない学習の`model.onnx`は、SHA-256が`791511ac…`で、再開した学習と一致した。
学習の時間は193.6秒（毎秒999局面）で、コンパイルを入れる前の902局面の約1.11倍であった。
`emulate_precision_casts`だけを有効にした`336b89d2`も毎秒990局面、`deterministic`を加えた学習器を環境変数で試したときも毎秒991局面で、2つの設定による遅れは見られなかった。

### 測定の道具

`load-check.py`は、`CUDA_VISIBLE_DEVICES=0`を付けて起動した学習器を最初の保存の後に止め、読み込みの時間2.1秒と最大常駐メモリ1.72 GBを読み出した。止めた後に学習器のプロセスは残らなかった。
`compare-outputs.py`で、コンパイルを入れる前の学習器と最終の学習器がそれぞれ189回学んだネットの固定局面集の出力を比べると、合法手の確率の差の最大は0.297、価値の差の最大は0.048であった（[結果](alphazero-multi-gpu-workstation/compile-vs-nocompile-outputs.json)）。
同じ初期値と同じデータでも、計算の丸めの違いが学習の間に育ち、FP16の照合の許容誤差（確率1×10⁻²）を大きく超えた。
`compare-validation.py`は、同じ2つのネットの検証の18局1,878局面で動き、方策の損失の差の95%区間は−0.0084〜0.0183であった。

### ONNX Runtime 1.31.1

| 比較 | 確率の差の最大 | 価値の差の最大 | 最大の手が違う局面 |
|---|---|---|---|
| CPU、バッチ1、7、256 | 6.6×10⁻⁷ | 3.7×10⁻⁶ | 0 |
| TensorRT、バッチ1 | 0.0040 | 0.0095 | 4 |
| TensorRT、バッチ7 | 0.0036 | 0.0094 | 3 |
| TensorRT、バッチ256 | 0.0032 | 0.0086 | 5 |

6つの比較は、どれも照合の許容誤差（CPUは1×10⁻⁴、TensorRTは確率1×10⁻²、価値5×10⁻²）に収まった（[結果](alphazero-multi-gpu-workstation/ort-1.31.1-check.json)）。
TensorRTのバッチ1の出力は、1.28.3で作ったエンジンの出力と288局面のすべてで一致した。
CUDAの実行プロバイダは4回とも正常に終わった。1.28.3では、同じ作業機で4回中2回が終了処理で異常終了していた（[ネットの入出力の照合の記録](../alphazero-network/random-net-output-check.md)）。

結果のJSONは[alphazero-multi-gpu-workstation/](alphazero-multi-gpu-workstation/)にある。

## 結論

作業機では、`--devices 1`の生成が改める前と同じ塊を書き、4回の勾配累積の学習が最後まで動いた。
`torch.compile`は、Inductorの`emulate_precision_casts`と`deterministic`を加えた最終の学習器で、採否の2つ目と3つ目の条件を満たし、作業機の学習を約1.11倍速くした。
1つ目の条件（初回のコンパイルを含めた1サイクルの時間）は、vast.aiのGPU 4枚の機械で判定する。
ONNX Runtime 1.31.1でも、照合の6つの比較は許容誤差に収まり、CUDAの実行プロバイダは終了処理で異常終了しなかった。
