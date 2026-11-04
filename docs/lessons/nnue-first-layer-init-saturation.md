# NNUE第1層の既定初期化がclipped ReLUを飽和させる

## 症状

世代0と世代1を合わせた1,106万局面でP型NNUEを20エポック学習しても、検証損失は0.509899から0.509853へ0.00005しか下がらず、訓練損失もほとんど動かず、過学習も起きなかった。
第1層の重みは初期値から平均絶対差1.13と大きく動いていたのに、ネットは局面をほぼ二値の乱数符号へ写す関数のままだった。

## 原因

学習器の`NnueModel`は第1層（`nn.EmbeddingBag`）をPyTorchの既定初期値N(0, 1)のままにしていた。
1局面で約55個の特徴が発火するため第1層の入力和の標準偏差は7.7になり、活性化を[0, 1]に切り詰めるclipped ReLUでは初期状態で94.5%、学習後も94.6%のユニットが切り詰めの外側にあって、勾配が第1層へ届かなかった。
参照実装用のランダムネット（`nnue_net.py random`）は標準偏差0.02で作っており、学習器の初期化だけが不釣り合いだった。
標準偏差0.05（入力和の標準偏差が約0.4〜0.5）に改めると、同じデータと条件で6エポックで最良に達し、以後は過学習へ転じた。

## 以後の規則

疎な特徴を加算する第1層の初期化は、入力和が活性化の切り詰めに張り付かない大きさで行い、初期局面で飽和するユニットが少ないことを確かめる。
上限の目安は発火する特徴数の平方根の逆数の標準偏差（約55個なら0.05程度）である。
これより小さい分には停滞しない。
入力次元に応じたfan-in則（標準偏差$`\sqrt{1/(3D)}`$）はtataraとnnue-pytorchの先例があり、minaseの幅256の要素積のネットでは標準偏差0.05より検証損失が下がり、固定ノードの対局でも改善した。
学習が停滞したときは、データを増やす前に活性化の飽和率（clipped ReLUなら[0, 1]の外側にあるユニットの割合）を測る。

## 出典

- [plans/evaluation-gen1.md](../plans/archived/evaluation-gen1.md) の「学習の構成」節と「教師不足の判定の検証結果と次期候補」節。
- 修正コミット`eca5276`（分岐`nnue-gen1`、rebase後は`82aa0e6`）。
- [measurements/evaluation-gen1/nnue-gen1-nodes100k.md](../measurements/evaluation-gen1/nnue-gen1-nodes100k.md) と [measurements/evaluation-gen1/nnue-gen1-tc.md](../measurements/evaluation-gen1/nnue-gen1-tc.md)。
- fan-in則の結果は[第1層の初期値を小さくしたネットの学習](../measurements/nnue-architecture-revision/nnue-architecture-revision-init-training.md)と[固定ノードの記録](../measurements/nnue-architecture-revision/nnue-architecture-revision-init-fixed-nodes.md)。
