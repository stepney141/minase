# improvingは補正後の静的評価で判定し、reverse futility pruningの余裕値、null move pruningの条件、late move pruningの上限、およびLMRの減深量に使い、静かな手のfutility pruningの余裕値には使わない

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

improvingは、現局面の静的評価が同じ手番の2手前より高いかを表すフラグであり、チェスのエンジンは良化していないノードで枝刈りと減深を強める。
[旧ADR](archive/judge-improving-on-the-corrected-static-eval-and-use-it-in-pruning-and-lmr.md)は、2026年10月18日に、improvingを補正後の静的評価で判定し、futility pruningとreverse futility pruningの余裕値、late move pruningの上限、およびLMRの減深量に使うと決めた。
futility pruningでは、良化していないノードで深さ別の余裕値に深さ別の縮小率`NonImprovingFutility1`〜`3`を掛けていた。
良化していないノードでLMRの減深量を1増やす案は、[棋力向上段階8](../plans/archived/strength-stage8.md)の[再診断](../measurements/strength-stage8/strength-stage8-improving-bench.md)で、減深によって良い結果を失う割合が15.0%から30.0%へ増え、基準の10ポイントを超えたので外した。

Stockfishの静かな手のfutility pruningの式はimprovingを使わない（`src/search.cpp:1242-1252`）。
一方、null move pruningを試す条件には`−47 × improving`の項がある（同1037-1039行）。
この条件はnull moveの判定の後にimprovingを更新する処理（同1080行）より前にあるので、更新前の値を使う。
2026年10月17日のエンジンの開発者の決定により、Stockfishにある項目は入れて形と値をStockfishに合わせることにした。
[浅い前向き枝刈りの深さと対象の拡張](../plans/archived/shallow-pruning-extensions.md)は、この決定に従って静かな手のfutility pruningを手ごとの判定に改め、null move pruningにStockfishの条件を加えた。

## 決定

improvingは、各ノードの最初に求めて手数ごとに保持した補正後の静的評価を、2手前の局面の値と比べて判定する。
手数が2未満のノードではimprovingを偽とし、比較の区間にnull moveが入る場合の例外は置かない。
null move pruningの判定の後、静的評価がβ以上ならimprovingを真にする。
opponentWorseningは、現局面の静的評価が、保持した1手前の局面の静的評価の符号を反転した値より大きいことをいう。
1手前の局面は相手の手番なので、これは相手から見た評価が直前の手で悪くなったことを表す。
これらの定義は旧ADRから引き継ぐ。

null move pruningの判定の後に更新する前のimprovingは、reverse futility pruningの余裕値とnull move pruningを試す条件に使う。
reverse futility pruningの余裕値はimprovingとopponentWorseningで縮め、null move pruningを試す条件では、improvingのときに静的評価に求める余裕を`NullMoveEvalImproving`だけ小さくする。
更新した後のimprovingは、late move pruningの上限とLMRの減深量に使う。
late move pruningの上限はimprovingでないときに縮め、LMRの表の値はimprovingでないときに割合で増やす。
LMRの減深量のうち枝刈りより前に決まる部分はlmrDepthにも入るので、更新後のimprovingは、静かな手と捕獲手のfutility pruningと静かな手のSEEによる枝刈りにもlmrDepthを通じて影響する。
静かな手のfutility pruningの余裕値には、improvingの項を置かない。

これらの式と係数は、[reverse futility pruningのADR](extend-shallow-pruning-to-stockfish-ranges-and-skip-it-when-a-royal-is-attacked.md)、[null moveのADR](try-null-move-only-at-cut-nodes-under-stockfish-conditions-and-allow-tt-cutoffs-at-pv-nodes.md)、[late move pruningのADR](apply-late-move-pruning-at-all-depths-and-pv-nodes-counting-only-quiet-moves.md)、および[LMRのADR](archive/reduce-lmr-by-log-table-history-steps-and-stockfish-terms.md)が定める。

## 検討した代案

良化していないノードで静かな手のfutility pruningの余裕値を深さ別の縮小率で縮め続ける旧ADRの方式は、Stockfishの静かな手のfutility pruningの式がimprovingを使わないので採らなかった。
null move pruningを試す条件にimprovingを使わない旧来の方式は、Stockfishにある項目は入れるというエンジンの開発者の決定により改めた。

旧ADRで棄却した案は、この決定でも棄却したままとする。
補正前の静的評価で比べる方式は、Stockfishに合わせるというエンジンの開発者の決定に反するので採らない。
2手前がない場合と比較の区間にnull moveが入る場合を良化とみなす例外は、Stockfishにない扱いなので置かない。
良化していないノードでLMRの減深量を一律に1増やす案は、上記の再診断で失う割合の増分が基準を超えたので採らない。
その代わりに、Stockfishと同じく表の値に割合で上乗せする項を置き、その強さをSPSAに選ばせる。

## 結果

improvingの判定は`crates/minase/src/search/alphabeta/negamax.rs`の`Searcher::improving`と`pruning.rs`の`is_improving`に、opponentWorseningは`is_opponent_worsening`にある。
null move pruningを試す条件の項の係数は`params.rs`の`NullMoveEvalImproving`であり、Stockfishの値を換算した23から調整する。
`NonImprovingFutility1`〜`3`は廃した。
null moveの前後は同じ盤面なので、null moveの直後のノードでは、1手前の値はnull moveをした局面の静的評価になり、Stockfishと同じ比較になる。
旧ADRが記したとおり、correction historyの学習につれて比較の基準が動く影響は、測っていない。

2026年10月18日から19日にかけての30,000ペアのSPSAでは、null moveを試す条件のimprovingの項`NullMoveEvalImproving`が開始値23から76へ上がり、reverse futility pruningのimprovingの項`ReverseFutilityImproving`は144から163、late move pruningの`LmpNonImproving`は31から26になった（[SPSAの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions.md)）。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、[前の設計書](../plans/archived/search-pruning-forms.md)を採用した版`a892a5e6`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、1,381ペアで`H1`となった（[STCの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc-on-the-measurement-machine.md)）。
