# continuation historyとpawn historyの値が低い静かな手を読まず、枝刈り用のlmrDepthをhistoryで増減する

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

[浅い前向き枝刈りの深さと対象の拡張](../plans/archived/shallow-pruning-extensions.md)は、静かな手のfutility pruningとSEEによる枝刈りに、手を読む深さからLMRの枝刈り前の減深量を引いた枝刈り用のlmrDepthを使う形を入れた（[枝刈りのADR](prune-captures-and-quiet-moves-by-see-and-capture-futility-with-depth-scaled-margins.md)）。
Stockfishは、これらの枝刈りの前に、1手前と2手前のcontinuation historyとpawn historyの3つの和が、残り深さに比例する閾値を下回る静かな手を読まず、続いてhistoryの値に応じてlmrDepthを増減する（`src/search.cpp:1227-1240`）。
minaseには、この2つの仕組みがなかった。

2026年10月17日のエンジンの開発者の決定により、Stockfishにだけある項目は入れる。
[historyの形の設計書](../plans/archived/history-forms.md)は、continuation historyとpawn historyを新設したうえで（[表の構成のADR](look-up-quiet-histories-in-butterfly-continuation-pawn-and-low-ply-tables.md)）、この2つの仕組みを入れた。

## 決定

静かな手のfutility pruningと同じ適用条件（前回の反復の主変化を辿るPVノードを除く）の下で、1手前と2手前の文脈のcontinuation historyとpawn historyの和が`−ContinuationPruning × d`を下回る手を読まない。
`d`はノードの残り深さであり、この判定はlmrDepthを使わない。
文脈の手がない項は0として足す。
王手になる手と、手番側の王駒に相手の疑似利きが届くノードは、既存の静かな手の枝刈りと同じく対象外とし、王手の判定は閾値の条件が成り立った後に行う。

読む手については、上の和にbutterfly historyの69/32倍を加えた値を`3000 + 7 × (min(d, 16) − 8)²`で割り、枝刈り用のlmrDepthに足してから、静かな手のfutility pruningとSEEによる枝刈りに渡す（`src/search.cpp:55-58, 1236-1240`）。
閾値の傾き`ContinuationPruning`はSPSAで調整し、butterfly historyの重みと除数の定数はStockfishの値で固定する。

## 検討した代案

枝刈りを入れない案は、Stockfishにだけある項目は入れるという決定に反するので採らなかった。
閾値をlmrDepthに比例させる案は、Stockfishが残り深さを使うので採らなかった。

## 結果

判定の式は`crates/minase/src/search/alphabeta/pruning.rs`の`continuation_history_prunes`、`history_lmr_divisor`、および`history_pruning_lmr_depth`のdocコメントにあり、適用の順序は`negamax.rs`にある。係数の現行値は`params.rs`にある。
変更の効果は、[探索ノード数と探索速度の記録](../measurements/history-forms/history-forms-bench.md)、[SPSAの記録](../measurements/history-forms/history-forms.md)、および[STCの記録](../measurements/history-forms/history-forms-stc.md)にある。
