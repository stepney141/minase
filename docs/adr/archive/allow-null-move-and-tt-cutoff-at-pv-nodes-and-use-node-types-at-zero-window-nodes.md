# null moveと置換表の打ち切りをPVノードでも行い、零窓のノードの打ち切りにはcutNodeとallNodeの区別を使う

## ステータス

置き換え済み。決定日は2026年10月18日。[null moveはcutNodeでStockfishの静的評価の条件を満たすときだけ試し、置換表の打ち切りはPVノードでも行って零窓のノードではcutNodeとallNodeの区別を使う](../try-null-move-only-at-cut-nodes-under-stockfish-conditions-and-allow-tt-cutoffs-at-pv-nodes.md)によって置き換えられた。

## コンテキスト

Stockfishは、null move pruningと置換表の値による打ち切りを、最善応手列の候補を読むPVノードでは行わない（[未導入の棋力向上手法の調査](../../research/unplanned-strength-techniques-survey.md)）。
それまでのminaseは、どちらもノードの種別を問わずに行っていた。
[旧ADR](allow-null-move-and-tt-cutoff-at-pv-nodes.md)は、2026年9月28日に、[棋力向上段階12](../../plans/archived/strength-stage12.md)の測定に基づいてこの方式を残すと決めた。
null moveを試す条件に、非PVノードであることと補正後の静的評価がβ以上であることを加える変更は、[発動率の診断](../../measurements/strength-stage12/strength-stage12-activation-diag.md)で、null moveを試していたノードの37.00%で試さなくなった。
そのSTCは上限の3,000ペアで判定保留となり、停止時点のLLRが−2.807と負だった（[測定記録](../../measurements/strength-stage12/strength-stage12-null-move-stc.md)）。
PVノードでは置換表の値で打ち切らない変更の動機は、探索値が経路に依存することだったが、PVノードの打ち切りの大半は反復を検出しない静止探索で起きており、通常探索のPVノードで打ち切ったものは0.66%だった。
そのSTCは1,346有効ペアで`H0`だった（[測定記録](../../measurements/strength-stage12/strength-stage12-pv-tt-cutoff-stc.md)）。
旧ADRは、β打ち切りが予想されるノードの予測を持ったうえで条件を加えるStockfishの方式を、ノード種別の予測の土台を要するので未測定と記していた。

2026年10月17日に、[探索の枝刈りとLMRの関数形の見直し](../../plans/archived/search-pruning-forms.md)で、前向き枝刈りと減深の式をチェスのStockfishの形へ寄せることにした。
同じ日のエンジンの開発者の決定により、Stockfishにある項目は入れて形と値をStockfishに合わせ、PVノードでの打ち切りのように中将棋の測定で決めた既存の仕組みは残す。
Stockfishは、探索の再帰呼び出しで、子がcutNodeかどうかを引数として受け渡す。
cutNodeは、親がβ打ち切りを見込んで零窓で読ませる子のノードであり、allNodeは、PVノードでもcutNodeでもないノードである（`src/search.cpp:746`）。
どちらも受け渡しの規則で決まる予測であり、実際にβ打ち切りが起きるかとは別である。
Stockfishは、零窓のノードの置換表の打ち切りにこの区別を使う（同894-934行）。
また、前の反復の根の評価値が大きい詰みを探す局面では、null moveの減深を半分にする（同1045行）。

## 決定

探索の再帰呼び出しで子がcutNodeかどうかを受け渡し、子に渡す値はStockfishの受け渡し（`src/search.cpp:1048, 1395-1455`）に従う。
根は`cutNode`を偽とするPVノードとする（同412行）。
PVノードはminaseの従来どおり探索窓の幅で区別し、窓の幅が1のノードのうちcutNodeでないものをallNodeとする。

置換表の値による打ち切りは、PVノードでは従来の条件のまま行う。
零窓のノードでは、Stockfishに合わせて、次の条件をすべて満たすときに限って打ち切る。

- 置換表の値`v`が専用の帯の外にあり、置換表の深さ`d_tt`が要求深さ`d`について`d_tt > d − [v ≤ β]`を満たす。
- 値の種別が値の向きと合う。つまり、`v ≥ β`なら下界か正確な値、`v < β`なら上界か正確な値である。
- cutNodeで`v ≥ β`か、allNodeで`v < β`か、残り深さが4より深い（`src/search.cpp:899`）。
- 残り深さが7以上で、合法手であることを確かめた置換表の手があり、`v`が詰み帯の外にある場合は、その手を指した局面を置換表で引き、値がないか、`v ≥ β`の真偽と「その値の符号を反転した値 ≥ β」の真偽が一致する（同917-931行）。着手後の局面の値が専用の帯にある場合は、値がないものとして扱う。

null move pruningは、ノードの種別と静的評価のβとの大小を条件にせずに試す。
詰みを探す局面では、着手1手分を含むnull moveの減深量を、0方向へ丸めたその半分だけ減らす。
詰みを探す局面の定義は、[reverse futility pruningのADR](skip-static-eval-pruning-when-a-royal-is-attacked-and-extend-reverse-futility-to-depth-18.md)にある。

## 検討した代案

Stockfishのように探索のテンプレート引数でPVノードを区別する案は採らなかった。
Stockfishは窓の幅が1になったPVノードをPVノードのまま扱って先頭の子をPVノードとして読むが、minaseは零窓のノードとして扱って子に零窓の規則を当てる。
この差は起こる頻度が低いので、規則を変えて合わせることはしない。
着手後の局面の値が専用の帯にあるときもその値で打ち切りを確かめる案は、経路に依存する値を打ち切りに使わない既存の規則（[ADR](../reserve-a-path-dependent-band-for-repetition-outcomes.md)）に反するので、設計書の判断で採らなかった。
Stockfishが50手規則で打ち切りを止める条件（`src/search.cpp:915`）は、中将棋に50手規則がないので持たない。

旧ADRで棄却した案は、この決定でも棄却したままとする。
null moveを非PVかつ補正後の静的評価がβ以上のノードに限る案は、上記のとおり判定保留でLLRが負だったので採らない。
PVノードで置換表の値による打ち切りをやめる案は、上記のとおりSTCで`H0`だったので採らない。

## 結果

null moveの適用条件は、残り深さ3以上、直前がnull moveでない、βが詰み帯と専用の帯の外、手番側に王駒以外の駒がある、の4つのままである。
2026年10月4日には、静的評価がβを上回る量に応じてnull moveの減深量を加算する変更が入ったが（[採用のADR](../adopt-revived-search-items-as-a-bundle.md)）、これも試すかどうかの条件ではない。
経路に依存する探索値が置換表を通じてPVへ入り得ることは、既知の近似として残る。
cutNodeに限ってnull moveを試すStockfishの条件は、ノード種別の予測を使えるようになったが、本決定には含めず、[浅い枝刈りの拡張の設計書](../../plans/archived/shallow-pruning-extensions.md)が扱う。
cutNodeとallNodeの区別は、[LMRの減深量](reduce-lmr-by-log-table-history-steps-and-stockfish-terms.md)と[IIRを行わない条件](../reduce-depth-by-one-without-tt-move-except-at-all-nodes-and-on-the-previous-pv.md)にも使う。

受け渡しの規則は`crates/minase/src/search/alphabeta/negamax.rs`の`child_cut_node`、零窓の打ち切りの条件は`pruning.rs`の`zero_window_tt_cutoff`、詰みを探す局面の減深は`seek_mate_null_move_reduction`のdocコメントにある。
`invariants`機能つきのビルドでは、PVノードがcutNodeでないことを検査する。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、設計書を始めた時点のmaster`99ccc6bb`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、776ペアで`H0`となった（[STCの記録](../../measurements/search-pruning-forms/search-pruning-forms-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](../adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
