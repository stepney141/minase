# 学習したネットの出力の照合

## 目的

[学習器の設計書](../../plans/archived/alphazero-trainer.md)のフェーズ2として、作業機で1サイクル分を学習したネットについて、PyTorchの出力を基準に、ONNX RuntimeのCPUとTensorRTのFP16の出力がバッチの大きさ1、7、および256で[ネットの出力の照合の手引き](../../guides/alphazero-network-check.md)の許容誤差に収まるかを確かめる。

## コマンドライン

```console
bash docs/measurements/alphazero-trainer/alphazero-trainer-trained-net-output-check/run.sh
```

[run.sh](alphazero-trainer-trained-net-output-check/run.sh)は、学習器の`fixture-eval`でPyTorchの出力を書き、`minase dev nn-eval`で固定局面集の288局面をCPUとTensorRTのそれぞれでバッチ1、7、256の大きさで推論し、`nn_compare.py`で照合する。
許容誤差は手引きのとおり、CPUが1×10⁻⁴、TensorRTのFP16が1×10⁻²である。
最後に[analysis.py](alphazero-trainer-trained-net-output-check/analysis.py)が、合法手のsoftmaxの確率の差、層ごとの値の大きさ、および出力の読み違え（各着手のlogitを別の着手から読む誤り）を想定した差を求める。
標準出力と標準エラーは[run.log](alphazero-trainer-trained-net-output-check/run.log)にある。

## エンジン

ネットは、[GPUでの学習の記録](alphazero-trainer-gpu-resume-check.md)の止めない学習（A）が書いた10ブロック128チャネルの`model.onnx`（SHA-256 `45204684…a6bb`）と、同じ出力先の`checkpoint.pt`である。
学習器はブランチ`alphazero-trainer`のコミットdb2119a8、`minase`は`crates/`がmasterのae6a3723と同一のworktreeからリリースビルドしたものである。
固定局面集は`tools/train/zero/fixtures/nn-positions.jsonl`、規則セットは`engine-default`である。

## 環境

作業機（Intel Core Ultra 7 265KF、メモリ31 GB、NVIDIA GeForce RTX 3060 Ti 8 GB、ドライバ615.78.08）で実行した。
ONNX Runtimeは1.28.3（CUDA 13版の配布物）、TensorRTは10.14.1.48、PyTorchは2.11.0（推論はCPU）である。
作業機では別のプロジェクトの計算とlishogiのBotが動いていた。

## 結果

PyTorchの出力との照合の結果は次のとおりである。
差は、288局面の合法手のlogitと価値についての差の絶対値の最大である。
最大のlogitを持つ手を、以下では最大の手と呼ぶ。
最大の手の不一致は、PyTorchの最大の手と候補の最大の手が異なる局面の数である。

| 候補 | 許容誤差 | logitの差の最大 | 価値の差の最大 | 最大の手の不一致 | 判定 |
|---|---|---|---|---|---|
| CPU、バッチ1 | 1×10⁻⁴ | 1.3×10⁻⁵ | 4.2×10⁻⁶ | 0 | 合格 |
| CPU、バッチ7 | 1×10⁻⁴ | 1.3×10⁻⁵ | 4.5×10⁻⁶ | 0 | 合格 |
| CPU、バッチ256 | 1×10⁻⁴ | 1.3×10⁻⁵ | 4.5×10⁻⁶ | 0 | 合格 |
| TensorRT FP16、バッチ1 | 1×10⁻² | 3.9×10⁻² | 9.7×10⁻³ | 7 | 不合格 |
| TensorRT FP16、バッチ7 | 1×10⁻² | 3.4×10⁻² | 7.7×10⁻³ | 4 | 不合格 |
| TensorRT FP16、バッチ256 | 1×10⁻² | 4.3×10⁻² | 1.1×10⁻² | 4 | 不合格 |

最大の手が入れ替わった局面では、PyTorchの最大のlogitと、候補が最大とした手のPyTorchのlogitの差は1.3×10⁻²以下であった。
TensorRTのエンジンの作成は、最初の実行でバッチの大きさごとに9.9〜13.2秒かかり、キャッシュからの読み込みは0.6秒前後であった。

TensorRTのFP16の差の性質を、次の3点で調べた。
第1に、学習したネットの合法手のlogitの絶対値の最大は、局面ごとの中央値で12.7、全体で14.2であった。
この大きさでのFP16の隣り合う値の間隔は約0.0078なので、観測した0.034〜0.043の差は、その4〜6倍に当たる。
ランダムに初期化したネットでは、各局面の合法手のlogitの標準偏差が0.046〜0.071しかなく（[ランダム初期化のネットの照合](../alphazero-network/random-net-output-check.md)）、FP16の差も4.8×10⁻⁴にとどまっていた。
第2に、PyTorchで固定局面集を推論したときの層ごとの出力の絶対値の最大は、方策の出力層の14.17が最も大きく、FP16の上限65,504に遠く、飽和はなかった。
第3に、MCTSが使うのは合法手のlogitをsoftmaxで正規化した確率なので、確率の差でも比べた。
TensorRTのFP16の確率の差の最大は1.6×10⁻³〜2.8×10⁻³、KLダイバージェンスの最大は5.9×10⁻⁵〜8.4×10⁻⁵であった。
これに対し、各合法手のlogitを合法手の列の隣の着手から読み違えたとすると、局面ごとの確率の差の最大は中央値で3.5×10⁻²、全体で0.21になる。

価値は288局面で0.096〜0.930に分布し、標準偏差は0.22であった。
ランダムに初期化したネットでは価値の散らばりが0.003しかなかったので、FP16の照合で価値の読み出しのずれを検出できなかったが、学習したネットではFP16の価値の差の最大（1.1×10⁻²）は標準偏差の約5%である。

## 結論

ONNX RuntimeのCPUの出力はPyTorchと許容誤差1×10⁻⁴に収まり、学習器が書き出したONNXの重みと出力の並びはPyTorchと一致した。
TensorRTのFP16の出力は、飽和は起きていないが、値の大きさに比例する丸めの累積によって許容誤差1×10⁻²を超えた。
手引きの6節は照合に失敗したネットを推論に使わないと定めているので、FP16の許容誤差か照合する値（logitか確率か）を決め直すまで、学習したネットを自己対局に使えない。
