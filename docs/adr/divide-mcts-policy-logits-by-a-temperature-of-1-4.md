# 対局用MCTSは、ネットの方策のlogitを温度1.4で割ってからPUCTの事前確率を作る

## ステータス

承認済み。決定日は2026年11月9日。

## コンテキスト

対局用MCTSのPUCTの係数は、dlshogiの既定値を取り入れたものである（[MCTSの選択式のADR](search-mcts-games-by-puct-with-virtual-loss-batches.md)）。
dlshogiはそのうち5つの係数を、方策の温度（policy softmax temperature）1.74と同時に調整しているが、minaseはlogitを温度で割る処理を持たず、温度1.0と同じ確率を使っていた。
ほかのエンジンの既定の温度は、lc0が1.359、CrazyAraが1.7である。
dlshogiでは、学習法を変えて方策のエントロピーが上がった後、温度1.74のままでは旧モデルより約28 Elo弱くなり、最適値は1.40〜1.50へ下がった。
出典は[大敗の原因の調査](../proposals/alphazero-strength-gap/search.md)の「PUCTの周辺」の節にある。
サイクル32の自己対局では1局面の合法手が平均84手あり、温度1.0の方策は下位の手の確率が小さいので、探索が下位の手に届きにくいとみられていた。
1より大きい温度でlogitを割ると確率の分布が平らになり、下位の手にも探索が回りやすくなる。

## 決定

対局用MCTSのエンジンは、推論の結果を探索木へ渡す前に、各合法手のlogitを方策の温度Tで割る。
TはUSIオプション`PolicyTemperature`で温度の100倍の整数として受け付け（dlshogiの`Softmax_Temperature`と同じ表し方）、既定値を140（温度1.4）、範囲を10〜1000とする。
値を変えたときは、保持している探索木を捨てる。

## 検討した代案

dlshogiの1.74をそのまま取り入れる案は採らなかった。
minaseの方策は、Gumbel AlphaZeroの方式で探索が補正した確率を学習の目標としており、訪問回数の分布を目標とするdlshogiとは分布の形が異なり得るからである。
固定ノードの対局による選別では、温度1.74のEloの点推定は温度1.4をわずかに下回った。

SPSAで温度とPUCTの係数を同時に調整する案は採らなかった。
GPUで推論するMCTSの対局は同時に数局しか指せず、MCTS 800回同士で1ペア約42秒かかるので、SPSAを例えば5,000ペアの規模で回すと約58時間かかる。

探索木や駆動部に温度を持たせる案は採らなかった。
自己対局のGumbelの方式は辺の確率を使って未訪問の手の価値を補うので、探索木が温度を持つと、自己対局の確率を変えないための引数が別に要る。
対局用エンジンの側で推論の結果のlogitを割れば、自己対局はその処理を通らないので影響を受けない。
正の数で割ってもlogitの大小は変わらないので、指す手を選ぶときに未訪問の辺をlogitで比べる結果も変わらない。

PUCTの6つの係数もUSIオプションとして追加する案は採らなかった。
係数を動かす測定を計画していないからである。

## 結果

温度1.0、1.4、および1.74の3条件をMCTS 800回対αβ 500ノードで各100ペア比べると、Eloの点推定は+3.5、+56.1、および+50.7であった（[温度1.0](../measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-t100-ladder.md)、[1.4](../measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-t140-ladder.md)、[1.74](../measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-t174-ladder.md)）。
100ペアの選別なので、1.4と1.74の優劣は決まっていない。
既定値を1.4にしたMCTSは、既定値1.0のMCTSとのSTCで111ペアで`H1`となり、得点率は71.1%であった（[STC](../measurements/mcts-royal-capture-temperature/mcts-royal-capture-temperature-temperature-stc.md)）。
LTCは、エンジンの開発者の決定により行わず、STCの`H1`をもって採用とした。
これは、[SPRTの手引き](../guides/sprt.md)の「採用の決定はLTCの判定だけが下す」という規則の、開発者の判断による例外である。

温度の最適値はネットと学習法によって変わるので、ネットの大きさを変える段階に進んだときや学習法を変えたときは、同じ固定ノードの対局で温度を選び直す必要がある。
自己対局の探索は変わらない。
実装は`crates/minase/src/search/mcts/engine.rs`にある。
