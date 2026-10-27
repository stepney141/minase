# AlphaZero型探索のネットの出力の照合の手引き

本書は、AlphaZero型探索のネットについて、PyTorchの出力、ONNX RuntimeのCPUの出力、およびTensorRTのFP16の出力が一致することを確かめる手順を定める。
照合は、ONNXへの書き出しで出力の並びがずれた場合や、minaseが別の着手のlogitを読む場合を、学習したネットを対局と自己対局に使う前に検出するために行う。
ネットの入出力の定義は`crates/minase/src/nn/`のdocコメントに、方式を選んだ理由は[ADR](../adr/README.md)（入力、方策、価値、推論、およびエンジンのキャッシュの各項）にある。
ランダム初期化のネットで手順を確かめた結果は、[ランダム初期化のネットの出力の照合](../measurements/alphazero-network/random-net-output-check.md)にある。

## 1. 照合を行う時期と機械

照合は、次のときに行う。

- 学習したネットを初めて推論に使う前（最初の自己対局の生成を始める前）。
- 最初の進捗測定の前。
- ネットの大きさを変えたとき。
- 推論に使うGPUの種別を変えたとき。

TensorRTのエンジンはGPUの機種ごとに作られるので、照合は学習に使ったGPUの機械と、対局に使う作業機の両方で行う。

## 2. 準備

環境は`tools/train/zero/setup.sh`で導入し、`tools/train/zero/env.sh`をbashで読み込む。
`env.sh`は自分の場所を`BASH_SOURCE`から求めるので、zshで`source`すると誤ったパスを設定する。
minaseは照合する版のソースから`cargo build --release --bin minase`でビルドする。

```console
source tools/train/zero/env.sh   # bash で実行する
cargo build --release --bin minase
```

## 3. minaseの出力を書き出す

`minase dev nn-eval`は、`tools/train/zero/fixtures/nn-positions.jsonl`と同じ288局面の固定局面集を、指定した実行プロバイダとバッチの大きさで推論し、各局面の合法手のラベルとlogit、および価値をJSONで書き出す。
CPUとTensorRTのFP16を、それぞれバッチの大きさ1、7、および256で実行する。

```console
for b in 1 7 256; do
  target/release/minase dev nn-eval --nn-model <ネット.onnx> --backend cpu --batch $b --out <出力>/cpu-b$b.json
  target/release/minase dev nn-eval --nn-model <ネット.onnx> --backend tensorrt --cache-dir <キャッシュの親ディレクトリ> \
    --batch $b --out <出力>/trt-b$b.json
done
```

標準エラーには、セッションの作成と推論にかかった秒数が出る。
10ブロック128チャネルのネットでは、TensorRTの初回の実行でエンジンの作成に7〜10秒かかり、2回目以降はキャッシュからの読み込みが0.4秒ほどで終わる。
キャッシュは、ネット、GPU、および最大バッチの組ごとに、`<キャッシュの親ディレクトリ>`の下の別のディレクトリに作られる。
CUDAの実行プロバイダ（`--backend cuda`）は、ONNX Runtime 1.28.3では出力を書き終えた後の終了処理で非決定的に異常終了することがあり、終了コード134でも出力のJSONは完全である。

## 4. PyTorchの出力を書き出す

PyTorchの出力は、照合する`minase dev nn-eval`の出力と同じ形のJSON（`"backend": "pytorch"`）で書き出す。
ランダム初期化のネット（`tools/train/zero/feasibility/make_nets.py`が書き出すもの）は、`tools/train/zero/nn_torch_reference.py`で書き出す。
このスクリプトは、同じシードからネットを作り直し、書き出したONNXのバイト列が`--onnx`のファイルと一致しなければ失敗する。

```console
cd tools/train/zero
uv run --frozen python nn_torch_reference.py --onnx <ネット.onnx> --blocks 10 --channels 128 \
  --fixture fixtures/nn-positions.jsonl --out <出力>/pytorch.json
```

学習したネットの出力は、学習器`minase_zero`の`fixture-eval`がチェックポイントから同じ形で書き出す。
`fixture-eval`は、チェックポイントの重みのネットをONNXへ書き出し直し、`--onnx`のファイルとバイト単位で一致しなければ失敗する。

```console
cd tools/train/zero
uv run --frozen python -m minase_zero fixture-eval --checkpoint <出力先>/checkpoint.pt \
  --onnx <出力先>/model.onnx --fixture fixtures/nn-positions.jsonl --out <出力>/pytorch.json
```

## 5. 照合する

各出力を、PyTorchの出力を基準として`tools/train/zero/nn_compare.py`で照合する。
ONNX RuntimeのCPUの出力の照合では、合法手のlogitと価値の差を、どちらも許容誤差1×10⁻⁴で比べる。
TensorRTのFP16の出力の照合では、各局面の合法手のlogitをsoftmaxで正規化した確率の差を許容誤差1×10⁻²で、価値の差を2×10⁻²で比べる。

```console
uv run --frozen python nn_compare.py --reference <出力>/pytorch.json --candidate <出力>/cpu-b7.json \
  --policy logits --policy-tolerance 1e-4 --value-tolerance 1e-4
uv run --frozen python nn_compare.py --reference <出力>/pytorch.json --candidate <出力>/trt-b7.json \
  --policy probabilities --policy-tolerance 1e-2 --value-tolerance 2e-2
```

照合は、ネットのSHA-256、局面数、各局面のSFEN、ラベルの列が一致し、すべての値が有限であり、`--policy`で選んだ量と価値の差の絶対値の最大がそれぞれの許容誤差以下のときに、終了コード0で終了する。
標準出力の1行のJSONは、局面数、logitの差の最大、合法手の確率の差の最大、価値の差の最大、最大のlogitを持つ手（以下、最大の手）の不一致の数、および不一致の局面ごとの基準のlogitの差を示す。

FP16をlogitではなく確率で比べるのは、FP16の丸めの誤差が値の大きさに比例して層ごとに累積し、logitの大きい学習したネットではlogitの差も大きくなるからである。
1サイクル分学習した10ブロック128チャネルのネットでは、合法手のlogitの最大が約14あり、FP16のlogitの差は最大4.3×10⁻²、価値の差は最大1.1×10⁻²であったが、MCTSが使う合法手の確率の差は最大2.8×10⁻³であった（[記録](../measurements/alphazero-trainer/alphazero-trainer-trained-net-output-check.md)）。
各合法手のlogitを、合法手の列で隣にある着手のものと読み違えた場合、同じネットで局面ごとの確率の差の最大は中央値3.5×10⁻²、全体で0.21になるので、確率の許容誤差1×10⁻²でも出力の並びのずれや読み違えは検出できる。
FP16の照合をこの形にしたのは2026年10月28日の開発者の決定である（[ADR](../adr/compare-fp16-zero-network-outputs-on-legal-move-probabilities.md)）。

最大の手の不一致は合否に使わない。
logitで比べる場合、各logitの差が許容誤差t以下なら、候補が最大とした手の基準のlogitは基準の最大から2t以内に収まるので、最大の手の一致は要素ごとの検査に含まれ、ほぼ同点の局面でだけ入れ替わる。

ランダム初期化のネットは、合法手のlogitの散らばりが小さく（局面ごとの標準偏差0.05〜0.07）、確率がほぼ一様になるので、FP16の確率の照合では読み違えを検出できない。
価値の散らばりも0.003しかなく、FP16の照合では価値の読み出しのずれも検出できない。
これらはCPUの1×10⁻⁴の照合で検出するので、CPUの照合は省略しない。

## 6. 照合に失敗したとき

照合に失敗したネットは、推論に使わない。
SHA-256、SFEN、またはラベルの列が一致しない場合は、ネットのファイルか固定局面集の版が食い違っているので、同じファイルとminaseのコミットで出力を作り直す。
CPUの照合が許容誤差を超えた場合は、ONNXへの書き出しと、PyTorchの側で入力を作る処理に不整合がないかを調べる。
CPUの照合が通り、TensorRTの照合だけが許容誤差を超えた場合は、FP16で値が飽和または桁落ちしている可能性があるので、層ごとの値の範囲を調べてから対処を決める。

## 7. 固定局面集を書き直すとき

固定局面集は、入力の符号化または方策のラベルを変えたときだけ書き直す。
入力の符号化か方策のラベルを変えると、学習済みのネットはすべて使えなくなる。

```console
cargo run --release --bin minase -- dev nn-fixture --out tools/train/zero/fixtures/nn-positions.jsonl
```

minaseのテストは、生成した内容とこのファイルがバイト単位で一致することを確かめる。
