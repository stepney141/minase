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
PyTorchの出力（4節）は学習に使った機械で1回だけ作り、ほかの機械ではそのファイルを基準にして、3節のminaseの出力だけを作って照合する。

## 2. 準備

環境は`tools/train/zero/setup.sh`で導入し、`tools/train/zero/env.sh`をbashで読み込む。
`env.sh`は自分の場所を`BASH_SOURCE`から求めるので、zshで`source`すると誤ったパスを設定する。
minaseは照合する版のソースから`cargo build --release --bin minase`でビルドする。

```console
source tools/train/zero/env.sh   # bash で実行する
cargo build --release --bin minase
```

## 3. minaseの出力を書き出す

USIの独自コマンド`nn-eval out <パス>`は、`crates/minase/tests/fixtures/nn-positions.jsonl`と同じ288局面の固定局面集を、セッションの推論器で推論し、各局面の合法手のラベルとlogit、および価値をJSONで書き出す。
`nn-eval`は、`--search mcts`で起動したminaseが`isready`で推論器を準備した後に受け付け、ネット、実行プロバイダ、バッチの大きさ、およびキャッシュの場所には、オプション`NnModel`、`NnBackend`、`NnBatchSize`、`NnCacheDir`の値を使う。
CPUとTensorRTのFP16を、それぞれバッチの大きさ1、7、および256で実行する。

```bash
run() {  # 実行プロバイダ、バッチの大きさ、出力のパス
  { printf 'usi\nsetoption name RuleSet value JCSA\nsetoption name NnModel value %s\n' '<ネット.onnx>'
    printf 'setoption name NnBackend value %s\nsetoption name NnBatchSize value %s\n' "$1" "$2"
    [ "$1" = tensorrt ] && printf 'setoption name NnCacheDir value %s\n' '<キャッシュの親ディレクトリ>'
    printf 'isready\nnn-eval out %s\nquit\n' "$3"
  } | target/release/minase --protocol usi --search mcts | tee "$3.log" | grep -q '^info string end$'
}
for b in 1 7 256; do
  run cpu $b <出力>/cpu-b$b.json && run tensorrt $b <出力>/trt-b$b.json || break
done
```

`RuleSet`は照合の内容に関係しないが、`isready`が要求するので送る。
成功した`nn-eval`は最後に`info string end`を出すので、上の例はこの行の有無で成否を判定している。
出力には、推論にかかった秒数が`info string NN inference`の行で出る。
10ブロック128チャネルのネットでは、TensorRTの初回の実行でエンジンの作成に7〜10秒かかり、2回目以降はキャッシュからの読み込みが0.4秒ほどで終わる。
キャッシュは、ネット、GPU、および最大バッチの組ごとに、`<キャッシュの親ディレクトリ>`の下の別のディレクトリに作られる。
ortクレートがrc.13であった2026年11月11日までのコミットでは、CUDAとTensorRTの実行が、出力を書き終えた後の終了処理で異常終了することがあった（終了コード134、標準エラーに`corrupted double-linked list`）。
原因はrc.13がONNX Runtimeの環境を解放する順序にあり、rc.15へ上げて直した（[ortの版と環境の寿命のADR](../adr/hold-the-onnx-runtime-environment-until-process-exit-through-ort-rc15.md)）。
それより前のコミットの実行ファイルで異常終了した場合も、出力のJSONは終了処理の前に書き終わっている。

## 4. PyTorchの出力を書き出す

PyTorchの出力は、照合する`nn-eval`の出力と同じ形のJSON（`"backend": "pytorch"`）で書き出す。
学習したネットと、学習器の`init`が書き出す強化学習の初期ネットの出力は、どちらも学習器`minase_zero`の`fixture-eval`がチェックポイントから書き出す。
`fixture-eval`は、チェックポイントの重みのネットをONNXへ書き出し直し、`--onnx`のファイルとバイト単位で一致しなければ失敗する。
PyTorchの出力は、ネットが学習に使う順伝播の値であり、合法手のlogitは方策の35,856ラベルの元の番号で引いたfloat32の値である。
書き出したONNXは、入力をuint8で受け、方策を、盤面によらず合法手になり得る16,108ラベルへ詰めたFP16で返す（[入出力の受け渡しのADR](../adr/exchange-zero-inference-tensors-as-uint8-inputs-and-packed-fp16-policies-in-pinned-memory.md)）。
`nn-eval`は、詰めた後の添字から読んだlogitを、元のラベルの番号とともに書き出すので、2つの出力は同じ形で比べられる。
書き出しはBatchNormを定数畳み込みで畳み込みの重みとバイアスへ融合し、その単精度の丸めはCPUによって変わるので、このバイト単位の一致は学習した機械でしか成り立たない。
1サイクル分学習したネットを作業機で書き出し直すと、70個の初期値のうち28個が最大4.8×10⁻⁷異なった（[記録](../measurements/alphazero-rl/alphazero-rl-cycle1-workstation-check.md)）。
このため`fixture-eval`は学習した機械で実行し、作業機などほかの機械では、そこで書いた出力を照合の基準に使う。

```console
cd tools/train/zero
uv run --frozen python -m minase_zero fixture-eval --checkpoint <出力先>/checkpoint.pt \
  --onnx <出力先>/model.onnx --fixture ../../../crates/minase/tests/fixtures/nn-positions.jsonl --out <出力>/pytorch.json
```

## 5. 照合する

各出力を、PyTorchの出力を基準として`tools/train/zero/nn_compare.py`で照合する。
ONNX RuntimeのCPUの出力の照合では、PyTorchの出力のlogitをFP16へ丸めてから合法手のlogitを比べ、差の絶対値からFP16の1目盛りを引いた値を許容誤差1×10⁻⁴で判定する。価値の差も許容誤差1×10⁻⁴で比べる。
1目盛りを許すのは、ONNXが返すlogitがFP16へ丸められており、float32の計算の誤差で丸めの境界をまたぐと1目盛りずれるからである。
TensorRTのFP16の出力の照合では、各局面の合法手のlogitをsoftmaxで正規化した確率の差を許容誤差1×10⁻²で、価値の差を5×10⁻²で比べる。

```console
uv run --frozen python nn_compare.py --reference <出力>/pytorch.json --candidate <出力>/cpu-b7.json \
  --policy fp16-logits --policy-tolerance 1e-4 --value-tolerance 1e-4
uv run --frozen python nn_compare.py --reference <出力>/pytorch.json --candidate <出力>/trt-b7.json \
  --policy probabilities --policy-tolerance 1e-2 --value-tolerance 5e-2
```

照合は、ネットのSHA-256、局面数、各局面のSFEN、ラベルの列が一致し、すべての値が有限であり、`--policy`で選んだ量と価値の差の絶対値の最大がそれぞれの許容誤差以下のときに、終了コード0で終了する。
標準出力の1行のJSONは、局面数、logitの差の最大、合法手の確率の差の最大、価値の差の最大、最大のlogitを持つ手（以下、最大の手）の不一致の数、および不一致の局面ごとの基準のlogitの差を示す。
logitの差の最大は、どちらの比べ方でも、丸める前のPyTorchの出力との差である。FP16への丸めだけで、学習したネットでは約4×10⁻³になる（logitの絶対値が8以上16未満のときの1目盛りの半分）。

FP16をlogitではなく確率で比べるのは、FP16の丸めの誤差が値の大きさに比例して層ごとに累積し、logitの大きい学習したネットではlogitの差も大きくなるからである。
1サイクル分学習した10ブロック128チャネルのネットでは、合法手のlogitの最大が約14あり、FP16のlogitの差は最大4.3×10⁻²、価値の差は最大1.1×10⁻²であったが、MCTSが使う合法手の確率の差は最大2.8×10⁻³であった（[記録](../measurements/alphazero-trainer/alphazero-trainer-trained-net-output-check.md)）。
各合法手のlogitを、合法手の列で隣にある着手のものと読み違えた場合、同じネットで局面ごとの確率の差の最大は中央値3.5×10⁻²、全体で0.21になるので、確率の許容誤差1×10⁻²でも出力の並びのずれや読み違えは検出できる。
FP16の照合を確率と価値の形にしたのは2026年10月28日の開発者の決定であり、価値の許容誤差は、強化学習のサイクルが進むにつれて価値の差が丸めの累積で増え、サイクル6で0.0218になったことを受けて、2026年10月29日に2×10⁻²から5×10⁻²へ改めた（[ADR](../adr/compare-fp16-zero-network-values-within-five-hundredths.md)）。

最大の手の不一致は合否に使わない。
要素ごとの検査が通れば、最大の手は、logitがほぼ同点の局面でだけ入れ替わるからである。

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

入力の面とラベルの意味を変えずに、ONNXの入力と出力の型や形だけを変えた場合は、固定局面集を書き直さない。
この場合、変える前に書き出したONNXは推論器が起動時に拒むので、チェックポイントからONNXだけを書き出し直す。

```console
cd tools/train/zero
uv run --frozen python -m minase_zero export-onnx --checkpoint <出力先>/checkpoint.pt --out <新しいネット.onnx>
```

書き出し直したONNXはSHA-256が変わるので、PyTorchの出力も同じ機械で`fixture-eval`を実行して作り直す。

```console
printf 'nn-fixture out crates/minase/tests/fixtures/nn-positions.jsonl\nquit\n' \
  | cargo run --release --bin minase -- --protocol usi
```

minaseのテストは、生成した内容とこのファイルがバイト単位で一致することを確かめる。
