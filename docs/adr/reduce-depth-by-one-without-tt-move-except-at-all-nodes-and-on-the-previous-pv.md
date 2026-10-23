# 置換表の記録手がないノードを1だけ浅く読み、allNodeと前の反復の主変化を辿るノードでは浅くしない

## ステータス

承認済み。決定日は2026年10月18日。

## コンテキスト

internal iterative reduction（IIR）は、置換表に記録手がないノードを浅く読み、次の反復で記録手を得てから本来の深さで読む手法である。
[旧ADR](archive/reduce-depth-by-one-without-tt-move.md)は、2026年9月12日に、残り深さ3以上で記録手がなければ1だけ浅く読むと決めた。
[棋力向上段階6](../plans/archived/strength-stage6.md)の[置換表と記録手の診断bench](../measurements/strength-stage6/strength-stage6-tt-bench.md)では、深さ6の残り深さ3以上のノードのうち7.7%に記録手がなかった。
記録手は置換表の助言的な情報であり、それまでは順序付けにだけ使っていたので、IIRはその有無を残り深さの決定にも使う点で置換表の契約を広げる変更だった。
STCは1,419有効ペアでLLR +2.965の`H1`だった（[測定記録](../measurements/strength-stage6/strength-stage6-iir-stc.md)）。
LTCは2,831有効ペアでLLR +2.971の`H1`だったが、外部のメモリ圧迫による停止で、候補6件と基準5件の計11件の時間切れが出た（[測定記録](../measurements/strength-stage6/strength-stage6-iir-ltc.md)）。
該当ペアを除いてもLLRは+3.114で判定は変わらなかったので、エンジンの開発者は、時間切れが0件であることを求めるLTCの採用条件から外れることを記録したうえで採用を決めた。
この判断は、後に[棋力測定の手引き](../guides/sprt.md)の「異常時の裁定」の節に先例として記された。

2026年10月17日に、[探索の枝刈りとLMRの関数形の見直し](../plans/archived/search-pruning-forms.md)で、前向き枝刈りと減深の式をチェスのStockfishの形へ寄せることにした。
Stockfishは、allNodeと、前の反復の主変化を辿るノードではIIRを行わない（`src/search.cpp:791-794, 1085`）。
後者のノードは、`followPV`という印で表される。
allNodeは、零窓のノードのうち、親がβ打ち切りを見込んで読ませる子（cutNode）でないものである。
Stockfishの深さの条件は6以上であり、minaseの3以上とは異なる。
同じ日のエンジンの開発者の決定により、Stockfishにある項目は入れ、深さ3の条件のように中将棋の測定で決めた既存の仕組みは残す。

## 決定

残り深さ3以上のノードで置換表の記録手がなければ、エントリがない場合も記録手を持たない場合も、残り深さを1減らして探索する。
ただし、allNodeと、前の反復の主変化を辿るノードでは減らさない。
根は主変化を辿るノードとし、子は、親が主変化を辿るノードであり、親から子への手が前の反復で確定した主変化の同じ手数の手と一致するときに限って、主変化を辿るノードとする。
置換表による即時打ち切りは要求された残り深さで判定し、減らした深さはその後の枝刈り、減深、および置換表への保存に使う。
根の探索には適用しない。

## 検討した代案

深さの条件をStockfishと同じ6以上にする案は、深さ3の条件が測定で決めた既存の仕組みなので、エンジンの開発者の決定により採らなかった。
Stockfishの2つの除外条件を入れない案は、Stockfishにある項目は入れるというエンジンの開発者の決定に反するので採らなかった。

旧ADRは、PVノードか否かで減深量を変える方式を、判定に窓の幅の追加条件を要し、STCの到達深さでは残り深さ3以上のノードが少数だとして採らなかった。
本決定はcutNodeとallNodeの区別を使ってallNodeを除くが、PVノードか否かで減深量を変える方式は、Stockfishにもないので引き続き採らない。

## 結果

適用の下限を3にしたのは、旧ADRの時点でLMR（late move reductions）とnull move pruningが働き始める深さと同じであり、記録手がない探索を浅く済ませて次の反復で記録手を得る意図に合ったからである。
本決定でもこの下限は、測定で決めた既存の仕組みとして残している。
記録手を合法性の検証前に着手として使わないこと、および評価値による即時打ち切りには検証済みの探索値だけを使うことは変わらない。
置換表の手は、合法手であることを確かめてから、枝刈りのために捕獲手かを判定することにも使う。
これらの契約は`crates/minase/src/search/alphabeta/tt.rs`の`Hit::best_move`のdocコメントにあり、条件の判定は`negamax.rs`の`negamax`にある。
null move pruningで早期に返るノードは置換表へ保存しないので、次の反復でも記録手がなく再び減深され得る。
cutNodeとallNodeの受け渡しの規則は、[置換表の打ち切りのADR](archive/allow-null-move-and-tt-cutoff-at-pv-nodes-and-use-node-types-at-zero-window-nodes.md)が定める。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、設計書を始めた時点のmaster`99ccc6bb`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、776ペアで`H0`となった（[STCの記録](../measurements/search-pruning-forms/search-pruning-forms-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc-on-the-measurement-machine.md)）。
