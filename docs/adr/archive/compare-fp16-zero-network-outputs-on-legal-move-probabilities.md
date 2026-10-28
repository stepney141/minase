# AlphaZero型探索のネットのTensorRTのFP16の出力は、logitではなく合法手の確率と価値で照合する

## ステータス

置き換え済み。決定日は2026年10月28日。[合法手の確率を1×10⁻²、価値を5×10⁻²で照合するADR](../compare-fp16-zero-network-values-within-five-hundredths.md)によって置き換えられた。

## コンテキスト

学習したネットを推論に使う前に、PyTorch、ONNX RuntimeのCPU、およびTensorRTのFP16の出力を固定局面集の288局面で照合する（[ネットの出力の照合の手引き](../../guides/alphazero-network-check.md)）。
照合は、ONNXへの書き出しで出力の並びがずれた場合や、minaseが別の着手のlogitを読む場合を検出するために行う。
[ネットの入出力と推論の設計書](../../plans/archived/alphazero-network.md)は、許容誤差の初期値を、合法手のlogitと価値の差についてCPUで1×10⁻⁴、FP16で1×10⁻²と定め、ランダムに初期化したネットで照合が通ることを確かめた。

1サイクル分学習した10ブロック128チャネルのネットでは、CPUの照合は通ったが、FP16のlogitの差は最大4.3×10⁻²、価値の差は最大1.1×10⁻²となり、1×10⁻²を超えた（[記録](../../measurements/alphazero-trainer/alphazero-trainer-trained-net-output-check.md)）。
層ごとの値の最大は14.17で飽和はなく、差は値の大きさに比例するFP16の丸めの累積であった。
MCTSが使うのは合法手のlogitをsoftmaxで正規化した確率であり、その差は最大2.8×10⁻³、KLダイバージェンスは最大8.4×10⁻⁵であった。
各合法手のlogitを、合法手の列で隣にある着手のものと読み違えた場合、局面ごとの確率の差の最大は中央値3.5×10⁻²、全体で0.21になる。
エンジンの開発者は、この結果を受けて照合の方式を決め直した。

## 決定

TensorRTのFP16の照合は、各局面の合法手のlogitをsoftmaxで正規化した確率の差を許容誤差1×10⁻²で、価値の差を2×10⁻²で比べる。
ONNX RuntimeのCPUの照合は、従来どおり合法手のlogitと価値の差をどちらも1×10⁻⁴で比べる。
照合の道具`tools/train/zero/nn_compare.py`は、比べる量（`--policy logits`か`--policy probabilities`）と、方策と価値の許容誤差を、既定値を持たない別々の引数で受け取る。

## 検討した代案

FP16のlogitの許容誤差を、絶対値と値の大きさに比例する項の和にする案は採らなかった。
比例の係数をネットの層の数や大きさに合わせて決める必要があり、照合の合否が、探索が実際に使う量とは別の基準で決まるからである。

FP16のlogitの許容誤差を単に大きくする案は採らなかった。
学習が進んでlogitが大きくなるたびに許容誤差を広げることになり、読み違えとの境が定まらないからである。

## 結果

学習したネットは、新しい基準でバッチの大きさ1、7、および256のFP16の照合に通った（[記録](../../measurements/alphazero-trainer/alphazero-trainer-fp16-probability-check.md)）。
ランダムに初期化したネットは確率がほぼ一様になるので、FP16の確率の照合では読み違えを検出できない。
読み違えと価値の読み出しのずれは、CPUの1×10⁻⁴の照合で検出するので、CPUの照合は省略しない。
低精度の推論の許容誤差を学習したネットの出力で決める規則は、[教訓](../../lessons/judge-low-precision-inference-on-trained-outputs.md)にまとめた。
