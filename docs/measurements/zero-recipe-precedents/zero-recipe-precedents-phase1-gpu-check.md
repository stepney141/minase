# AlphaZeroの形に改めたネットと学習器のGPUでの確認

## 目的

[学習レシピを先行実装に揃える設計書](../../plans/archived/zero-recipe-precedents.md)のフェーズ1で改めたネットと学習器について、20ブロック256チャネルの初期ネットのパラメータ数と出力の照合、GPUのbf16でコンパイルした学習の勾配、およびGPUで中断して再開した学習の一致を確かめる。

## コマンドライン

作業ディレクトリは`tools/train/zero`とし、bashで`env.sh`を読み込んだ。
`<S>`は作業用のディレクトリ、`<W>`はworktreeのルートである。
作業機では持ち時間つきの対局の測定が並行して走っていたので、すべて`nice -n 19`と`OMP_NUM_THREADS=4`で実行した。

```console
uv run --frozen python -m minase_zero init --blocks 20 --channels 256 --seed 20261116 --out-dir <S>/init --trainer-commit <コミット>
uv run --frozen python -m minase_zero fixture-eval --checkpoint <S>/init/checkpoint.pt --onnx <S>/init/model.onnx \
  --fixture <W>/crates/minase/tests/fixtures/nn-positions.jsonl --out <S>/pytorch.json
```

minaseの出力は、[出力の照合の手引き](../../guides/alphazero-network-check.md)の3節の手順で、CPUとTensorRTのバッチ1、7、256の6通りを書き出し、5節の`nn_compare.py`の許容誤差で照合した。

コンパイルの有無と精度（bf16とfp32）の4通りの勾配は、[複数GPU化の測定の道具](../alphazero-multi-gpu/compile-gradients.py)で、初期ネットの重みと、後に述べる自己対局で生成した教師データのミクロバッチ1つから求めた。

```console
uv run --frozen python <W>/docs/measurements/alphazero-multi-gpu/compile-gradients.py --checkpoint <S>/init/checkpoint.pt \
  --data <S>/selfplay-v3b/chunk-00000.mnsd --micro-batch-size 256 --seed 42 --device cuda --out <S>/gradients.json
```

教師データの形式MNSDの版3の自己対局のデータがNASになかった（1回目の学習のデータは版2）ので、同じ初期ネットとフェーズ1の時点の生成器で、組み替えた初期配置から448局（訓練局面15,814）を生成して使った。

```console
<S>/minase-phase1 data zero-selfplay --output-dir <S>/selfplay-v3b --nn-model <S>/init/model.onnx --nn-backend tensorrt \
  --generation-commit <コミット> --start-position shuffled-setup --seed 20261117 --games 448 --target-positions 14000 \
  --chunk-games 32 --simulations 32 --playthrough-percent 100 --threads 2 --devices 1 --games-per-thread 112
```

中断と再開の確認では、上で生成した14ファイル（32局ずつの塊）を最新のサイクルとする学習を3つのプロセスで行い、各プロセスに空の`TORCHINDUCTOR_CACHE_DIR`を与えた。
1つ目のプロセスでは中断なしで最後まで学び、2つ目のプロセスでは5回目の更新の後の保存を標準エラーの`steps=5/14`の行で検出してプロセスの木をSIGKILLで止め、3つ目のプロセスで`--resume`から再開した。

```console
uv run --frozen torchrun --standalone --nproc-per-node 1 -m minase_zero train --init <S>/init/checkpoint.pt \
  --data-list <S>/data-list.json --latest-data <14ファイル> --out-dir <出力先> --cycle 1 --seed 7 \
  --batch-size 1024 --micro-batch-size 256 --lr 0.04 --momentum 0.9 --weight-decay 0.0001 --clip-grad-norm 10 \
  --value-lambda 0.333 --precision bf16 --checkpoint-steps 5 --device cuda --replay-threads 4 --trainer-commit <コミット>
uv run --frozen torchrun --standalone --nproc-per-node 1 -m minase_zero train --resume <出力先> \
  --device cuda --replay-threads 4 --lr 0.04
```

## エンジン

学習器と生成器には、ブランチ`zero-recipe-precedents`のフェーズ1のコミット`5c60285b`と同じ内容の作業ツリーのものを使った。
初期ネットのSHA-256は`4b430a4414d250f96094c475dc489efbc865afa20e1c9bfc5b2a958cae4abf07`であり、この初期ネットは確認のためだけに作ったもので、2回目の学習には使わない。
出力の照合の固定局面集は288局面、規則セットは`JCSA`である。

## 環境

作業機のCPUはIntel Core Ultra 7 265KF（20コア20スレッド）、GPUはNVIDIA GeForce RTX 3060 Ti（8 GB、ドライバ615.78.08）である。
PyTorchは2.11.0、ONNX Runtimeは1.31.1である。

## 結果

初期ネットのパラメータ数は25,182,204であり、設計書の値と一致した。

出力の照合は6通りとも許容誤差の内側であった（[照合の出力](zero-recipe-precedents-phase1-gpu-check/)）。

| 実行の方式 | バッチ | logitの差の最大 | 合法手の確率の差の最大 | 価値の差の最大 |
|---|---|---|---|---|
| CPU | 1、7、256 | 3.04×10⁻⁵ | 7.94×10⁻⁷ | 8.87×10⁻⁸ |
| TensorRT | 1 | 4.82×10⁻⁴ | 1.28×10⁻⁵ | 2.49×10⁻⁴ |
| TensorRT | 7 | 5.84×10⁻⁴ | 1.12×10⁻⁵ | 2.52×10⁻⁴ |
| TensorRT | 256 | 5.10×10⁻⁴ | 1.11×10⁻⁵ | 2.63×10⁻⁴ |

コンパイルの勾配は、4通りとも有限であった（[結果](zero-recipe-precedents-phase1-gpu-check/compile-gradients.json)）。
コンパイルの有無による相対誤差はbf16で0.278、fp32で0.078であり、コンパイルしないbf16とfp32の間の相対誤差は0.339であった。
複数GPU化の作業機の測定（[記録](../alphazero-multi-gpu/alphazero-multi-gpu-workstation.md)）では、大域プーリングを持つ旧い構成のネットを189回の更新で学んだ重みで、同じ3つの相対誤差がそれぞれ約0.21、約0.055、および約0.23であった。

中断と再開の確認では、14回の更新のうち5回目の後に止めた学習を再開すると、`model.onnx`のバイト列が中断なしの学習と一致した。
集計の`training_losses`、`diagnostics`、`learning_curve`、`data_order_sha256`、`optimizer_steps`、および`last_lr`も一致した。
保存した区間の末尾の学習率は、累計の更新数s＝4、9、13で0.0008、0.0016、0.00224であり、0.04×(s＋1)/250と一致した。
中断なしの学習の速さは毎秒464局面（コンパイルを含む全体）であった。

## 結論

フェーズ1のネットと学習器は、出力の照合、GPUのbf16でコンパイルした勾配の条件（コンパイルの誤差がbf16とfp32の差より小さいこと）、およびGPUでの中断と再開の一致を満たした。
