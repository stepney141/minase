# null moveはcutNodeでStockfishの静的評価の条件を満たすときだけ試し、置換表の打ち切りはPVノードでも行って零窓のノードではcutNodeとallNodeの区別を使う

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

null move pruningは、手番を相手に渡しても縮めた深さの探索の値がβ以上なら、そのノードを打ち切る手法である。
Stockfishは、null move pruningと置換表の値による打ち切りを、最善応手列の候補を読むPVノードでは行わない（[未導入の棋力向上手法の調査](../research/unplanned-strength-techniques-survey.md)）。
[棋力向上段階12](../plans/archived/strength-stage12.md)は、null moveを試す条件に非PVノードであることと補正後の静的評価がβ以上であることを加える変更を測った。
この変更により、[発動率の診断](../measurements/strength-stage12/strength-stage12-activation-diag.md)では、null moveを試していたノードの37.00%で試さなくなった。
そのSTCは上限の3,000ペアで判定保留となり、停止時点のLLRが−2.807と負だった（[測定記録](../measurements/strength-stage12/strength-stage12-null-move-stc.md)）。
PVノードでは置換表の値で打ち切らない変更の動機は、探索値が経路に依存することだったが、PVノードの打ち切りの大半は反復を検出しない静止探索で起きており、通常探索のPVノードで打ち切ったものは0.66%だった。
そのSTCは1,346有効ペアで`H0`だった（[測定記録](../measurements/strength-stage12/strength-stage12-pv-tt-cutoff-stc.md)）。

[旧ADR](archive/allow-null-move-and-tt-cutoff-at-pv-nodes-and-use-node-types-at-zero-window-nodes.md)は、2026年10月18日に、探索の再帰呼び出しで子がcutNodeかどうかを受け渡し、零窓のノードの置換表の打ち切りにこの区別を使うと決めた。
cutNodeは、親がβ打ち切りを見込んで零窓で読ませる子のノードであり、allNodeは、PVノードでもcutNodeでもないノードである（`src/search.cpp:746`）。
同じADRは、null moveを、ノードの種別と静的評価のβとの大小を条件にせずに試し続けると決めた。
null moveを試す条件は、残り深さ3以上、直前がnull moveでない、βが詰み帯と専用の帯の外、手番側に王駒以外の駒がある、の4つだった。
減深量は、基本量`(NullMoveBase + d × NullMoveSlope)/1200`に、静的評価がβを上回る量に応じた加算を上限3で足したものであり、加算は[一括採用のADR](adopt-revived-search-items-as-a-bundle.md)で導入された。
旧ADRは、ノード種別の予測を使えるようになった後も、cutNodeに限ってnull moveを試すStockfishの条件を自らの決定に含めず、[浅い前向き枝刈りの深さと対象の拡張](../plans/archived/shallow-pruning-extensions.md)の設計書に委ねていた。
また、[浅いノードの前向き枝刈りの旧ADR](archive/skip-static-eval-pruning-when-a-royal-is-attacked-and-extend-reverse-futility-to-depth-18.md)は、null move pruningには王駒への利きの判定を加えないと決めていた。

Stockfishは、cutNodeで`staticEval + 50 × priorNMPFailHigh ≥ β − 13 × depth − 47 × improving + 365`かつ`β ≥ −2000`のときだけnull moveを試す（`src/search.cpp:1037-1039`）。
`priorNMPFailHigh`は、同じ親の子のうち先に探索したノードで、null moveが打ち切りを起こした回数であり、親のノードを始めるときに0へ戻す（同830、1057、1074行）。
Stockfishには残り深さの下限がなく、王手を受けたノードは静的評価を使う枝刈りに進まないので、null moveも試さない（同1001行）。
減深量は`R = 7 + depth/3 + max((staticEval − β)/256, 0)`である（同1044行）。
2026年10月17日のエンジンの開発者の決定により、Stockfishにある項目は入れて形と値をStockfishに合わせ、中将棋の規則から来る王駒への利きの除外は残すことにした。
設計書は、この決定に従ってnull moveを試す条件と減深量を改めた。

## 決定

探索の再帰呼び出しで子がcutNodeかどうかを受け渡し、子に渡す値はStockfishの受け渡し（`src/search.cpp:1048, 1395-1455`）に従う。
根は`cutNode`を偽とするPVノードとする（同412行）。
PVノードはminaseの従来どおり探索窓の幅で区別し、窓の幅が1のノードのうちcutNodeでないものをallNodeとする。
この受け渡しは旧ADRから引き継ぐ。

null moveは、次の条件をすべて満たすノードで試す。

- cutNodeである。
- 直前の手がnull moveでない。
- αとβが詰み帯と専用の帯の外にある。
- 手番側に王駒以外の駒がある。
- 検証探索の中なら、根からの手数が検証探索の定める手数以上である。
- `β ≥ −P × 2000/208`が成り立つ。
- `e + P × NullMoveFailHighBonus × n/100 ≥ β + P × (NullMoveEvalMargin − NullMoveEvalDepth × d − NullMoveEvalImproving × [improving])/100`が成り立つ。
- 手番側のいずれの王駒にも相手の疑似利きが届かない。

`P`は歩兵価値、`d`は内部反復減深を適用した後の残り深さ、`e`は補正後の静的評価、`n`は`priorNMPFailHigh`に当たる回数である。
improvingは、null moveの判定の後の更新を反映する前の値を使う。
残り深さ3以上という条件は、Stockfishが持たないので外した。
手番側に王駒以外の駒があるという条件は、Stockfishの`non_pawn_material`に当たる条件として残す。
王駒への利きの条件は、Stockfishが王手を受けたノードでnull moveを試さないことに当たる、中将棋の条件である。

減深量`r`は、基本量`(NullMoveBase + d × NullMoveSlope)/1200`に`((e − β) × NullMoveEvalScale/100)/(2P)`を0以上に切り詰めて足した値とし、加算の上限3は外す。
null moveの子は、`R = 1 + r`として`max(0, d − R)`の深さで読む。
子の深さで引く1手は減深量の外にあるので、基本量に子の1手を足した量がStockfishの`7 + depth/3`に当たる。
詰みを探す局面では、着手1手分を含む`R`から、0方向へ丸めた`R/2`を引く。
この半減は旧ADRから引き継ぎ、詰みを探す局面の定義は[浅い前向き枝刈りのADR](extend-shallow-pruning-to-stockfish-ranges-and-skip-it-when-a-royal-is-attacked.md)にある。

null moveの子の値がβ以上なら、残り深さが16未満か検証探索の中ならそのまま打ち切り、それ以外なら[検証探索のADR](verify-null-move-cutoffs-at-remaining-depth-16-and-above.md)の検証探索を行う。
打ち切るたびに、親が子のために保持する回数`n`を1増やす。
打ち切りで返す値は、子の値が詰み帯と専用の帯にあればβとし、それ以外は子の値とする。
この扱いは[専用の帯のADR](reserve-a-path-dependent-band-for-repetition-outcomes.md)の決定であり、本決定では変えない。

置換表の値による打ち切りは、PVノードでは従来の条件のまま行う。
零窓のノードでは、Stockfishに合わせて、次の条件をすべて満たすときに限って打ち切る。
この条件は旧ADRから引き継ぐ。

- 置換表の値`v`が専用の帯の外にあり、置換表の深さ`d_tt`が要求深さ`d`について`d_tt > d − [v ≤ β]`を満たす。
- 値の種別が値の向きと合う。つまり、`v ≥ β`なら下界か正確な値、`v < β`なら上界か正確な値である。
- cutNodeで`v ≥ β`か、allNodeで`v < β`か、残り深さが4より深い（`src/search.cpp:899`）。
- 残り深さが7以上で、合法手であることを確かめた置換表の手があり、`v`が詰み帯の外にある場合は、その手を指した局面を置換表で引き、値がないか、`v ≥ β`の真偽と「着手後の局面の値の符号を反転した値 ≥ β」の真偽が一致する（同917-931行）。着手後の局面の値が専用の帯にある場合は、値がないものとして扱う。

## 検討した代案

ノードの種別と静的評価のβとの大小を条件にせずにnull moveを試す旧ADRの方式は、Stockfishに合わせるというエンジンの開発者の決定により改めた。
減深量の加算に上限3を置き続ける案と、残り深さ3以上の条件を残す案は、Stockfishにない制限なので採らなかった。
王駒への利きの条件を付けない旧来の扱いは、Stockfishが王手を受けたノードでnull moveを試さないことに当たる条件を欠くので改めた。
null moveの値が勝ちの値ならStockfishと同じく打ち切らない案は、詰み帯と専用の帯の扱いを変える設計書の範囲の外の問題なので、本決定では変えなかった（`src/search.cpp:1052-1076`）。

段階12で判定保留となった、非PVノードで補正後の静的評価がβ以上のときだけnull moveを試す案は、その形のまま棄却したままとする。
本決定は、試すノードを非PVノードより狭いcutNodeに限り、静的評価の条件に深さ、improving、および先行する打ち切りの回数に応じた余裕をStockfishの式で加え、その係数をSPSAで調整する点で、その案と異なる。
同時に残り深さ3以上の条件を外して減深量も改めたので、段階12の測定は本決定の形を測ったものではない。
本決定の形は、エンジンの開発者の決定によりStockfishに合わせて入れたものであり、この項目だけの対局測定はしていない。

旧ADRで棄却した案は、この決定でも棄却したままとする。
Stockfishのように探索のテンプレート引数でPVノードを区別する案は、窓の幅が1になったPVノードの扱いの差が起こる頻度が低いので採らない。
着手後の局面の値が専用の帯にあるときもその値で打ち切りを確かめる案は、経路に依存する値を打ち切りに使わない規則に反するので採らない。
Stockfishが50手規則で打ち切りを止める条件（`src/search.cpp:915`）は、中将棋に50手規則がないので持たない。
PVノードで置換表の値による打ち切りをやめる案は、上記のとおりSTCで`H0`だったので採らない。

## 結果

null moveを試す条件の判定は`crates/minase/src/search/alphabeta/negamax.rs`の`negamax`にあり、王駒への利きはほかの条件がすべて成立した後に`royal_under_attack`で調べる。
静的評価の条件、βの下限、および減深量は、`pruning.rs`の`NullMoveCoefficients::allows_null_move`、`null_move_beta_minimum`、および`NullMoveCoefficients::null_move_reduction`にあり、βの下限に使うStockfishの値2000は`NULL_MOVE_BETA_LIMIT`である。
先行する打ち切りの回数は`negamax.rs`の`prior_nmp_fail_high`に、打ち切りで返す値は`null_move_cutoff_score`に、詰みを探す局面の減深は`pruning.rs`の`seek_mate_null_move_reduction`にある。
受け渡しの規則は`negamax.rs`の`child_cut_node`、零窓の打ち切りの条件は`pruning.rs`の`zero_window_tt_cutoff`のdocコメントにある。
`invariants`機能つきのビルドでは、PVノードがcutNodeでないことを検査する。

新しい係数は`params.rs`の`NullMoveEvalMargin`、`NullMoveEvalDepth`、`NullMoveEvalImproving`、および`NullMoveFailHighBonus`であり、Stockfishの値を換算した175、6、23、および24から調整する。
既存の`NullMoveBase`、`NullMoveSlope`、および`NullMoveEvalScale`は、起点の4799、393、および133から、Stockfishの換算値7200、400、および163へ開始値を置き直した。
`NullMoveBase`と`NullMoveSlope`の範囲の上限は、Stockfishの換算値を含むように4800と400から10000と600へ広げた。
王駒への利きを調べるノードはnull moveの分だけ増え、その費用は係数で消せない。
cutNodeとallNodeの区別は、[LMRの減深量](archive/reduce-lmr-by-log-table-history-steps-and-stockfish-terms.md)、[IIRを行わない条件](reduce-depth-by-one-without-tt-move-except-at-all-nodes-and-on-the-previous-pv.md)、およびrazoringにも使う。
経路に依存する探索値が置換表を通じてPVへ入り得ることは、既知の近似として残る。

`minase dev bench`の深さ3、5、7の全15局面の探索ノード数の合計は、null moveの条件、減深量、および検証探索を入れたコミットで直前より1.6%増えた（[benchの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-bench.md)）。

2026年10月18日から19日にかけての30,000ペアのSPSAでは、試す条件の定数`NullMoveEvalMargin`が開始値175から−847へ下がり、null moveを試すノードを増やす側へ動いた。減深量の切片`NullMoveBase`は7,200から範囲の上限10,000の近くの9,985へ、傾き`NullMoveSlope`は400から474へ、静的評価の項の倍率`NullMoveEvalScale`は163から238へ上がり、減深量を増やす側へ動いた。打ち切り1回あたりの加算`NullMoveFailHighBonus`は24のままだった（[SPSAの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions.md)）。

この変更は、段階ゲートを通らずに採用した。
SPSAの値を反映した候補は、[前の設計書](../plans/archived/search-pruning-forms.md)を採用した版`a892a5e6`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、1,381ペアで`H1`となった（[STCの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-stc.md)）。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)）。
