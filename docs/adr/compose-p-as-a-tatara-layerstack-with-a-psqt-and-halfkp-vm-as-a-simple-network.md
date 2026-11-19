# P型をtataraのLayerStackにPSQTを足した構造で、HalfKP_vmを256x2-32-32のSimpleで組み、量子化をtataraにそろえる

## ステータス

承認済み。決定日は2026年11月19日。[NNUEを、探索用駒価値で初期化した線形の経路と、要素積と2段の隠れ層を持つ非線形の経路で組み立てる](archive/compose-the-nnue-of-a-piece-value-initialized-linear-path-and-pairwise-products-with-two-hidden-layers.md)と、[NNUEの線形の経路の学習率を0.03に保ち、第1層と同じ学習率にそろえない](archive/keep-the-nnue-linear-path-learning-rate-at-0-03.md)を置き換える。

## コンテキスト

[以前の決定](archive/compose-the-nnue-of-a-piece-value-initialized-linear-path-and-pairwise-products-with-two-hidden-layers.md)は、P型のNNUEを、序中盤と終盤の2列を駒数で補間する線形の経路と、第1層を4分割する独自の要素積と2段の隠れ層を持つ非線形の経路の和で組み、後段を16ビット（尺度256と4,096）で量子化した。
線形の経路の学習率は0.03に保っていた（[ADR](archive/keep-the-nnue-linear-path-learning-rate-at-0-03.md)）。

エンジンの開発者は2026年11月15日に、P型もtataraの枠組みへ載せ替え、重みをtataraの形式にすると決めた。
tataraの形式は、補間つきの線形の経路を表せない。
tataraのLayerStackは、第1層（幅H）の出力の成分jとj＋H/2の積、16出力の第1後段層（最後の1成分を出力へ直接足す）、2乗とそのままの2つの活性化、32出力の層、および出力からなり、駒と升ごとの点数を足す表PSQT（piece-square table）を並べられる。
Simpleは、両視点の第1層の出力を連結して隠れ層へ通す、区分もPSQTもない形である。
HalfKP_vmと後段256x2-32-32は、将棋エンジンやねうら王が使ってきた構成であり、開発者はHalfKP-hm-b8の大敗の原因を独自の区分、隠れ層がないこと、および教師の少なさと見て、実績のある構成で試し直すと決めた（[設計書の目的](../plans/archived/nnue-tatara-framework-and-halfkp-vm.md#目的)）。

## 決定

P型は、tataraのLayerStack（後段の区分1つ）に区分1つのPSQTを並べた構造とし、第1層の幅を256とする。
PSQTの寄与は手番側と相手側のPSQTの和の差の半分であり、後段の整数の出力に足してから`fv_scale`で割る。
HalfKP_vmは、tataraのSimple（活性化は0〜1へ切り詰めるCReLU、第1層の幅256を両視点で連結して32、32、1の層へ通す256x2-32-32）とする。
量子化はtataraにそろえ、第1層は16ビットで尺度127、後段の重みは8ビットで尺度64、後段のバイアスとPSQTは32ビットで尺度127×64＝8,128とする。
学習はtataraに倣い、PSQTを第1層と同じ最適化器の群に入れて第1層の学習率を使い、LayerStackの第1層の重みは半値幅√(1÷13,680)、後段の重みは±0.01の一様分布、バイアスは0、Simpleは第1層を含めて±0.01の一様分布で初期化する。
PSQTの初期値は、手書きの評価関数v0の駒割を出力の換算の尺度K_o（[勝率尺度のADR](separate-the-nnue-teacher-win-rate-scale-from-the-output-scale.md)）で割った値とし、王駒と先獅子の行は0とする。

## 検討した代案

補間つきの線形の経路を残す案は、tataraの形式に表せないので採らなかった。
v0は序盤と終盤に同じ値を置いているので、区分1つのPSQTで正確に表せる。
いまの後段の量子化（16ビット、尺度256と4,096）を残す案は、tataraの形式から外れるので採らなかった。
以前に測った尺度64での平均の量子化誤差29.80 cp（センチポーン、歩兵1枚の100分の1）（[測定](../measurements/nnue-architecture-revision/nnue-architecture-revision-training.md)）は、第1層の尺度255、独自の4分割の要素積、16ビットの出力の重みという、tataraと違う条件の値であり、tataraの量子化を退ける根拠にならない。
線形の経路の学習率0.03は、センチポーンの単位の値であり、ネットの出力の単位のPSQTへそのまま移せないので引き継がなかった。
第1層の正規分布の初期化（[教訓](../lessons/nnue-first-layer-init-saturation.md)）は、飽和を避ける同じ目的をtataraの初期化も満たすので残さなかった。

## 結果

推論の整数の式は`crates/minase/src/eval/nnue/network/layer_stack.rs`と`simple.rs`のdocコメントに、学習の実数の式、初期化、および学習中の切り詰めは`crates/minase-trainer/src/nnue/layerstack/`と`simple/`のdocコメントにある。
後段の重みと、第1後段層L1と第2後段層L2のバイアスは、tataraと同じく学習中に±127/64へ切り詰める。
ただし、L1の重みは区分ごとの組と`L1 shared`の組を書き出しで足すので、それぞれを±127/128へ切り詰め、和を量子化した値が8ビットに収まるようにする。
第1層は、エンジンが累算値の上限（バイアスの絶対値と、升ごとの重みの絶対値の最大値144個と、先獅子の最大値の和が32,767以下）を満たさないネットを拒否するので、量子化した絶対値が224以下になるよう切り詰める（和の項は146個なので、224×146＝32,704が上限に収まる）。
これはtataraにない、minaseの安全策である。
LayerStackの区分ごとのL1と、全区分に共通のtataraの`L1 shared`は、区分が1つでも別に学び、書き出しで足してから1回だけ量子化する。
書き出しで量子化の範囲を超える値はエラーで止め、tataraのように切り詰めない。
載せ替えたP型は、載せ替えの確認のSTCで`H1`となり、載せ替える前のP型が+64.4 Elo（95%信頼区間+37.2〜+92.4）強かった（[測定記録](../measurements/nnue-tatara-framework-and-halfkp-vm/nnue-tatara-framework-and-halfkp-vm-stc.md)）。
載せ替え後のP型は、同じ教師で学んだ載せ替え前のP型を相手にすると、後段の重みを通常どおり8ビットに量子化するより全層を実数で計算した方が得点率が高かった。
一方、8ビットに量子化した載せ替え後のP型を相手にすると、全層を実数で計算しても優位は見られなかった（[棋力低下の調査報告](../audits/tatara-p-regression-2026-11-19.md)）。
2026年11月19日に開発者は、このSTCの結果と調査報告を踏まえ、載せ替えたP型を採用すると決めた。
[設計書](../plans/archived/nnue-tatara-framework-and-halfkp-vm.md)は「載せ替える前のP型より10 Elo以上弱いとは言えないこと」を採用の条件にしていたが、この決定はその条件を満たさないまま下した。
