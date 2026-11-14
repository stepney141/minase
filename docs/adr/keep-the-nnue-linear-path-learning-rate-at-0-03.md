# NNUEの線形の経路の学習率を0.03に保ち、第1層と同じ学習率にそろえない

## ステータス

承認済み。決定日は2026年11月7日。

## コンテキスト

NNUEの線形の経路は、駒状態、陣営、および升ごとの点数（cp）を序中盤用と終盤用の2列で持つ表であり、他のエンジンのPSQT（piece-square table）にあたる。
その学習率0.03は、線形の経路を学習済みのPSTで初期化して微調整していた頃のPSTの学習器の設定を引き継いだ値である（[AdamWのADR](use-pytorch-adamw-with-bias-correction.md)）。
駒価値から初期化する今の手順では、中断した世代反復の第2世代の学習で、線形の経路の6,840行が初期値から平均約0.5 cp、最大約10 cpしか動かなかった。

先行の学習器は、線形の表を第1層と同じ学習率で学ぶ。
tataraのPSQTは第1層の重みと同じパラメータ群に入り、Fairy-Stockfishの学習器variant-nnue-pytorchのPSQTは入力層と同じ学習率で学ぶ（[学習レシピの調査](../research/nnue-training-recipes.md)）。
minaseのロジットは`(線形の経路の値)/(勝率尺度) + (非線形部分の出力)`なので、ロジットの単位で第1層の学習率1×10⁻³にそろえると、cp単位では勝率尺度679.03 cpを掛けた0.67903になる。

[標準レシピv1の設計書](../plans/nnue-standard-recipe.md)は、中断した系列の第2世代の教師で、学習率0.67903で学んだネットを候補、0.03で学んだネットを対照としてSTCで比べた（[測定記録](../measurements/nnue-standard-recipe/nnue-standard-recipe-lr-linear-stc.md)）。
結果は1,607ペアで`H0`（得点率49.4%）だった。

## 決定

NNUEの線形の経路の学習率は0.03のままとし、第1層と出力部の学習率1×10⁻³とは別の値を使う。

## 検討した代案

先行例に合わせて0.67903にする案は、上の測定で強くならなかったので採らない。
換算は学習率の設定値をそろえるだけであり、AdamWの実際の移動量は勾配の履歴で変わるので、学習の速さが同じになるとまでは言えない。
0.03と0.67903の間で値を探す案は、根拠のない値を増やすので採らない。

## 結果

学習器の`--lr-linear`の既定値は0.03のままである。
測定は二値交差エントロピーの損失と早期終了を使っていた当時の学習の形で行ったので、[64 superbatchの学習](train-the-nnue-for-64-superbatches-and-export-the-last-network.md)と[勝率の差の2乗の損失](train-the-nnue-on-search-value-teachers-with-the-squared-win-rate-difference.md)でも結論が成り立つことは確かめていない仮定である。
