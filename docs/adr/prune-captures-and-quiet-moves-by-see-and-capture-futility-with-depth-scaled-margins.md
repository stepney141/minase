# 捕獲手と静かな手をSEEと駒得の見積りで枝刈りし、余裕値を残り深さまたはlmrDepthに比例させてcapture historyで補正する

## ステータス

承認済み。決定日は2026年10月19日。

## コンテキスト

SEE（static exchange evaluation）は、着手の到達升での駒の取り合いを実際には読まず、取る駒と取り返される駒の価値の列から損得を見積もる手法である。
[旧ADR](archive/prune-losing-captures-by-see-at-shallow-nodes.md)は、2026年9月21日に、浅いノードでSEEの値が深さ別の余裕値の符号を反転した値を下回る捕獲手を展開しないと決めた。
当時のfutility pruningは、駒の増減がない手にだけ静的評価を上界として使えたので、捕獲手を対象にできなかった。
[棋力向上段階8](../plans/archived/strength-stage8.md)は、余裕値をチェスや本将棋の値から流用せず、深さごとに「失う良い結果の割合が10%以下の候補のうち枝刈りが最も多いもの」として[診断](../measurements/strength-stage8/strength-stage8-pruning-bench.md)から選んだ。
旧ADRは、余裕値を深さによらず1つにする案を、この診断で深さごとに適格な値が異なったので採らなかった。
静止探索と同じ余裕値0を全深さで使う案も、残り深さ2で失う割合が13.1%となり基準を超えたので採らなかった。
この診断は、評価関数がPSTだった時期のものである。
深さ別の余裕値`SeeMargin1`〜`3`は、その後のSPSAで調整され、起点では残り深さ1、2、3で歩兵価値の15%、97%、3%と、深さについて単調でなかった。

Stockfishは、捕獲手と王手になる手を同じ枝で扱い、SEEの値が`−(177 × depth + captHist × 34/1024)`を下回る手を読まない（`src/search.cpp:1205, 1220-1225`）。
`captHist`は、動かす駒、到達升、および取られる駒種で引くcapture historyの値である。
王手にならない捕獲手には、その前に捕獲手のfutility pruningを行い、静的評価に取る駒の価値、lmrDepthに比例する余裕値、およびcapture historyの項を足してもαに届かない手を読まない（同1208-1217行）。
王手にならない静かな手には、SEEの値が`−23 × max(lmrDepth, 0)²`を下回る手を読まない枝刈りを行う（同1255-1259行）。
いずれも深さの上限を持たず、PVノードでも働く。
minaseのcapture historyは、[capture historyのADR](bound-capture-history-and-use-it-in-qsearch.md)が定めるとおり、通常探索と静止探索の捕獲手の順序にだけ使われていた。

2026年10月17日のエンジンの開発者の決定により、Stockfishにある項目は入れて形と値をStockfishに合わせ、新しい係数の開始値はStockfishの値を換算した値とすることにした。
同じ決定により、成る手を枝刈りの対象から外すminaseの保護は残す。
[浅い前向き枝刈りの深さと対象の拡張](../plans/shallow-pruning-extensions.md)は、この決定に従って3つの枝刈りを実装した。
深さ別の余裕値は深さに比例する1つの係数に置き換わるので、旧ADRが診断に基づいて深さ別の値を選んだ判断は、この決定で覆る。

## 決定

捕獲手は、置換表の手と相手の最後の王駒を取る手を除いて、SEEの値が次の余裕値`m`の符号を反転した値を下回れば読まない。
王手になる静かな手も、置換表の手と成る手を除いて、同じ余裕値で判定する。
`m = max(0, P × SeeDepthMargin × d/100 + H(SeeCaptureHistory))`であり、`P`は歩兵価値、`d`は内部反復減深を適用した後の残り深さである。
`H(w) = (h × w/1024) × P/208`は、capture historyの値`h`に重み`w`/1024を掛けたStockfishの評価値の単位の量を、Stockfishの歩兵価値208で換算した項であり、除算は0方向へ丸める。
王手になる静かな手では`h`を0とする。
獅子の規則に依存してSEEが判定不能な手は読み、成る捕獲手は対象に含める。

捕獲手のfutility pruningは、置換表の手、成る手、相手の最後の王駒を取る手、および王手になる手を除く捕獲手について、lmrDepthが8未満で次の式が成り立てば、その手を読まない。
`e + P × (CaptureFutilityBase + CaptureFutilityLmrDepth × lmrDepth)/100 + V + H(CaptureFutilityHistory) ≤ α`であり、`e`は補正後の静的評価、`V`は取る駒の価値の合計である。
獅子、角鷹、および飛鷲が2枚を取る手では、2枚の価値を足す。
判定はSEEによる捕獲手の枝刈りより前に行う。

静かな手のSEEによる枝刈りは、静かな手のfutility pruningで枝刈りされなかった静かな手を対象とし、置換表の手、成る手、王手になる手、および経由升を持つ手を除いて、SEEの値が`−P × QuietSeeMargin × max(lmrDepth, 0)²/100`を下回れば読まない。
経由升を持つ手は、捕獲手のSEEでも判定不能として扱うので対象にしない。
静かな手のSEEは、到達升の駒の価値を0として交換列を作る点を除き、捕獲手のSEEと同じ駒の価値、取り返す駒の成り益、および獅子の規則による判定不能の扱いを使う。
捕獲かどうかは着手が実際に取る升の有無で決め、到達升に自駒が残るじっとも静かな手として扱う。

3つの枝刈りは、いずれも深さの上限を持たず、零窓のノードとPVノードの両方で行う。
静かな手のSEEによる枝刈りだけは、根から前の反復の主変化を辿るfollowPVのPVノードでは行わない。
適用するノードの共通の条件とlmrDepthの定義は、[浅い前向き枝刈りのADR](extend-shallow-pruning-to-stockfish-ranges-and-skip-it-when-a-royal-is-attacked.md)にある。
読まなかった手もLMRの表を引く手番号に数えるが、探索済みの手には加えないので、historyとcapture historyの減点とkiller手の更新の対象にならない。

## 検討した代案

深さ別の余裕値`SeeMargin1`〜`3`を使い続ける案は、深さの上限を外すと4以上の深さの値を決められず、Stockfishの形にもないので採らなかった。
旧ADRが診断で棄却した、全深さで余裕値を0にする案は、`SeeDepthMargin`の範囲の下限0とcapture historyの項の0での切り詰めによって形に含まれるので、選ぶかどうかはSPSAに委ねた。
Stockfishと同じく余裕値を0で切り詰めない案は、現行の`see_prunes`が非負の余裕値を契約とするので、本決定では採らなかった。
後続の[静止探索の補正と枝刈りの形の見直し](../plans/qsearch-forms.md)が、引数を符号付きの閾値へ移した後で切り詰めを外す。

成る手を捕獲手のfutility pruningと静かな手の2つのSEEの枝刈りの対象にする案は、成る手を枝刈りの対象から外す保護を残すというエンジンの開発者の決定により採らなかった。
王手になる静かな手のcapture historyの項を表から引く案は、minaseの表が取る駒の種類で引き、取る駒のない手の欄を持たないので採らなかった。
捕獲手のfutility pruningで取る駒の価値に手の順序付けの整列の値を流用する案は、その値がcapture historyの補正を含むので採らなかった。

YaneuraOuにだけある、捕獲手のSEEによる枝刈りの`α ≥ 引き分けの値`の条件は、エンジンの開発者の決定により入れなかった。
Stockfishの捕獲手のSEEによる枝刈りにある、最後の駒を捨てる手の除外（`src/search.cpp:1223-1224`）は持たない。
この除外は合法手のない側を引き分けとするステイルメイトのための条件であり、合法手のない側を負けとする中将棋には相当する仕組みがない（RULES.md第23条）。
YaneuraOuは、将棋ではSEEが負の静かな手でもその後に詰む場合があるので慎重に扱うべきだと注記しているが（`yaneuraou-search.cpp:3685-3686`）、Stockfishにある項目は入れるというエンジンの開発者の決定により、静かな手のSEEによる枝刈りを入れた。

旧ADRで棄却した案のうち、余裕値を深さによらず1つの定数にする案は、深さに比例する形を採るので棄却したままとする。

## 結果

capture historyは、手の順序に加えて、捕獲手のSEEによる枝刈りと捕獲手のfutility pruningの余裕値にも使う。
capture historyの上限は起点の係数`CaptureHistoryLimit`のままであり、Stockfishの上限10692（`src/history.h:135`）とは`h`の大きさが違うので、重みの意味は[historyの形の見直し](../plans/history-forms.md)が上限をStockfishの値に固定するまでStockfishと一致しない。
王手になる静かな手と王手にならない静かな手を分けるので、静かな手のSEEの判定と王手の判定の費用が加わる。
王手の判定は[王手の判定のADR](judge-gives-check-on-the-post-move-occupancy-without-precomputation.md)が定め、ほかの条件がすべて成り立った後に求める。

`minase dev bench`の深さ3、5、7の全15局面の探索ノード数の合計は、SEEによる捕獲手の枝刈りの余裕値を改めたコミットで直前より0.9%増え、捕獲手のfutility pruningを加えたコミットで13.6%減り、静かな手のSEEによる枝刈りを加えたコミットで10.7%減った（[benchの記録](../measurements/shallow-pruning-extensions/shallow-pruning-extensions-bench.md)）。
静かな手のSEEの判定は、benchの局面と固定シードのランダムな対局の局面の、経由升を持たない静かな手について、参照実装の値と照合した。

捕獲手と静かな手のSEEによる枝刈りの判定は`crates/minase/src/search/alphabeta/negamax.rs`の`negamax`にあり、SEEの判定契約は`see.rs`の`see_prunes`にある。
捕獲手のSEEの余裕値は`pruning.rs`の`SeeCoefficients::see_margin`、捕獲手のfutility pruningの見積りは`CaptureFutilityCoefficients::capture_futility_value`、lmrDepthの上限8は`CAPTURE_FUTILITY_LMR_DEPTH_LIMIT`、静かな手のSEEの余裕値は`quiet_see_margin`にある。
係数は`params.rs`の`SeeDepthMargin`、`SeeCaptureHistory`、`CaptureFutilityBase`、`CaptureFutilityLmrDepth`、`CaptureFutilityHistory`、および`QuietSeeMargin`であり、Stockfishの値を換算した85、34、113、119、134、および11から調整する。
`SeeMargin1`〜`3`は廃した。

<!-- SPSAとSTCの結果を完了時に書く -->
