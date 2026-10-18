# improvingは補正後の静的評価で判定し、futilityとreverse futilityの余裕値、late move pruningの上限、およびLMRの減深量に使う

## ステータス

承認済み。決定日は2026年10月18日。

## コンテキスト

improvingは、現局面の静的評価が同じ手番の2手前より高いかを表すフラグであり、チェスのエンジンは良化していないノードで枝刈りと減深を強める。
[旧ADR](archive/use-improving-only-for-futility-margins.md)は、2026年10月4日に、improvingを浅いノードのfutility pruningの余裕値を縮めることにだけ使い、良化の判定には補正前の静的評価を使うと決めた。
2手前がない場合と、比較の区間にnull moveが入る場合は良化しているとみなし、LMRの減深量はimprovingで変えないとした。
良化していないノードでLMRの減深量を1増やす案は、[棋力向上段階8](../plans/archived/strength-stage8.md)の[再診断](../measurements/strength-stage8/strength-stage8-improving-bench.md)で、減深によって良い結果を失う割合が15.0%から30.0%へ増え、基準の10ポイントを超えたので外した。
余裕値を縮める変更は、[不採用だった探索部の改良の再調整](../plans/archived/search-revival-spsa.md)で深さごとの縮小率をSPSAで調整し、8項目の一括採用の1つとして採用した（[採用のADR](adopt-revived-search-items-as-a-bundle.md)）。
補正後の静的評価で比べる案は、correction historyの学習につれて比較の基準が動くとして採らなかった。

2026年10月17日に、[探索の枝刈りとLMRの関数形の見直し](../plans/archived/search-pruning-forms.md)で、前向き枝刈りと減深の式をチェスのStockfishの形へ寄せることにした。
同じ日のエンジンの開発者の決定により、Stockfishにある項目は入れて形と値をStockfishに合わせ、中将棋の診断や測定で決めたminaseの既存の仕組みは残す。
Stockfishは、補正後の静的評価を同じ手番の2手前と比べる（`src/search.cpp:883`）。
2手前がない根の近くでは比較の相手を`VALUE_NONE`（同309行）とするのでimprovingは偽になり、null moveの区間にも例外を置かない。
null move pruningの後に静的評価がβ以上なら、improvingを真にする（同1080行）。
Stockfishのimprovingは、reverse futility pruningの余裕値、late move pruningの上限、およびLMRの減深量に入る（同1029行、1199行、1919行）。
reverse futility pruningの余裕値には、相手から見た評価が直前の手で悪くなったことを表すopponentWorseningも入る（同884行、1029行）。

## 決定

improvingは、各ノードの最初に求めて手数ごとに保持した補正後の静的評価を、2手前の局面の値と比べて判定する。
手数が2未満のノードではimprovingを偽とし、比較の区間にnull moveが入る場合の例外は置かない。
null move pruningの判定の後、静的評価がβ以上ならimprovingを真にし、その値をfutility pruning、late move pruning、およびLMRに使う。
opponentWorseningは、現局面の静的評価が、保持した1手前の局面の静的評価の符号を反転した値より大きいことをいう。
1手前の局面は相手の手番なので、これは相手から見た評価が直前の手で悪くなったことを表す。

良化していないノードでは、浅いノードのfutility pruningの余裕値に深さ別の縮小率を掛ける。
この縮め方は旧ADRから引き継ぐ。
reverse futility pruningの余裕値はimprovingとopponentWorseningで縮め、late move pruningの上限はimprovingでないときに縮め、LMRの表の値はimprovingでないときに割合で増やす。
これらの式と係数は、[reverse futility pruningのADR](skip-static-eval-pruning-when-a-royal-is-attacked-and-extend-reverse-futility-to-depth-18.md)、[late move pruningのADR](count-only-quiet-moves-for-late-move-pruning-and-shrink-the-limit-when-not-improving.md)、および[LMRのADR](reduce-lmr-by-log-table-history-steps-and-stockfish-terms.md)が定める。

## 検討した代案

補正前の静的評価で比べ続ける旧ADRの方式は、Stockfishに合わせるというエンジンの開発者の決定に反するので採らなかった。
2手前がない場合と比較の区間にnull moveが入る場合を良化とみなす旧ADRの例外は、Stockfishにない扱いなので廃した。
良化していないノードでLMRの減深量を一律に1増やす案は、上記の再診断で失う割合の増分が基準を超えたので、引き続き採らない。
本決定はその代わりに、Stockfishと同じく表の値に割合で上乗せする項を入れ、その強さをSPSAに選ばせた。

## 結果

improvingの判定は`crates/minase/src/search/alphabeta/negamax.rs`の`Searcher::improving`と`pruning.rs`の`is_improving`に、opponentWorseningは`is_opponent_worsening`にある。
futilityの縮小率は`params.rs`の`NonImprovingFutility1`から`NonImprovingFutility3`である。
LMRの項の係数`LmrNonImproving`は、2026年10月18日の30,000ペアのSPSAで開始値38から3へ動き、ほとんど働かない値になった（[SPSAの記録](../measurements/search-pruning-forms/search-pruning-forms.md)）。
null moveの前後は同じ盤面なので、null moveの直後のノードでは、1手前の値はnull moveをした局面の静的評価になり、Stockfishと同じ比較になる。
旧ADRが懸念した、correction historyの学習につれて比較の基準が動く影響は、測っていない。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、設計書を始めた時点のmaster`99ccc6bb`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、776ペアで`H0`となった（[STCの記録](../measurements/search-pruning-forms/search-pruning-forms-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
