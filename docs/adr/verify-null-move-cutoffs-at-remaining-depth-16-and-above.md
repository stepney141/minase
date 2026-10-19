# null moveの打ち切りは残り深さ16以上で検証探索により確かめる

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

null move pruningは、手番を相手へ渡しても縮めた深さの探索の値がβ以上なら、そのノードを打ち切る手法である。
この打ち切りは、手番を持つこと自体が不利になり、手番を渡せるなら有利になる局面（zugzwang）で誤る。
検証探索（verification search）は、null moveの打ち切りを、null moveを禁じた縮めた深さの通常探索で確かめ、値がβを下回れば通常の探索へ戻す機構である。

[旧ADR](archive/omit-null-move-verification-search.md)は、2026年9月6日に、検証探索を実装しないと決めた。
[棋力向上段階4](../plans/archived/strength-stage4.md)は、確かめた探索がβを下回る割合を矛盾率とし、1%未満なら実装しないという基準を測定前に定めた。
[診断bench](../measurements/strength-stage4/strength-stage4-nmp-verify-bench.md)では、縮めた深さが1以上のnull moveの打ち切りが根の深さ5で723回、根の深さ7で7,455回あり、矛盾はどちらも0件だった。
旧ADRは、この診断がnull moveの減深量をSPSAで調整する前で、評価関数がNNUEへ置き換わる前の構成で行ったものであり、再提案するときは現行の構成で矛盾率を測り直す必要があると記していた。

Stockfishは、null moveの探索がβ以上を返したとき、残り深さが16以上で検証探索中でなければ、同じノードを縮めた深さの零窓で探索し直し、その値もβ以上のときだけ打ち切る（`src/search.cpp:1055-1076`）。
検証探索の間は、根からの手数が`ply + 3 × (depth − R)/4`に達するまでnull moveを試さない（同1066行）。
YaneuraOuも同じ形を持つ（`yaneuraou-search.cpp:3278-3297`）。
2026年10月17日のエンジンの開発者の決定により、Stockfishにある項目は入れ、深さの下限のような値はStockfishの値で固定することにした。
[浅い前向き枝刈りの深さと対象の拡張](../plans/archived/shallow-pruning-extensions.md)は、この決定に従って検証探索を入れ、旧ADRの求めに従って矛盾率を測り直した。

測り直しでは、全15局面のbenchを深さ18と20で探索し、残り深さ16以上で検証探索を行って数えた（[benchの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-bench.md)）。
起点では、検証探索を持たないコミットを展開した使い捨ての写しに、打ち切りの判断を変えずに同じ条件の検証探索を追加で行って数える計測を入れた。
完了した検証探索は、起点で深さ18が1,520回、深さ20が5,260回、本決定を入れた構成で深さ18が502回、深さ20が3,112回であり、βを下回った数はいずれも0回だった。
起点の計測では、追加の探索で置換表などが更新されるので、計測のない起点とは探索が少し異なる。

## 決定

null moveの子の値がβ以上で、残り深さが16以上であり、検証探索の中でなければ、同じノードを、null moveの子と同じ深さ、零窓`(β − 1, β)`、および`cutNode`を偽として探索し直す。
検証探索の間は、根からの手数が、検証探索を始めたノードの手数にnull moveの子の深さの3/4を0方向へ丸めて足した値に達するまで、null moveを試さない。
検証探索の値もβ以上なら、null moveの打ち切りの値を返し、βを下回れば、そのノードの通常の探索を続ける。
検証探索の中では検証探索を重ねず、null moveの打ち切りをそのまま使う。
残り深さの下限16とnull moveを禁じる手数の割合3/4は、Stockfishの値で固定する。

検証探索は同じ手数で同じノードを探索し直すので、戻った後に、外側のノードの静的評価とttPvの印を検証探索の前の値に戻し、そのノードの最善応手列を消す。
Stockfishは同じノードの状態を上書きしたまま続けるが、minaseはノードの静的評価を局所変数と手数ごとの配列の両方に持ち、null moveの部分木でcorrection historyが更新されると両者が食い違うので、外側の値に戻す。

## 検討した代案

検証探索を入れない旧ADRの方式は、矛盾率が起点と本決定を入れた構成のどちらでも0%だったが、Stockfishにある項目は入れるというエンジンの開発者の決定により改めた。
検証探索を行う残り深さの下限やnull moveを禁じる手数の割合をSPSAで調整する案は、深さの条件のような値はStockfishの値で固定するというエンジンの開発者の決定により採らなかった。
Stockfishと同じく、検証探索の後もノードの状態を上書きしたまま続ける案は、上記のとおり静的評価の2つの写しが食い違うので採らなかった。

## 結果

[前の設計書の事前対局](../measurements/search-pruning-forms/search-pruning-forms-smoke.md)では、STCで反復深化が読み終えた深さの平均が約12手だった。
このためSTCでは残り深さ16以上のノードがほとんど現れず、検証探索は実質的に働かない。
LTCやそれより長い持ち時間での効果は測っていない。
矛盾率が0%なので、検証探索が打ち切りを覆す場面は、測った局面では見つかっていない。

検証探索は`crates/minase/src/search/alphabeta/negamax.rs`の`search_null_move_verification`にあり、null moveを禁じる手数は`nmp_min_ply`に保持する。
固定の定数は`pruning.rs`の`NULL_MOVE_VERIFICATION_MIN_DEPTH`、`NULL_MOVE_VERIFICATION_PLY_NUMERATOR`、および`NULL_MOVE_VERIFICATION_PLY_DENOMINATOR`である。
`search-stats`機能つきのビルドでは、完了した検証探索の数とβを下回った数を`null_move_verifications`と`null_move_verification_failures`として数える。
null moveを試す条件と減深量は[null moveのADR](try-null-move-only-at-cut-nodes-under-stockfish-conditions-and-allow-tt-cutoffs-at-pv-nodes.md)が定める。

検証探索を行う残り深さの下限16は固定の定数であり、SPSAの対象ではない。
STCでは残り深さ16以上のノードがほとんど現れないので、SPSAとSTCの結果は、この決定の効果をほとんど含まない。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、[前の設計書](../plans/archived/search-pruning-forms.md)を採用した版`a892a5e6`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、1,381ペアで`H1`となった（[STCの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
