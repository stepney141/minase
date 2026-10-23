# 捕獲手と静かな手をSEEの符号付きの閾値と駒得の見積りで枝刈りし、閾値を残り深さまたはlmrDepthに比例させてcapture historyで補正する

## ステータス

承認済み。決定日は2026年10月21日。

## コンテキスト

SEE（static exchange evaluation）は、着手の到達升での駒の取り合いを実際には読まず、取る駒と取り返される駒の価値の列から損得を見積もる手法である。
[2026年9月21日のADR](archive/prune-losing-captures-by-see-at-shallow-nodes.md)は、浅いノードでSEEの値が深さ別の余裕値の符号を反転した値を下回る捕獲手を展開しないと決めた。
当時のfutility pruningは、駒の増減がない手にだけ静的評価を上界として使えたので、捕獲手を対象にできなかった。
[棋力向上段階8](../plans/archived/strength-stage8.md)は、余裕値をチェスや本将棋の値から流用せず、深さごとに「失う良い結果の割合が10%以下の候補のうち枝刈りが最も多いもの」として[診断](../measurements/strength-stage8/strength-stage8-pruning-bench.md)から選んだ。
同じADRは、余裕値を深さによらず1つにする案を、この診断で深さごとに適格な値が異なったので採らなかった。
静止探索と同じ余裕値0を全深さで使う案も、残り深さ2で失う割合が13.1%となり基準を超えたので採らなかった。
この診断は、評価関数がPSTだった時期のものである。
深さ別の余裕値`SeeMargin1`〜`3`は、その後のSPSAで調整され、残り深さ1、2、3で歩兵価値の15%、97%、3%と、深さについて単調でなかった。

Stockfishは、捕獲手と王手になる手を同じ枝で扱い、SEEの値が`−(177 × depth + captHist × 34/1024)`を下回る手を読まない（`src/search.cpp:1205, 1220-1225`）。
`captHist`は、動かす駒、到達升、および取られる駒種で引くcapture historyの値である。
王手にならない捕獲手には、その前に捕獲手のfutility pruningを行い、静的評価に取る駒の価値、lmrDepthに比例する余裕値、およびcapture historyの項を足してもαに届かない手を読まない（同1208-1217行）。
王手にならない静かな手には、SEEの値が`−23 × max(lmrDepth, 0)²`を下回る手を読まない枝刈りを行う（同1255-1259行）。
いずれも深さの上限を持たず、PVノードでも働く。
minaseのcapture historyは、[2026年10月10日のADR](archive/bound-capture-history-and-use-it-in-qsearch.md)が定めたとおり、通常探索と静止探索の捕獲手の順序にだけ使われていた。

2026年10月17日のエンジンの開発者の決定により、Stockfishにある項目は入れて形と値をStockfishに合わせ、新しい係数の開始値はStockfishの値を換算した値とすることにした。
同じ決定により、成る手を枝刈りの対象から外すminaseの保護は残す。
[浅い前向き枝刈りの深さと対象の拡張](../plans/archived/shallow-pruning-extensions.md)は、この決定に従って3つの枝刈りを実装した。
深さ別の余裕値は深さに比例する1つの係数に置き換わるので、2026年9月21日のADRが診断に基づいて深さ別の値を選んだ判断は覆った。

[旧ADR](archive/prune-captures-and-quiet-moves-by-see-and-capture-futility-with-depth-scaled-margins.md)は、2026年10月19日に、この3つの枝刈りの形を定めた。
当時のSEEの判定`see_prunes`は非負の余裕値`m`だけを受け取り、交換の値が`−m`未満なら枝刈りする契約だったので、旧ADRは捕獲手のSEEの余裕値を`max(0, ·)`で0に切り詰めた。
Stockfishの捕獲手のSEEの余裕値は切り詰めず、capture historyの値が大きく負なら閾値は正になる（`src/search.cpp:1222`）。
旧ADRは、切り詰めを外すことを後続の[静止探索の補正と枝刈りの形の見直し](../plans/archived/qsearch-forms.md)に委ねた。
同じ設計書は、静止探索で`α − futilityBase`という正にもなり得る閾値でSEEを判定するので、`see_prunes`の引数を符号付きの閾値へ移し、捕獲手のSEEの余裕値の切り詰めを外した。

## 決定

`see_prunes`は駒価値と同じ単位の符号付きの閾値`T`を受け取り、交換の値が`T`未満なら枝刈りすると判定する。
等号では枝刈りせず、獅子の規則に依存して判定不能な交換は枝刈りせず（[判定不能の範囲のADR](mark-only-lion-rule-dependent-exchanges-unknown-in-see.md)）、経由升を持つ2段階移動の手は枝刈りしない。
呼び出し側は、合法な捕獲手か、経由升を持たない合法な静かな手を渡し、相手の最後の王駒を取る手を除く。

冒頭の早期判定は、初手の利得を`g₀`として、`g₀ ≥ T`かつ`取る駒の価値 − 動かす駒の着手前の価値 − 最大の成り益 ≥ T`なら、取り返しを列挙せずに偽（枝刈りしない）を返す。
最初の取り返しの後は、その利得を`g₁`として、`g₀ ≥ T`かつ`g₁ ≤ −T`なら、残る逆算を省いて偽（枝刈りしない）を返す。
交換列を逆算した結果は`min(g₀, max(−g₁, X))`の形をとるので、この条件なら後続の交換によらず閾値以上と分かるからである。
`g₀ < T`だけで直ちに真を返す早期判定は行わない。
後続の交換が獅子の規則に依存して判定不能になる場合に、判定不能なら枝刈りしないという契約を破るからである。

`P`を歩兵価値、`d`を内部反復減深を適用した後の残り深さとし、`A = trunc(P × SeeDepthMargin × d/100)`とする。
`H(w) = trunc(trunc(h × w/1024) × P/208)`は、capture historyの値`h`に重み`w`/1024を掛けたStockfishの評価値の単位の量を、Stockfishの歩兵価値208で換算した項である。
除算は0方向へ丸める。
各呼び出しの閾値は次のとおりである。

- 通常探索の捕獲手のSEEによる枝刈りは`T = −(A + H(SeeCaptureHistory))`を使う。
- 通常探索の王手になる静かな手のSEEによる枝刈りは、`h`を0とした`T = −A`を使う。
- 通常探索の静かな手のSEEによる枝刈りは`T = −trunc(P × QuietSeeMargin × max(lmrDepth, 0)²/100)`を使う。
- 静かな手の並べ替えで王手の加点を与えるかの判定は`T = −trunc(75P/208)`を使う（[ADR](order-quiets-by-the-stockfish-history-sum-and-update-histories-in-stockfish-form.md)）。
- killer手を先に返すかを決める先頭の捕獲手の判定は`T = 0`を使う（[ADR](try-killers-before-a-losing-first-capture.md)）。
- 静止探索のSEEの閾値と下限は、[静止探索の捕獲手の枝刈りのADR](prune-qsearch-captures-by-candidate-order-with-recapture-promotion-and-check-exemptions.md)が定める閾値を使う。

通常探索の捕獲手は、置換表の手と相手の最後の王駒を取る手を除いて、SEEの値が`−(A + H(SeeCaptureHistory))`を下回れば読まない。
王手になる静かな手も、置換表の手と成る手を除いて、`h`を0とした同じ式の閾値`−A`で判定する。
獅子の規則に依存してSEEが判定不能な手は読み、成る捕獲手は対象に含める。

捕獲手のfutility pruningは、置換表の手、成る手、相手の最後の王駒を取る手、および王手になる手を除く捕獲手について、lmrDepthが8未満で次の式が成り立てば、その手を読まない。
`e + P × (CaptureFutilityBase + CaptureFutilityLmrDepth × lmrDepth)/100 + V + H(CaptureFutilityHistory) ≤ α`であり、`e`は補正後の静的評価、`V`は取る駒の価値の合計である。
獅子、角鷹、および飛鷲が2枚を取る手では、2枚の価値を足す。
判定はSEEによる捕獲手の枝刈りより前に行う。

静かな手のSEEによる枝刈りは、静かな手のfutility pruningで枝刈りされなかった静かな手を対象とし、置換表の手、成る手、王手になる手、および経由升を持つ手を除いて、SEEの値が`−trunc(P × QuietSeeMargin × max(lmrDepth, 0)²/100)`を下回れば読まない。
経由升を持つ手は、捕獲手のSEEでも判定不能として扱うので対象にしない。
静かな手のSEEは、到達升の駒の価値を0として交換列を作る点を除き、捕獲手のSEEと同じ駒の価値、取り返す駒の成り益、および獅子の規則による判定不能の扱いを使う。
捕獲かどうかは着手が実際に取る升の有無で決め、移動元と到達升が同じじっとも静かな手として扱う。

通常探索の3つの枝刈りは、いずれも深さの上限を持たず、零窓のノードとPVノードの両方で行う。
静かな手のSEEによる枝刈りだけは、根から前の反復の主変化を辿るfollowPVのPVノードでは行わない。
適用するノードの共通の条件とlmrDepthの定義は、[浅い前向き枝刈りのADR](extend-shallow-pruning-to-stockfish-ranges-and-skip-it-when-a-royal-is-attacked.md)にある。
読まなかった手もLMRの表を引く手番号に数えるが、探索済みの手には加えないので、historyとcapture historyの減点とkiller手の更新の対象にならない。

## 検討した代案

捕獲手のSEEの余裕値を`max(0, A + H)`で0に切り詰める旧ADRの形は、Stockfishが切り詰めず、`see_prunes`が符号付きの閾値を受け取れるようになったので採らなかった。

深さ別の余裕値`SeeMargin1`〜`3`を使い続ける案は、深さの上限を外すと4以上の深さの値を決められず、Stockfishの形にもないので採らなかった。
2026年9月21日のADRが診断で棄却した、全深さで余裕値を0にする案は、`SeeDepthMargin`と`SeeCaptureHistory`をともに0とした形に含まれ、両者の範囲はいずれも0を含むので、選ぶかどうかはSPSAに委ねた。

成る手を捕獲手のfutility pruningと静かな手の2つのSEEの枝刈りの対象にする案は、成る手を枝刈りの対象から外す保護を残すというエンジンの開発者の決定により採らなかった。
王手になる静かな手のcapture historyの項を表から引く案は、minaseの表が取る駒の種類で引き、取る駒のない手の欄を持たないので採らなかった。
捕獲手のfutility pruningで取る駒の価値に手の順序付けの整列の値を流用する案は、その値がcapture historyの補正を含むので採らなかった。

YaneuraOuにだけある、捕獲手のSEEによる枝刈りの`α ≥ 引き分けの値`の条件は、エンジンの開発者の決定により入れなかった。
Stockfishの捕獲手のSEEによる枝刈りにある、最後の駒を捨てる手の除外（`src/search.cpp:1223-1224`）は持たない。
この除外は合法手のない側を引き分けとするステイルメイトのための条件であり、合法手のない側を負けとする中将棋には相当する仕組みがない（RULES.md第23条）。
YaneuraOuは、将棋ではSEEが負の静かな手でもその後に詰む場合があるので慎重に扱うべきだと注記しているが（`yaneuraou-search.cpp:3685-3686`）、Stockfishにある項目は入れるというエンジンの開発者の決定により、静かな手のSEEによる枝刈りを入れた。

2026年9月21日のADRで棄却した案のうち、余裕値を深さによらず1つの定数にする案は、深さに比例する形を採るので棄却したままとする。

## 結果

capture historyは、手の順序に加えて、捕獲手のSEEによる枝刈りと捕獲手のfutility pruningの余裕値にも使う。
capture historyの上限は、[capture historyのADR](update-capture-history-in-stockfish-form-and-use-it-in-ordering-qsearch-and-lmr.md)が定めるとおりStockfishと同じ10692であり、`H`の重みはStockfishと同じ大きさの`h`に掛かる。
capture historyの値が大きく負の捕獲手では捕獲手のSEEの閾値が正になり、その手はSEEの値がその閾値以上の得でなければ読まれない。
王手になる静かな手と王手にならない静かな手を分けるので、静かな手のSEEの判定と王手の判定の費用が加わる。
王手の判定は[王手の判定のADR](judge-gives-check-on-the-post-move-occupancy-without-precomputation.md)が定め、ほかの条件がすべて成り立った後に求める。

`minase dev bench`の深さ3、5、7の全15局面の探索ノード数の合計は、SEEによる捕獲手の枝刈りの余裕値を深さに比例させたコミットで直前より0.9%増え、捕獲手のfutility pruningを加えたコミットで13.6%減り、静かな手のSEEによる枝刈りを加えたコミットで10.7%減った（[benchの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-bench.md)）。
静かな手のSEEの判定は、benchの局面と固定シードのランダムな対局の局面の、経由升を持たない静かな手について、参照実装の値と照合した。
`see_prunes`を符号付きの閾値へ移したコミットは、benchの全15局面の深さ3、5、7で、探索ノード数、評価値、および最善手が直前のコミットと一致した（[benchの記録](../measurements/qsearch-forms/qsearch-forms-bench.md)）。
`see_prunes`は、負、0、正の閾値のそれぞれで、交換列を最後まで逆算した参照値と照合するテストで確かめている。
捕獲手のSEEの余裕値の切り詰めを外す変更は、静止探索のSEEの閾値と下限を加え、`DeltaMargin`の開始値を置き直した変更と同じコミットに入っており、そのコミットは同じ合計を直前より1.9%減らした。
切り詰めを外したことだけの効果は切り分けていない。

捕獲手と静かな手のSEEによる枝刈りの判定は`crates/minase/src/search/alphabeta/negamax.rs`の`negamax`にあり、SEEの判定契約は`see.rs`の`see_prunes`にある。
捕獲手のSEEの余裕値は`pruning.rs`の`SeeCoefficients::see_margin`、捕獲手のfutility pruningの見積りは`CaptureFutilityCoefficients::capture_futility_value`、lmrDepthの上限8は`CAPTURE_FUTILITY_LMR_DEPTH_LIMIT`、静かな手のSEEの余裕値は`quiet_see_margin`にある。
係数は`params.rs`の`SeeDepthMargin`、`SeeCaptureHistory`、`CaptureFutilityBase`、`CaptureFutilityLmrDepth`、`CaptureFutilityHistory`、および`QuietSeeMargin`であり、Stockfishの値を換算した85、34、113、119、134、および11から調整を始めた。
`SeeMargin1`〜`3`は廃した。

2026年10月18日から19日にかけての30,000ペアのSPSAでは、SEEの枝の`SeeDepthMargin`が開始値85から168、`SeeCaptureHistory`が34から122、捕獲手のfutility pruningの`CaptureFutilityBase`が113から504、`CaptureFutilityHistory`が134から475、静かな手のSEEの`QuietSeeMargin`が11から48へ上がり、いずれも枝刈りを控える側へ動いた（[SPSAの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions.md)）。
SPSAの値を反映した候補は、[前の設計書](../plans/archived/search-pruning-forms.md)を採用した版`a892a5e6`に対して、SPSAに使ったvast.aiの計算機で持ち時間を換算せずに行った`time=10000+100`のSTCで、1,381ペアで`H1`となった（[STCの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-stc.md)）。
係数の現行値は、その後のサブタスクのSPSAを経た値であり、`params.rs`にある。
2026年10月18日のエンジンの開発者の決定により、探索の関数形の各サブタスクはSTCの結果によらず採用し、7本を通して元のmasterより強くなったかは統括計画のフェーズ10のLTCで確かめる（[採用の例外のADR](adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc-on-the-measurement-machine.md)）。

形の変更の90係数を、vast.aiで借りた計算機で30,000ペアのSPSAにより調整した（[SPSAの記録](../measurements/qsearch-forms/qsearch-forms.md)）。
最終値を反映した版は、同じ計算機で持ち時間を換算せずに行ったSTCで、設計書を始めた時点のmasterに対して107ペアで`H1`（約+261 Elo）だった（[STCの記録](../measurements/qsearch-forms/qsearch-forms-stc.md)）。
逐次検定は境界を越えた時点で止まるので、この点推定は大きめに出やすく、採否には使っていない。
SPSAの最終値は、`SeeDepthMargin`が234から328へ上がり、`SeeCaptureHistory`は177から178へほとんど動かなかった。
