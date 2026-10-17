# historyの更新量の形とcontinuation historyの設計書

## 先に読む要約

本書は、静かな手（駒を取らない手）の成績を学習するhistoryの更新と使い方を、チェスのStockfishと将棋のYaneuraOu（やねうら王）が共通に持つ形へ寄せ、現行のmasterより強くなるかを確かめる。
minaseのbutterfly history（手番、移動元、移動先ごとの静かな手の成績の表）は、β打ち切り（ある手の値が探索窓の上端βに達し、残りの手を読まずに打ち切ること）のたびに深さの2乗だけ加減点するので、1回の更新が表の上限に占める割合が両エンジンの数十分の1と小さい（[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)）。
本書が変えるのは5つである。
1つ目に、加点と減点の量を、深さの2乗に比例する形からStockfishの上限つき1次式へ置き換え、減点を先に読んだ手ほど大きくする（history bonus/malus、malus decay）。
2つ目に、置換表（探索結果を局面ごとに保存する表）による打ち切り、fail low（全手がα以下）のノードの直前の手、および最善手が置換表の手だった場合にもhistoryを更新する。
3つ目に、直前の相手の手と2手前の自分の手を文脈として静かな手の成績を引くcontinuation historyの表を新設し、手の並べ替えとLMRに使う（1手前の表はcounter move historyとも呼ばれる）。
4つ目に、late move reductions（LMR、後方の手を浅く読む減深）のhistory補正を、閾値による段差からhistory値に比例する連続量へ置き換える。
5つ目に、LMRの対象を捕獲手とkiller手（同じ深さの別のノードでβ打ち切りを起こした静かな手）へ広げる。
1つ目と4つ目は形そのものを置き換えるので、両エンジンの値を換算した開始値から始める。
残りの新しい係数は現行と同じ探索になる値を開始値とする。
すべての係数を既存の係数とまとめてSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で調整し、起点のmasterと短時間条件STCおよび長時間条件LTCの2段階の逐次検定（段階ゲート）で対局させ、1つの候補として採否を決める。
完了条件は、候補の採否が測定記録とともに確定することである。

## 状態

起案。2026年10月17日に起案した。
[浅い枝刈りの深さと対象の拡張](shallow-pruning-extensions.md)の完了を待って着手し、着手は利用者の指示による。
設計判断の表の後に挙げた4点は、利用者の判断を待っている。
次の一手は、その判断を得て前の設計書の完了後に起点を確定し、フェーズ1（形の変更の実装）を始めることである。

## 目的

本書が確かめるのは、「historyの更新量を両エンジンの形へ寄せ、continuation historyとLMRのhistory補正を加えて係数を調整すれば、現行の形で調整した探索より強くなる」という仮説である。
現行の加点は`min(深さ² × HistoryBonusScale/100, HistoryLimit)`であり、起案時点の既定値（尺度100%、上限17408）では、深さ10の加点は上限の約0.6%にとどまる。
Stockfishの加点は深さ10で上限7183の約15%であり（`src/search.cpp:2004-2006, 2015`）、値が上限に近づくほど更新量を小さくする重力式（後述）による忘却が働く。
minaseでは1回の更新が小さいので、重力式による忘却はほとんど働かず、根の探索ごとの減衰（`HistoryDecay`）が忘却の役割をほぼ担っていると推測される。
また、直前の手を文脈とする表は、[手の順序付けの改善](archived/move-ordering.md)の診断で、表を対局内で持ち越し、根の探索ごとに23%へ減衰させる条件で、静かな手の順序を変えるノードがSTC相当で44.6%（1手前）と61.3%（2手前）あると分かったが、表の大きさの設計が残っていた（[診断の記録](../measurements/move-ordering-diag.md)）。
本書は、これらの形を同じ候補にまとめ、係数を調整し直して測る。

## 適用範囲

本書の用語を次のように定める。

- **起点**は、前の設計書が完了した後、フェーズ1を始める時点のmasterの先頭のコミットである。既存の係数の現行値（`HistoryLimit`、`LmrDivisor`、`CaptureHistoryLimit`など）は、起点の`crates/minase/src/search/alphabeta/params.rs`の表の既定値を指す。本書が例に挙げる値は起案時点の値である。
- **形の変更**は、本書の5項目を実装したコミット列であり、新しい係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で`params.rs`の表の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **bench**は、固定の15局面を指定の深さで探索し、探索ノード数、評価値、最善手、および探索速度を出力する`minase dev bench`である。
- **ワーカー**は、1つの探索を並列に進める探索スレッドであり、その数はUSIオプション`Threads`で決まる。historyの表はワーカーごとに持つ。
- **butterfly history**は、`history.rs`の`HistoryTable`であり、手番側、移動元、移動先で引く静かな手の成績である。値の上限は`HistoryLimit`である。
- **重力式**は、符号つきの更新量`b`で値`v`を`v + b − trunc(v×|b|/上限)`へ置き換える現行の更新式である（`ordering.rs`の`record_quiet_beta_cutoff`）。`trunc`は0の方向への切り捨てである。更新量の絶対値が上限以下なら、値は上限の範囲に収まる。
- **静かな手の段**は、手選択器（`ordering.rs`の`MovePicker`）が置換表の手、捕獲手、killer手を返した後に、残りの静かな手をhistory値の降順に返す区切りである。
- **着手の記録**は、`Searcher`がplyごとに持つ、そのplyのノードで指した手の`Move`と着手前の駒種の組である。ply `p`の記録は、ply `p`の局面からply `p + 1`の局面へ進めた手を表す。着手後の駒（成る手なら成った駒）は、`Move`の成りの指定と着手前の駒種から導く。null moveはそのplyの記録を空にし、根より前のplyは記録を持たない。[correction historyの鍵の追加の設計書](correction-history-tables.md)のC3も、同じ記録を使う。
- **文脈の手**は、ply `p`のノードで静かな手を引くときに参照する、ply `p − 1`とply `p − 2`の記録の手である。記録が空のplyは文脈の手を持たない。1手前がnull moveでも、2手前に記録があれば、その手は2手前の文脈の手になる。
- **improving**は、`negamax.rs`の`improving`が定める判定であり、同じ手番の2手前の局面より静的評価が上がったことをいう。
- **PVノード**は窓の幅が1より大きいノード、**零窓のノード**は窓の幅が1のノードである。
- **i16**は、−32768から32767までを表す16ビットの符号つき整数である。
- **PST**と**FM**は、既定の評価関数がNNUEに置き換わる前に使っていた評価関数の部品であり、それぞれ駒と升ごとの評価値の表と、2駒の関係による評価の補正である。`FmScale`はFMの補正に掛ける倍率であり、NNUEでは使わないのでSPSAの対象から外す。
- 減深量と深さの単位は、探索の深さ1を1手と数える。

対象は次の5項目である。

- butterfly historyの加点と減点の形と、減点の逓減（M1）。
- historyを更新する条件の拡大（M2）。
- continuation historyの新設と、手の並べ替えおよびLMRへの使用（M3）。
- LMRのhistory補正の連続量への置き換え（M4）。
- LMRの対象の捕獲手とkiller手への拡大（M5）。

次の事項は対象外とする。

- 根でのLMRと、根の手による履歴の更新。根の探索は`root.rs`の`search_root`が別に行い、手の並べ替えは前回の反復の部分木ノード数で決まる（[ADR](../adr/order-root-moves-by-previous-subtree-nodes.md)）。根でLMRを行うには、減深した探索の結果を途中結果の採用（`RootResults`）へ記録しない扱いも決める必要があり、根の手は全ノードのごく一部なので、固定の予算のSPSAに係数を1つ増やす価値が小さい。
- β打ち切りを起こさずにαを上げたノード（PVノードの正確な値）での更新。Stockfishは最善手があれば更新するが（`src/search.cpp:1602-1608`）、この変更には係数がなく、killer表を更新するかどうかも別に決める必要がある。minaseの正確な値のノードは零窓のノードに比べて少ないので、本書では扱わない。
- Stockfishの加点のうち、非PVノードで探索した手数に比例して加点を増やす項、直前の手のhistoryの合計（Stockfishの変数名でstatScore）の項、反駁された直前の手のcontinuation historyへの追加の減点（`src/search.cpp:909-910, 2033-2035`）、およびfail lowの加点を局面の条件で増減する項（同1614-1621）。いずれも係数を増やすほどの根拠をminaseでは持たない。
- 3手前、4手前、6手前の表、王手を受けているか直前の手が捕獲かで表を分けること、pawn history、およびlow ply history。1手前と2手前は診断で効果を確かめた範囲であり、StockfishのLMRもこの2つだけを使う（`src/search.cpp:1375-1378`）。minaseには王手の概念がない。
- 捕獲履歴の更新量の形（深さの2乗）。M5は捕獲履歴を上限で正規化してLMRに使い、値の水準は既存の`CaptureHistoryLimit`と新しい係数の調整に委ねる。
- history値による枝刈り（Stockfishのcontinuation history based pruning、`src/search.cpp:1229-1235`）と、futility pruningの深さへのhistoryの反映。history値の低い手だけを切る枝刈りは、[late move pruningのADR](../adr/count-only-quiet-moves-for-late-move-pruning.md)に記録された診断で適格な候補がなかった。
- 表の初期値を負の値にすること（Stockfishの`src/search.cpp:719-727`）と、continuation historyのワーカー間の共有。
- correction historyにcontinuationの鍵を加えること。[correction historyの鍵の追加の設計書](correction-history-tables.md)が扱う。

## 依存関係

起点は、[浅い枝刈りの深さと対象の拡張](shallow-pruning-extensions.md)が完了した後のmasterの先頭である。
前の設計書の候補が採用されたかどうかにかかわらず、起点の表の既定値をSPSAの開始値とする。
前の設計書の候補がmasterへ統合されると`params.rs`の既定値と探索が変わり、本書の検証（起点との探索ノード数の一致）の基準も変わるので、実装を含めて前の設計書の完了後に始める。
系列の順序は提案であり、最終的な順序は利用者が決める。

M4とM5は、LMRの減深量を1/100手単位の整数で計算し、最後に手単位へ直す形を前提にする。
この形は[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF4が導入するが、F4の採否ではなく起点の実装で判断する。
起点のLMRが1/100手単位で計算されていなければ、F4の設計判断「LMRの減深量の単位」と同じ方法で減深量の表を1/100手単位に改める変更を、探索ノード数が一致するコミットとしてフェーズ1の最初に入れる。
起点に`LmrNonImproving`、`LmrBase`、`LmrPvReduction`、`LmrMaxReduction`がなければ、本書の式の該当する項は0、上限は3として読み、これらの係数は加えない。
前の設計書が手の成績の値を新しい箇所で読むようにした場合も、本書の組合せ値（M3）を使うのは手の並べ替えとLMRだけとし、その他の箇所はbutterfly historyの値のまま変えない。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと共用するので、同時対局数は16を上限とする。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は次のADRの決定の一部を変える。
採用した場合は、完了時に、起点の時点で有効なADR（先行の設計書が置き換えていればその後継）を確認し、新しいADRで置き換える。
部分的に変える場合は、残る決定も新しいADRへ引き継ぐ。

- [LMRの式のADR](../adr/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)（2026年9月7日）は、history補正を閾値による±1の段差とし、連続量の案を比例係数と切り詰めの2つの自由度を絞る診断が要るとして棄却した。対象は置換表の手、捕獲手、killer手を除く非捕獲手とした。M4は段差を連続量に置き換え、比例係数はSPSAで決める。切り詰めは手単位への整数化の後の上下限だけにする。M5は対象を捕獲手とkiller手へ広げる。このADRは評価関数がPSTだった時期の測定に基づき、上限3と段差を「STCの到達深さが5〜6だった時期の判断」として見直しの余地を留保している。同じADRの上限は[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF4も変える。
- [静かな手の順序のADR](../adr/order-quiets-with-malus-and-pst-tiebreak.md)（2026年10月10日）は、加点と減点を別の尺度で深さの2乗に比例させ、重力式で更新すると決めた。M1は加点と減点を上限つき1次式に置き換え、減点の逓減を加える。M2は更新する場面を増やす。重力式と同点破りは変えない。このADRはPSTを既定の評価関数としていた時期の測定に基づき、同点破りの効果をNNUEで測り直していないことと、尺度がSPSAの調整を待つ既定値であることを記している。
- [historyの持ち越しのADR](../adr/carry-history-tables-within-a-game.md)（2026年10月10日）は、butterfly historyと捕獲履歴を対局内で減衰させて持ち越し、killer表と補正表は持ち越さないと決めた。M3はcontinuation historyの表を加え、butterfly historyと同じ時点で初期化して対局内で持ち越すが、根の探索ごとの減衰はかけない。このADRはPSTとFMを既定の評価関数としていた時期の測定に基づき、continuation historyの設計を別の設計書に委ねている。
- [butterfly表だけのADR](../adr/keep-only-the-butterfly-history-table.md)（2026年9月7日）は、静かな手のhistoryをbutterfly表だけで構成すると決め、直前の手と組み合わせる表は対象外とした。M3は並べ替えとLMRに使うhistory値へcontinuation historyを加える。駒種と到達升で引く表（piece-to history）は加えない。このADRはPSTの時期の測定に基づき、4次元の表が疎だという判断は表を根の探索ごとに初期化していた時期のものなので、再提案するときは現行の表の寿命で測り直す必要があると留保している。
- [捕獲履歴のADR](../adr/bound-capture-history-and-use-it-in-qsearch.md)（2026年10月10日）は、捕獲履歴を捕獲手の並べ替えと静止探索の群内の順序に使うと決めた。M5は捕獲手のLMRにも使う。並べ替えの決定は変えない。このADRはPSTの時期の測定に基づき、LMRへの使用については留保していない。

[late move pruningの数え方のADR](../adr/count-only-quiet-moves-for-late-move-pruning.md)と[killer手を損な先頭捕獲より先に返すADR](../adr/try-killers-before-a-losing-first-capture.md)の決定は変えない。
killer手はlate move pruningで数えず、手選択器の段の順序も変わらない。

## 設計判断

| 項目 | 決定 |
|---|---|
| 候補の束ね方 | 5項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回の段階ゲートで採否を決める。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。項目ごとにSPSAと段階ゲートを繰り返す案は、1項目あたり約82〜114時間を要するので採らない。代償として、項目ごとの寄与は測らない。不採用のときに一部の項目だけを選び直して再測定しない。 |
| 開始値の方針 | M2、M3、M5の新しい係数は、開始値で現行と同じ探索になる値（追加の更新の重み、continuation historyの重み、捕獲手とkiller手の減深の倍率は0）とし、探索ノード数の一致で検証する。M1とM4は形そのものを置き換えるので、現行と同じ探索になる開始値を持てない。両者は両エンジンの値の換算を開始値とし、benchのノード数の変化を記録する。現行の形と新しい形を併存させて開始値を現行に合わせる案は、同じ働きの項を重複させて係数を増やすだけなので採らない。 |
| 加点と減点の形（history bonus/malus） | 現行の深さの2乗の項（`HistoryBonusScale`、`HistoryMalusScale`）を廃し、Stockfishの上限つき1次式`min(傾き×d − 切片, 上限)`に置き換える（`src/search.cpp:2004-2006`）。YaneuraOuも同じ形である（`yaneuraou-search.cpp:5308-5310`）。 |
| M1の開始値 | Stockfishの値を、表の上限に対する比で換算した値とする（倍率は`HistoryLimit`の起案時点の値17408を7183で割った約2.42）。重力式の働きは更新量と上限の比で決まるので、比をそろえれば同じ速さで忘却する。現行の加点に1次式を当てはめる案は、1回の更新が上限の約0.6%という現行の水準を引き継ぎ、本書が確かめたい水準の差を開始時点で試さないので採らない。SPSAの1回のセッションで係数が動ける量は範囲の幅の一部であり、当てはめた値から換算値の水準（深さ10で約27倍）まで動く保証はない。 |
| 上限の単位 | 加点と減点の上限は`HistoryLimit`に対する百分率とし、範囲を100以下にする。重力式の値を上限の範囲に保つには、更新量の絶対値が`HistoryLimit`以下でなければならないからである。Stockfishも更新量を表の上限で切り詰める（`src/history.h:70-76`）。減点の上限はStockfishでは深さ3以上で効くので、減点の形に欠かせない。 |
| 減点の逓減（malus decay） | 同じノードで先に読んだ静かな手を、読んだ順に、1手ごとに直前の手の減点量の`HistoryMalusDecay`%へ縮めた量で減点する。最初に読んだ手が最も大きく減点される。Stockfishは約90%（921/1024、`src/search.cpp:2018-2022`）、YaneuraOuは約95%（977/1024、`yaneuraou-search.cpp:5327`）である。範囲の上限を100にするのは、100を超えると後の手の減点が`HistoryLimit`を超え、重力式の値が上限の範囲を外れ得るからである。M1と同じくStockfishの値から始める。 |
| 更新の条件の拡大 | 置換表で打ち切ったときの静かな置換表の手への加点（TT cutoff bonus）、fail lowのノードでの直前の静かな手への加点（prior countermove bonus）、およびβ打ち切りを起こした手が置換表の手だったときの加点の上乗せ（TT move bonus）を、それぞれ開始値0の係数で加える（Stockfish `src/search.cpp:902-906, 1611-1634, 2004-2005`）。Stockfishがfail lowの加点を直前の手の成績や評価値の変化で増減する項は、1つの重みにまとめる。負の値は良い手を減点することになり意味がないので、範囲の下限を開始値の0とする。 |
| 置換表の手の確認 | 置換表で打ち切ったときの加点は、置換表の手が現局面で合法な静かな手であることを`MoveGenerator::is_legal_move`で確かめてから行う。置換表の手はハッシュの衝突で別の局面のものである場合があり、continuation historyを引くには移動元の駒種が要るからである。 |
| continuation historyの文脈の深さ | 1手前と2手前の2表とする。[手の順序付けの改善](archived/move-ordering.md)の診断はこの2表の効果だけを確かめた。StockfishとYaneuraOuは並べ替えに1、2、3、4、6手前の表を使うが（Stockfish `src/movepick.cpp:233-239`、YaneuraOu `movepick.cpp:362-368`）、LMRの補正には1手前と2手前だけを使う（Stockfish `src/search.cpp:1375-1378`、YaneuraOu `yaneuraou-search.cpp:3930-3932`）。 |
| continuation historyの添字 | 1表を「文脈の手の着手前の駒種、文脈の手の到達升、現在の手の着手前の駒種、現在の手の到達升」の`[29][144][29][144]`とし、要素はi16とする。29は成駒を含む駒種の数（`PIECE_KIND_COUNT`）、144は盤の升の数である。升は、表を引く手を指す側が後手なら盤を180度回転させた升（升の番号`s`を`143 − s`）で引く。中将棋は初期配置と規則が180度の回転と手番の入れ替えについて対称なので、先手の局面と回転した後手の局面の統計を同じ要素に集められる。手番の次元を加える案は、1表が約70 MB、2表で約140 MBになり、要素あたりの更新も半分になるので採らない。回転も手番の次元もない添字は、先手番と後手番の別の型の統計が同じ要素に混ざるので採らない。 |
| continuation historyの大きさと寿命 | 1表は17,438,976要素で約35 MB、2表でワーカーあたり約70 MBである。表はbutterfly historyと同じ時点（新規対局、受理した規則と置換表の容量`USI_Hash`の変更、ワーカー数の変更）で0にし、対局内で持ち越す。根の探索ごとの減衰はかけない。約70 MBの読み書きを根の探索ごとに行うと、STCの1手あたりの思考時間に対して無視できない時間がかかり、Stockfishも減衰させるのはbutterfly historyだけである（`src/search.cpp:341-343`）。忘却は重力式に任せる。効果の根拠とした診断は根の探索ごとに23%へ減衰させる条件で測ったものであり、減衰させない本書の条件での発動率は測っていない。 |
| continuation historyの上限 | 表の値の上限は新しい係数`ContinuationHistoryLimit`とし、範囲をi16に収まる32000以下とする。`HistoryLimit`の範囲は65536まであり、i16に収まらないからである。更新量は、butterfly historyと同じ加点と減点を、この上限で切り詰めて使う。 |
| 組合せ値 | 手の並べ替えとLMRのhistory値には、butterfly historyの値に、2表の値の和の`ContinuationHistoryWeight`%を加えた組合せ値を使う。表ごとに重みを分ける案は、Stockfishの並べ替えの重みが両表で同じであり（`src/movepick.cpp:235-236`）、LMRの重みもほぼ同じ（1126と1093）なので採らない。文脈の手を持たない表の項は和に含めない。根のノードには文脈の手がないので、根の並べ替えは変わらない。 |
| LMRのhistory補正（statScoreによる減深の増減） | 閾値による±1手の段差（`LmrHistoryThreshold`）を廃し、組合せ値に比例する連続量に置き換える。M1で値の水準が深さ10で約27倍に上がると、起案時点の閾値146（範囲0〜512）を超える手がほとんどになり、段差は値の符号だけで決まる補正になる。段差と連続量を併存させる案は、同じ働きの項を重複させるので採らない。連続量は、組合せ値を`HistoryLimit`で割った比に`LmrHistoryScale`を掛けた1/100手とする。上限で割るのは、`HistoryLimit`の調整で値の水準が変わっても係数の意味を保つためである。開始値はStockfishの換算値とする。 |
| LMRの対象の拡大 | 捕獲手とkiller手にも、静かな手と同じ基本量から補正項を引いた1/100手単位の減深量へ倍率（`LmrCaptureScale`、`LmrKillerScale`）を掛けて適用する。捕獲手の補正項は、捕獲履歴の値を`CaptureHistoryLimit`で割った比に別の係数`LmrCaptureHistoryScale`を掛けたものとする。捕獲履歴と組合せ値は更新量の水準が異なるので、係数を分ける。両エンジンは捕獲手にも静かな手と同じ式を使うので（Stockfish `src/search.cpp:1371-1381, 1395`）、倍率100%が両エンジンの形に当たる。倍率は補正項を加えた後に掛けるので、倍率0なら補正項にかかわらず減深量は0になる。置換表の手と先頭の手を減深しないことは変えない。 |
| 重みが0のときの処理 | 重みや倍率が0でも、表の参照と更新、および置換表の手の合法性の確認を省かない。省く分岐を置くと、フェーズ1で測る探索速度に、SPSAで重みが動いた後の費用が現れないからである。 |
| SPSAの対象 | 起点の表のすべての係数（`FmScale`を除く）から、廃する3係数（`HistoryBonusScale`、`HistoryMalusScale`、`LmrHistoryThreshold`）を除き、新しい16係数を加える。新しい係数だけを調整する案は、既存の係数との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の係数を含むので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 総ペア数を除き、既定の設定を流用する。3,750反復、1反復8ペア（計30,000ペア）、持ち時間10秒＋0.1秒、同時対局数16、`c_end`（最後の反復の摂動幅）は範囲の1/6、`r_end`（最後の反復の学習率）は0.002である（[SPSAの手引き](../guides/spsa.md)）。系列の先行の設計書をすべて採用した場合、係数の数は本書の時点で75前後になる（起点の表で数え直す）。30,000ペアでこの数を十分に調整できるかは未検証であり、不採用はこの予算と設定で改善を示さなかったことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。最終値を最適値とは呼ばない。総ペア数は、2026年10月17日の利用者の決定により、[SPSAの手引き](../guides/spsa.md)の標準の3,000ペアではなく30,000ペアとする。Stockfishは100〜170個の探索の定数を一度に調整するときに数万ペアを使っており（コミット90c83c38は約124個を12.5万局、約6.25万ペアで調整した）、標準の3,000ペアでは係数が多い本書の調整に足りないおそれがあるからである。 |
| 新しい係数の範囲 | 両エンジンの対応する値を範囲に含め、開始値を範囲の内側に置く。開始値0の重みと倍率、および上限の百分率は、負の値と100を超える値に意味がないので、開始値または両エンジンの値が範囲の端に来ることがある。[利得のADR](../adr/decay-spsa-gains-from-one-sixth-of-the-range.md)は、こうした範囲の端による序盤の摂動の切り詰めを許容しており、係数ごとの摂動幅の設定は加えない。 |

次の4点は、利用者の判断を待つ。

1. M1とM4の開始値を、Stockfishの換算値とすること。代案は現行の加点と減点への当てはめであり、開始時点の探索は現行に近いが、本書が確かめたい更新量の水準の差を開始時点で試さない。
2. continuation historyの添字を、180度の回転で先手と後手の統計を共有する形にすること。手番の次元を加える案より表が半分になるが、盤の対称性を前提にする。
3. 増えるメモリへの対応。continuation historyはワーカーあたり約70 MBを使う。lishogiのBotは`Threads`を4としているので（`minase-lishogi-bot/config.yml`）、1局あたり約280 MB増え、同時対局数を掛けた分だけコンテナのメモリ上限の見直しが要る。採用した場合のBotの更新の前に、上限を確かめる。
4. 根でのLMRと根の手による履歴の更新を対象外とすること。

## 変更の内容

以下の式で、`d`はノードの残り深さ（置換表で打ち切った場合は要求深さ、それ以外はIIRの後の深さ）、`L`は`HistoryLimit`、`[条件]`は条件が成り立てば1、成り立たなければ0を表す。
IIR（internal iterative reduction）は、置換表に記録手がない残り深さ3以上のノードを1だけ浅く読む現行の規則であり、適用しないノードではIIRの後の深さは要求深さに等しい。
重力式の除算は現行どおり0の方向へ切り捨てる。
それ以外の除算は、断りのない限り負の無限大の方向へ切り捨てる。

### M1　butterfly historyの加点と減点の形（history bonus/malus、malus decay）

加点の基本量`b₀`と減点の量`m₀`を次の式にする。

```math
b_0(d) = \min\left(\mathrm{HistoryBonusSlope} \times d - \mathrm{HistoryBonusBase},\ \left\lfloor \frac{L \times \mathrm{HistoryBonusCap}}{100} \right\rfloor\right)
```

```math
m_0(d) = \min\left(\mathrm{HistoryMalusSlope} \times d - \mathrm{HistoryMalusBase},\ \left\lfloor \frac{L \times \mathrm{HistoryMalusCap}}{100} \right\rfloor\right)
```

β打ち切りを起こした静かな手への加点は、`b₀(d)`に、その手が置換表の手ならM2の上乗せ（TT move bonus）を足し、0以上`L`以下に切り詰めた量とする。
同じノードで先に読んだ静かな手のうち`k`番目（0から数える）への減点は、`m₀(d)`を0以上に切り詰めた値を`m⁽⁰⁾`とし、`m⁽ᵏ⁾ = floor(m⁽ᵏ⁻¹⁾ × HistoryMalusDecay/100)`で求める。
`HistoryMalusCap`と`HistoryMalusDecay`は100以下なので、減点は`L`を超えない。
更新は現行の重力式で行い、killer表の更新は変えない。
開始値では現行と探索が変わるので、benchのノード数の変化を記録する。

### M2　historyを更新する条件の拡大

次の3つの場面で加点する。
減点とkiller表の更新は行わない。
更新先は、butterfly historyと、M3のcontinuation historyの両方である。

置換表で打ち切ったとき（TT cutoff bonus）は、置換表の値がβ以上で、置換表の手が現局面で合法な静かな手なら、その手に`floor(b₀(d) × HistoryTtCutoffBonus/100)`を0以上`L`以下に切り詰めて加点する。
置換表の値がα以下で打ち切った場合は更新しない。

fail lowのとき（prior countermove bonus）は、ノードの値が探索開始時のα以下で、このノードへ至った手（1つ前のplyの記録の手）が静かな手（null moveでも捕獲手でもない手）なら、その手を指した側のbutterfly historyのその手の要素と、その手を現在の手とするcontinuation historyの要素に、`floor(b₀(d) × HistoryPriorBonus/100)`を加点する。
切り詰めは表ごとに行い、butterfly historyでは0以上`L`以下、continuation historyでは0以上`ContinuationHistoryLimit`以下とする（M3）。
その手の文脈の手は、その手より1手前と2手前の記録の手である。

β打ち切りを起こした静かな手が置換表の手だったとき（TT move bonus）は、M1の加点に`HistoryTtMoveBonus`点を上乗せしてから切り詰める。

3つの係数が0のとき、更新量が0になり、重力式は値を変えないので、M2の前のコミットと同じ探索になる。

### M3　continuation history（counter move historyとfollow-up history）

`HistoryTables`に、1手前の文脈の表`C₁`と2手前の文脈の表`C₂`をワーカーごとに加え、`Searcher`に着手の記録を加える。
ply `p`のノードで現在の手の表の値`c₁`、`c₂`を次のように引く。

```math
c_1 = C_1[k_{p-1}][\rho(t_{p-1})][k][\rho(t)], \qquad c_2 = C_2[k_{p-2}][\rho(t_{p-2})][k][\rho(t)]
```

ここで`k`と`t`は現在の手の着手前の駒種と到達升、`k_{p−i}`と`t_{p−i}`はply `p − i`の記録の着手前の駒種と到達升、`ρ`は、表を引く手（参照では現在の手、M2のfail lowの加点では1つ前のplyの手）を指す側が後手なら升を180度回転し、先手ならそのまま返す写像である。
文脈の手の升も、表を引く手を指す側を基準に回転する。
文脈の手を持たない表は、参照も更新もしない。

β打ち切りの加点と減点、およびM2の加点では、butterfly historyと同じ更新量を`ContinuationHistoryLimit`以下に切り詰め、`ContinuationHistoryLimit`を除数とする重力式で、文脈の手を持つ表に適用する。
手の並べ替えの静かな手の段とLMRでは、次の組合せ値`H`をhistory値として使う。

```math
H = h_{\mathrm{bf}} + \left\lfloor \frac{\mathrm{ContinuationHistoryWeight} \times \sum_{i \in A} c_i}{100} \right\rfloor
```

ここで`h_bf`はbutterfly historyの値、`A`は1と2のうち文脈の手を持つ表の番号の集合である。
同点破り（線形経路の差分、生成順）は変えない。
`ContinuationHistoryWeight`が0のとき、`H = h_bf`となり、M3の前のコミットと同じ探索になる。

### M4とM5　LMRの減深量（statScoreによる補正、捕獲手とkiller手への拡大）

置換表の手と先頭の手を除く各手の減深量を、次の順で1本の式として求める。

まず、1/100手単位の基本量`r₀`を求める。

```math
r_0 = \left\lfloor \frac{t \times (100 + \mathrm{LmrNonImproving} \times [\text{improvingでない}])}{100} \right\rfloor + \mathrm{LmrBase} - \mathrm{LmrPvReduction} \times [\text{PVノード}]
```

ここで`t`は残り深さと手番号から引く1/100手単位の表の値`floor(100 × ln d × ln m/(LmrDivisor/100))`であり、`m`は置換表の手と先頭の手も含めて0から数えた手番号である。
減深の対象は先頭の手を除くので`m ≥ 1`であり、表は手番号0の要素を0とする現行の持ち方のままとする。

次に、補正項`q`を引いて`r = r₀ − q`とする。
静かな手とkiller手では`q = floor(H × LmrHistoryScale / L)`、捕獲手では捕獲履歴の値`h_c`から`q = floor(h_c × LmrCaptureHistoryScale / CaptureHistoryLimit)`とする。
後続の[correction historyの鍵の追加の設計書](correction-history-tables.md)のC6が加える`LmrCorrection`の項も、この位置で引く。

続いて手の種類の倍率`s`を掛けて整数化し（`r`が1/100手単位、`s`が百分率なので、10000で割って手単位にする）、0以上`min(LmrMaxReduction, d ⊖ 2)`以下に切り詰める。

```math
\mathrm{reduction} = \min\left(\max\left(\left\lfloor \frac{r \times s}{10000} \right\rfloor, 0\right),\ \min(\mathrm{LmrMaxReduction}, d \ominus 2)\right)
```

`s`は、静かな手なら100、killer手なら`LmrKillerScale`、捕獲手なら`LmrCaptureScale`である。
`d ⊖ 2`は`max(d − 2, 0)`であり、残り深さ2以下では減深量は0になる。
減深した手は残り深さ`d − 1 − reduction`で読み、探索開始時のαを上回った場合に通常の深さで読み直す。

M4は静かな手の段差を補正項`q`に置き換えるので、開始値では現行と探索が変わり、benchのノード数の変化を記録する。
M5は倍率0から始めるので、捕獲手とkiller手の減深量は0のままで、M5の前のコミットと同じ探索になる。

### 新しい係数

廃する係数は`HistoryBonusScale`、`HistoryMalusScale`、および`LmrHistoryThreshold`である。
Stockfishの値は、butterfly historyでは上限の比17408/7183 ≈ 2.42を掛け、捕獲履歴では上限の比で正規化して換算した。

| 係数 | 開始値 | 範囲 | 範囲の根拠 |
|---|---|---|---|
| `HistoryBonusSlope`（加点の深さ1あたりの増分、履歴点） | 283 | 0〜600 | Stockfishは133×899/1024 ≈ 117点で、換算すると約283点（`src/search.cpp:2005, 2015`）。YaneuraOuは128×806/1024 ≈ 101点で、換算すると約244点（`yaneuraou-search.cpp:5309, 5319`） |
| `HistoryBonusBase`（加点の切片、履歴点） | 172 | −200〜600 | Stockfishは81×899/1024 ≈ 71点で換算すると約172点、YaneuraOuは77×806/1024 ≈ 61点で約147点 |
| `HistoryBonusCap`（加点の上限、`HistoryLimit`に対する百分率） | 18 | 5〜100 | Stockfishは1487×899/1024を7183で割って約18%、YaneuraOuは1529×806/1024で約17% |
| `HistoryMalusSlope`（減点の深さ1あたりの増分、履歴点） | 2655 | 0〜5000 | Stockfishは968×1159/1024 ≈ 1096点で、換算すると約2,655点（`src/search.cpp:2006, 2018`）。YaneuraOuは882×1113/1024 ≈ 959点で、換算すると約2,323点（`yaneuraou-search.cpp:5310, 5321`） |
| `HistoryMalusBase`（減点の切片、履歴点） | 645 | −500〜2000 | Stockfishは235×1159/1024 ≈ 266点で換算すると約645点、YaneuraOuは204×1113/1024 ≈ 222点で約537点 |
| `HistoryMalusCap`（減点の上限、`HistoryLimit`に対する百分率） | 35 | 5〜100 | Stockfishは2244×1159/1024を7183で割って約35%、YaneuraOuは2122×1113/1024で約32% |
| `HistoryMalusDecay`（先に読んだ手1つごとの減点の倍率、百分率） | 90 | 70〜100 | Stockfishは約90%、YaneuraOuは約95%。100は逓減しない現行の形に当たる |
| `HistoryTtCutoffBonus`（置換表の打ち切りでの加点、`b₀`に対する百分率） | 0 | 0〜200 | Stockfishは131×深さで、深さ5では通常の加点の約128%（`src/search.cpp:906`）。YaneuraOuは`min(130×深さ − 71, 1043)`で約131%（`yaneuraou-search.cpp:2739-2740`） |
| `HistoryPriorBonus`（fail lowの直前の手への加点、`b₀`に対する百分率） | 0 | 0〜400 | Stockfishの重みは局面の条件で0〜約666の`bonusScale`に比例し、深さ5で通常の加点の0〜約570%になる（`src/search.cpp:1614-1625`）。YaneuraOuも同じ形である（`yaneuraou-search.cpp:4321-4336`） |
| `HistoryTtMoveBonus`（最善手が置換表の手のときの上乗せ、履歴点） | 0 | 0〜2000 | Stockfishは364×899/1024 ≈ 320点で換算すると約775点、YaneuraOuは353×806/1024 ≈ 278点で約673点 |
| `ContinuationHistoryWeight`（組合せ値に加える2表の値の倍率、百分率） | 0 | 0〜200 | 両エンジンの並べ替えはbutterfly historyの2倍に各表の値を足すので、butterfly historyに対して50%。StockfishのLMRは1126/2252と1093/2252で約50%と49%（`src/search.cpp:1375-1378`） |
| `ContinuationHistoryLimit`（表の値の上限、履歴点） | 30000 | 4096〜32000 | 両エンジンは30000（Stockfish `src/history.h:138`、YaneuraOu `history.h:249`）。上限はi16に収まる値とする |
| `LmrHistoryScale`（静かな手とkiller手で、組合せ値が`HistoryLimit`に等しいときの減深の減少、1/100手） | 165 | 0〜400 | butterfly historyが上限のとき、Stockfishは2252/1024 × 7183 × 439/4096/1024で約1.65手、YaneuraOuは2 × 7183 × 428/4096/1024で約1.47手（Stockfish `src/search.cpp:1375-1381`、YaneuraOu `yaneuraou-search.cpp:3930-3937`） |
| `LmrCaptureHistoryScale`（捕獲手で、捕獲履歴が`CaptureHistoryLimit`に等しいときの減深の減少、1/100手） | 112 | 0〜400 | Stockfishは捕獲履歴が上限10692のとき10692 × 439/4096/1024で約1.12手、YaneuraOuは10692 × 428/4096/1024で約1.09手（Stockfish `src/search.cpp:1371-1373`、YaneuraOu `yaneuraou-search.cpp:3925-3926`） |
| `LmrCaptureScale`（捕獲手の減深量の倍率、百分率） | 0 | 0〜150 | 両エンジンは捕獲手にも同じ式を使うので100に当たる |
| `LmrKillerScale`（killer手の減深量の倍率、百分率） | 0 | 0〜150 | 両エンジンはkiller手を持たず、同じ手を静かな手として減深するので100に当たる |

## 所要時間の見積り

時間は、同時対局数16の測定機での見積りである。
SPSAは、[SPSAの手引き](../guides/spsa.md)が記録する同時対局数16で3,000ペアの完走実績（27,138秒、1ペア約9秒）に基づき、30,000ペアを約75時間（約3日）と見込む。
2026年10月17日の別のセッションでは1反復あたり約1分で進んだが、暫定の観測である。
1反復の時間は持ち時間で決まるので、係数の数が増えても、continuation historyで探索速度が下がっても、大きくは変わらない。
段階ゲートの時間は、判定までのペア数が強さの差で変わるので幅を持たせた。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 形の変更の実装と検証（codexへ委任） | 1日程度 |
| 2 | 時間切れの事前確認（13反復、104ペア） | 約15分 |
| 3 | SPSA（30,000ペア） | 約75時間 |
| 4 | 反映、テスト、および事前対局（32ペア） | 約1時間 |
| 5 | 候補のSTC | 約1〜8時間 |
| 6 | 通過した場合のLTC | 約5〜30時間 |

段階2から段階6までの測定機の占有は、STCで棄却される場合に約77〜84時間（約3.2〜3.5日）、LTCまで進む場合に約82〜114時間（約3.4〜4.8日）となる。

## 実装フェーズ

基本シードは、時間切れの事前確認を3800000000、セッションを3850000000、事前対局を3900000000、STCを3950000000、LTCを4000000000と仮に定める。
これらは起案時点で、保存済みの測定とSPSAの基本シード（`data/matches/`と`data/spsa/`の`manifest.json`）、および他の設計書が予約した値から、各測定のペア数以上離れた未使用の値である（[教訓](../lessons/derive-seed-adjacent-collision.md)）。
着手時に同じ方法で未使用であることを確かめ直し、使われていれば設計書を書き換える。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ1　形の変更の実装

起点からブランチ`history-forms`を切り、worktree `data/worktrees/history-forms`で作業する。
起点のLMRが1/100手単位でない場合は、依存関係の節のとおり、その単位の変更を最初のコミットにする。
続いて、探索を変えない項目を先に、M2、M3、M5の順に1項目1コミットで入れ、探索を変えるM1とM4を最後に置く。
M2とM5はM3の表と組合せ値を使わずに書けるので、M2の加点のうちcontinuation historyへの更新はM3のコミットで加える。
M5のコミットでは、静かな手の補正項は起点の段差のままとし、killer手にも同じ段差を使う。
killer手と捕獲手は倍率0なので、このコミットでは減深量が0のままである。
M4のコミットで、静かな手とkiller手の補正項を連続量に置き換える。
M1より前のコミットでは、M2の`b₀(d)`を起点の加点の式`min(d² × HistoryBonusScale/100, HistoryLimit)`として読む。
M2、M3、M5の各コミットでは、benchの全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が起点と一致することを確かめる。
M1とM4の各コミットでは同じbenchのノード数を記録し、直前のコミットとの差を残す。
M3はノードごとに表の参照と更新を加え、M2は置換表の打ち切りで合法性を確かめるので、`taskset -c 3`で1つのコアに固定した`minase dev bench --depth 9 --repetitions 3`の中央値を、起点とM5までのコミットで交互に3回ずつ測り、探索速度の変化を記録する（[教訓](../lessons/pin-bench-to-performance-core.md)）。
同じ実行で最大常駐メモリも記録し、増分がワーカーあたり約70 MBであることを確かめる。
新しい係数は`params.rs`の表へ追加し、廃する3係数を表から削除する。
調整用ビルド（`--features tuning`つきのビルド）が新しい係数を`Tune_<係数名>`のUSIオプションとして宣言し、廃した係数を宣言しないことを確かめる。
新しい式の単体テストは、式の写しではなく数値の例で書く（[テストの手引き](../guides/testing.md)）。
着手の記録とcontinuation historyのテストは、null moveがそのplyの記録を空にすること、文脈の手を持たない表は参照も更新もしないこと、1手前がnull moveでも2手前の記録があれば2手前の表を参照すること、根のノードでは文脈の手がないこと、後手番で升を180度回転して引くこと、fail lowの加点では1つ前のplyの手を指した側を基準に回転すること、成る手でも着手前の駒種で引くこと、記録から着手後の駒を導けること、および更新量を表の上限で切り詰めることを確かめる。
M1のテストは、減点の逓減で後の手の減点が前の手より大きくならないことと、更新後の値が上限の範囲に収まることを確かめる。
M2のテストは、置換表の手が不合法なら加点しないこと、置換表の値がα以下の打ち切りでは更新しないこと、および直前の手が捕獲手かnull moveならfail lowの加点をしないことを確かめる。
LMRのテストは、倍率0なら補正項が負でも減深量が0であることと、残り深さ2以下で減深量が0であることを確かめる。
完了条件は、`cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`の通過、ノード数の一致と変化の記録、および探索速度とメモリの記録である。

### フェーズ2　パラメーターファイルと時間切れの事前確認

形の変更の最後のコミットでビルドした`minase`を`data/spsa/history-forms-runner/`へ複製し、以後の再ビルドで変わらないrunner（測定を実行するプログラム）として固定して、以後のフェーズはこのrunnerで実行する。
`minase spsa params --engine commit:<形の変更> --rules engine-default`で既定のパラメーターファイルを生成し、`FmScale`の行を削除したファイルを`data/spsa/history-forms.params`とする。
開始値と範囲が形の変更の`params.rs`の表と一致すること、および`c_end`と`r_end`が既定値であることを、使い捨てのスクリプトで機械的に突き合わせる。

続いて、手引きの第3節に従い、時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても時間切れが起きないことを確かめる。
写しの開始値を`HardRemainingShare`＝50、`HardSoftRatio`＝800、`MinMoves`＝40、`IterationRatio`＝150へ書き換えた`data/spsa/history-forms-safety.params`で、13反復（104ペア）を実行する。

```console
<固定したminase> spsa \
  --run-dir data/spsa/history-forms-safety --seed 3800000000 \
  --engine commit:<形の変更> --params data/spsa/history-forms-safety.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 13 --pairs-per-iteration 8
```

`time_forfeits`（時間切れの件数）が1件でもあれば、[デバッグの手引き](../guides/debugging.md#時間切れと時間管理)の手順で原因を調べ、該当する係数の範囲を狭める案を利用者に示してから本番へ進む。
結果は測定記録`history-forms-safety`に残す。
完了条件は、時間切れとエンジン異常が0件であることと、測定記録の保存である。

### フェーズ3　SPSAによる調整

```console
<固定したminase> spsa \
  --run-dir data/spsa/history-forms --seed 3850000000 \
  --engine commit:<形の変更> --params data/spsa/history-forms.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 3750 --pairs-per-iteration 8
```

最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`history-forms`に、手引きの第6節の項目とともに残す。
最終値を丸めた整数が範囲の上限または下限になった係数を示す。
新しい16係数については、最終値が開始値からどれだけ離れたかを、`c_end`に対する比で示す。
この比は、どの形の変更が調整で動いたかを見る目安であり、採否には使わない。
完了条件は、セッションの完了と測定記録の保存である。

### フェーズ4　反映と事前対局

手引きの第7節の手順1に従い、`minase spsa apply`で最終値を表の既定値へ書き込み、`git diff`で既定値だけが変わったことを確かめてコミットする。
続いて`cargo test`（通常ビルドと調整用ビルド）を測定より先に実行し、表の既定値に依存するテストが失敗した場合は、テストの前提が新しい既定値でも成り立つようにテストを直して別のコミットにする。
テストの失敗を、末尾に`; true`を付けるようなコマンドの組み方で隠さない。
候補の調整用ビルドの宣言を`minase spsa params`で出力し、既定値の欄が`apply`の出力と一致することを確かめる。

採否測定の前に、候補が時間切れやエンジン異常を起こさないことと、時間の使い方の変化を確かめるため、STCの条件で候補と起点の32ペアの事前対局を行う。

```console
<固定したminase> match run \
  --run-dir data/matches/history-forms-smoke --seed 3900000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 elo --pairs 32
```

候補と起点の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ（反復深化で読み終えた深さ）、および探索を止めた理由の分布を集計して、測定記録`history-forms-smoke`に残す。
完了条件は、テストの通過、事前対局での時間切れとエンジン異常が0件であること、および測定記録の保存である。

### フェーズ5　採否測定

```console
<固定したminase> match run \
  --run-dir data/matches/history-forms-stc --seed 3950000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

STCの判定は[SPRTの手引き](../guides/sprt.md)の振分け規則に従う。
逐次検定が候補を有意に強いと判定した場合（判定`H1`）はLTCへ進み、改善を示せないと判定した場合（判定`H0`）は不採用とする。
3,000ペアで判定保留なら、その時点の対数尤度比の符号でLTCへ進むかを決める。
LTCは測定名の接尾辞を`-ltc`、基本シードを4000000000、持ち時間を`time=60000+200`とし、`--max-pairs`を付けない。
LTCで判定`H1`となり、エンジン異常、時間切れ、および拒否着手（審判層が合法とした相手の着手をエンジンが受け付けなかった反則）が0件なら採用し、判定`H0`なら不採用とする。
LTCの判定保留は、手引きのとおり`--resume`と`--target-pairs`で延長する。
結果は測定名ごとの測定記録に残す。
完了条件は、採否の確定と測定記録の保存である。

## 検証

- M2、M3、M5の各コミットで、benchの全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。
- M1とM4の各コミットで、benchのノード数の変化が記録されていること。
- 探索速度と最大常駐メモリの変化が記録され、continuation historyの表がワーカーあたり約70 MBであること。
- 新しい16係数が調整用ビルドの`Tune_`オプションとして宣言され、廃した3係数が宣言されず、パラメーターファイルの開始値と範囲が表と一致すること。
- 候補の`params.rs`の差分が既定値の書き換えだけであり、調整用ビルドの宣言の既定値が`apply`の出力と一致すること。
- `cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`が通過すること。
- 時間切れの事前確認、SPSA、事前対局、および採否の測定で、エンジン異常、時間切れ、および拒否着手が0件であること。最初の測定で1件でも出た場合は、測定を止めて原因を調べる。

## 完了条件

- 形の変更の検証結果、時間切れの事前確認、SPSAの結果、事前対局、および採否が、測定記録に残っている。
- 採用した場合は、候補のブランチがmasterへ統合され、依存関係の節に挙げたADRが新しいADRに置き換えられている。統合は利用者の許可を得てから行う。
- 採用した場合は、lishogiのBotのコンテナのメモリ上限が、continuation historyの増分を含めて足りることを確かめてからBotを更新する。
- 不採用の場合は、結論と測定記録だけをmasterへ残し、候補のブランチはmasterへ統合しない。
- ROADMAPの状態表が本書の結論と一致している。

## 参考資料

- [探索と時間管理の係数の関数形に関する既存実装との比較](../research/search-coefficient-forms-prior-art.md)。
- [手の順序付けの改善](archived/move-ordering.md)と、counter move historyとcontinuation historyの再診断を含む[診断の記録](../measurements/move-ordering-diag.md)。
- 根の探索ごとに初期化する表での発動率の記録（[counter move history](../measurements/strength-stage5-cmh-bench.md)、[continuation history](../measurements/strength-stage5-continuation-bench.md)）と、その表の設計を記した[棋力向上段階5](archived/strength-stage5.md)。
- Stockfish（コミット5c4cef0b）の`src/search.cpp`。根の探索ごとの減衰は341〜343行、文脈の表の設定は688〜700行、表の初期値は719〜727行、置換表の打ち切りでの更新は902〜910行、LMRのhistory補正は1371〜1381行、LMRの適用条件は1395行、最善手があるときの更新は1602〜1608行、fail lowの直前の手への加点は1611〜1634行、加点と減点は2004〜2035行、continuation historyの更新は2049〜2075行、静かな手の表の更新は2077〜2089行。`src/movepick.cpp`の静かな手の並べ替えは233〜239行。`src/history.h`の重力式は70〜76行、表の上限は128〜143行。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`。置換表の打ち切りでの更新は2735〜2748行、LMRのhistory補正は3925〜3937行、LMRの適用条件は3954行、fail lowの直前の手への加点は4311〜4339行、加点と減点は5308〜5350行、continuation historyの更新は5384〜5398行。`source/movepick.cpp`の静かな手の並べ替えは361〜368行、`source/history.h`の表の上限は206行、244行、249行。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)、[SPRTによる棋力測定の手引き](../guides/sprt.md)、および[SPSAの反復予算の調査](../research/spsa-iteration-budget.md)。
