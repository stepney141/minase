# historyの更新量の形とcontinuation historyの設計書

## 先に読む要約

本書は、手の成績を学習する表（history）の更新と使い方をチェスのStockfishの形へそろえ、現行のmasterより強くなるかを確かめる。
minaseのbutterfly history（手番、移動元、移動先ごとの静かな手の成績の表）は、β打ち切り（ある手の値が探索窓の上端βに達し、残りの手を読まずに打ち切ること）のたびに深さの2乗だけ加減点するので、1回の更新が表の上限に占める割合がStockfishの数十分の1と小さい（[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)）。
本書が変えるのは5つである。
1つ目に、加点と減点の量をStockfishの上限つき1次式に置き換え、表の上限と値の尺度をStockfishと同じにする（history bonus/malus）。
2つ目に、historyを更新する場面を、最善手がαを上げたノード、置換表（探索結果を局面ごとに保存する表）による打ち切り、fail low（全手がα以下）のノードへ至った手、および静的評価の変化へ広げる。
3つ目に、1、2、3、4、6手前の手を文脈として静かな手の成績を引くcontinuation history、歩兵と仲人の配置で引くpawn history、および根に近いplyで引くlow ply historyを新設し、手の並べ替えに使う。並べ替えには、王手になる手への加点と、価値の低い駒に狙われる升への出入りの加減点も加える。
4つ目に、late move reductions（LMR、後方の手を浅く読む減深）の補正に、現行の閾値による段差を残したまま、history値に比例する連続量（statScore）を加え、continuation historyの値が低い静かな手を読まない枝刈りを加える。
5つ目に、LMRを、先頭の手を除くすべての手と根へ広げ、根でもhistoryを更新する。
killer手や段差のように中将棋の診断と測定で決めた既存の仕組みは残す。
新しい係数の開始値はStockfishの値とし、既存の係数とまとめてSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で調整する。
調整した版を、起点のmasterと短時間条件STCおよび長時間条件LTCの2段階の逐次検定（段階ゲート）で対局させ、1つの候補として採否を決める。
完了条件は、候補の採否が測定記録とともに確定することである。

## 状態

起案。2026年10月17日に起案した。
[浅い枝刈りの深さと対象の拡張](shallow-pruning-extensions.md)の完了を待って着手し、着手は利用者の指示による。
利用者の判断を待つ点はない。
次の一手は、前の設計書の完了後に起点を確定し、フェーズ1（形の変更の実装）を始めることである。

## 目的

本書が確かめるのは、「historyの更新と使い方をStockfishの形へそろえ、係数を調整すれば、現行の形で調整した探索より強くなる」という仮説である。
現行の加点は`min(深さ² × HistoryBonusScale/100, HistoryLimit)`であり、起案時点の既定値（尺度100%、上限17408）では、深さ10の加点は上限の約0.6%にとどまる。
Stockfishの加点は深さ10で表の上限7183の約15%であり（`src/search.cpp:2004-2006, 2015`）、値が上限に近づくほど更新量を小さくする重力式（後述）による忘却が働く。
minaseでは1回の更新が小さいので、重力式による忘却はほとんど働かず、根の探索ごとの減衰（`HistoryDecay`）が忘却の役割をほぼ担っていると推測される。
また、直前の手を文脈とする表は、[手の順序付けの改善](archived/move-ordering.md)の診断で、表を対局内で持ち越し、根の探索ごとに23%へ減衰させる条件で、静かな手の順序を変えるノードがSTC相当で44.6%（1手前）と61.3%（2手前）あると分かったが、表の大きさの設計が残っていた（[診断の記録](../measurements/move-ordering-diag.md)）。
2026年10月17日の利用者の決定により、形や値は両エンジンで違えばStockfishに合わせ、Stockfishにだけある項目は含める。
同じ日の利用者の決定により、minaseにあってStockfishにない既存の仕組みのうち、中将棋の診断や測定で決めたものは残し、Stockfishの仕組みを並べて足す。
本書は、historyに関わるStockfishの仕組みを1つの候補にまとめ、係数を調整し直して測る。

## 適用範囲

本書の用語を次のように定める。

- **起点**は、前の設計書が完了した後、フェーズ1を始める時点のmasterの先頭のコミットである。既存の係数の現行値（`LmrDivisor`、`HistoryDecay`、`CaptureHistoryScale`など）は、起点の`crates/minase/src/search/alphabeta/params.rs`の表の既定値を指す。本書が例に挙げる値は起案時点の値である。
- **形の変更**は、本書の項目を実装したコミット列であり、新しい係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で`params.rs`の表の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **bench**は、固定の15局面を指定の深さで探索し、探索ノード数、評価値、最善手、および探索速度を出力する`minase dev bench`である。
- **ワーカー**は、1つの探索を並列に進める探索スレッドであり、その数はUSIオプション`Threads`で決まる。
- **butterfly history**は、`history.rs`の`HistoryTable`であり、手番側、移動元、移動先で引く静かな手の成績である。**捕獲履歴**（capture history）は、`capture_history.rs`の、動かす駒、到達升、取られる駒種で引く捕獲手の成績である。
- **重力式**は、符号つきの更新量`b`を表の上限`D`で`[−D, D]`に切り詰めてから、値`v`を`v + b − trunc(v×|b|/D)`へ置き換える更新式である。`trunc`は0の方向への切り捨てである。現行の`record_quiet_beta_cutoff`と同じ式であり、Stockfishの`StatsEntry::operator<<`（`src/history.h:70-76`）も同じ形である。値は常に`[−D, D]`に収まる。
- **Stockfishの尺度**は、表の値と更新量をStockfishと同じ数値で表すことをいう。本書は表の上限をStockfishと同じ値にするので、Stockfishの定数を換算せずに使える。評価値だけは尺度が違うので、minaseの評価値`e`を`e × 208/P`（`P`は探索用の歩兵価値、208はStockfishの歩兵価値、`src/types.h:199`）へ換算して使う。
- **静かな手の段**は、手選択器（`ordering.rs`の`MovePicker`）が置換表の手、捕獲手、killer手（同じ深さの別のノードでβ打ち切りを起こした静かな手）を返した後に、残りの静かな手を並べ替えの値の降順に返す区切りである。
- **着手の記録**は、`Searcher`がplyごとに持つ、そのplyのノードで指した手の`Move`、着手前の駒種、および捕獲手かどうかの組である。ply `p`の記録は、ply `p`の局面からply `p + 1`の局面へ進めた手を表す。着手後の駒（成る手なら成った駒）は、`Move`の成りの指定と着手前の駒種から導く。null moveはそのplyの記録を空にし、根より前のplyは記録を持たない。[correction historyの鍵の追加の設計書](correction-history-tables.md)のC3も、同じ記録を使う。
- **文脈の手**は、ply `p`のノードで手を引くときに参照する、ply `p − i`（`i`は1、2、3、4、6）の記録の手である。記録が空のplyは文脈の手を持たない。
- **ノードの手数**は、そのノードで手選択器の候補列から取り出した適格な合法手の数であり、枝刈りで飛ばした手も含む。Stockfishの`moveCount`に当たり、[浅い枝刈りの深さと対象の拡張](shallow-pruning-extensions.md)の数え方と同じである。M1の加点の式が使う「最善手より先に読んだ手の数」は、実際に読んだ手の数であり、これとは別である。
- **improving**は、`negamax.rs`の`improving`が定める判定であり、同じ手番の2手前の局面より静的評価が上がったことをいう。
- **PVノード**は窓の幅が1より大きいノード、**零窓のノード**は窓の幅が1のノードである。
- **i16**は、−32768から32767までを表す16ビットの符号つき整数である。
- **PST**と**FM**は、既定の評価関数がNNUEに置き換わる前に使っていた評価関数の部品であり、それぞれ駒と升ごとの評価値の表と、2駒の関係による評価の補正である。`FmScale`はFMの補正に掛ける倍率であり、NNUEでは使わないのでSPSAの対象から外す。
- 減深量と深さの単位は、探索の深さ1を1手と数える。

対象は次の6項目である。

- 着手の記録など、後続の項目が使う探索経路の状態の追加（M0、挙動を変えない移行）。
- 加点と減点の形と表の上限（M1）。
- historyを更新する条件の拡大（M2）。
- continuation history、pawn history、およびlow ply historyの新設と、王手と脅威の項を含む手の並べ替え（M3）。
- LMRと枝刈りでのhistoryの使用（M4）。
- LMRの対象の全手と根への拡大と、根でのhistoryの更新（M5）。

次の事項は対象外とする。

- 置換表の手の成績の表（Stockfishの`ttMoveHistory`）。Stockfishはこの表をsingular extensionの余裕値にだけ使い（`src/search.cpp:1290, 1308, 1608`）、minaseにはsingular extensionがない。
- continuation historyの王手の次元。Stockfishは表を、文脈の手を指した局面が王手を受けていたかで分け、王手を受けている局面では2手前までの表だけを更新する（`src/search.cpp:690-691, 2059-2061`）。minaseは王手を受けている局面を区別しない（王駒を利きへ置く手も合法であり、RULES.md第8条第3項から第5項）。
- killer手の表の変更。killer手はStockfishにないminaseだけの仕組みであり、2026年10月17日の利用者の決定（中将棋の診断や測定で決めた仕組みは残す）により、本書はその更新と手選択器の段を変えない。
- LMRの減深量のうちhistory以外の項（cutNode、allNode、ttPv、置換表の手が捕獲手の場合、子ノードの打ち切りの回数、`α − eval`、手数、補正量など）と、減深の結果に応じて読み直しの深さを増減する処理。これらは[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)と[correction historyの設計書](correction-history-tables.md)が扱う。本書のstatScoreの項は、それらの項と同じ式の中で引く（M4）。
- 置換表で打ち切る条件そのもの。本書は打ち切ったときの更新だけを加える。
- correction historyにcontinuationの鍵を加えること。[correction historyの設計書](correction-history-tables.md)が扱う。

## 依存関係

起点は、[浅い枝刈りの深さと対象の拡張](shallow-pruning-extensions.md)が完了した後のmasterの先頭である。
前の設計書の候補が採用されたかどうかにかかわらず、起点の表の既定値をSPSAの開始値とする。
前の設計書の候補がmasterへ統合されると`params.rs`の既定値と探索が変わるので、実装を含めて前の設計書の完了後に始める。
系列の順序は提案であり、最終的な順序は利用者が決める。

M4とM5は、LMRの減深量を1/100手単位の整数で計算し、最後に手単位へ直す形を前提にする。
この形は[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF4が導入するが、F4の採否ではなく起点の実装で判断する。
起点のLMRが1/100手単位で計算されていなければ、F4の設計判断「LMRの減深量の単位」と同じ方法で減深量の表を1/100手単位に改める変更を、探索ノード数が一致するコミットとしてフェーズ1の最初に入れる。
LMRの減深量と減深後の深さの式は、F4の式を正本とする。
本書はF4の`r₀`からstatScoreの項`q_hist`を引くことだけを加え、その位置はallNodeの拡大の前とする（M4）。
[correction historyの設計書](correction-history-tables.md)のC5の`q_corr`もallNodeの拡大の前に引き、[浅い枝刈りの深さと対象の拡張](shallow-pruning-extensions.md)のS9の詰みを探す局面での調整はallNodeの拡大の後に行う。
起点にこれらの項がなければ、該当する項は0、上限は3として読み、これらの係数は加えない。

M4の減深後の深さ（lmrDepth）へのhistoryの項は、[浅い枝刈りの深さと対象の拡張](shallow-pruning-extensions.md)が静かな手のfutility pruningにlmrDepthを導入していることを前提にする。
起点にlmrDepthがなければ、この項は加えず、continuation historyによる枝刈りだけを加える。

pawn historyは、歩兵と仲人の配置の鍵（pawn key）で引く。
この鍵は本書が導入し、[correction historyの設計書](correction-history-tables.md)のC2は本書の鍵を再利用する。

M3の王手の加点は、[浅い枝刈りの深さと対象の拡張](shallow-pruning-extensions.md)が導入する王手の判定（RULES.md第8条の定義に従い、Stockfishの`check_squares`に当たる前計算）と、同じ設計書のS5が導入する静かな手のSEEの判定器（取る駒がない手の交換列を作る判定）を再利用する。
起点にこれらがなければ、同じ設計書の定義どおりの判定と判定器だけを、挙動を変えない移行のコミットとして本書が先に入れる。
SEEの判定器の引数を符号つきの閾値`T`（`SEE < T`なら真）に改める契約の移行（[静止探索の設計書](qsearch-forms.md)）には、王手の加点に使う判定も含め、閾値`−trunc(75P/208)`を保つ。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと共用するので、同時対局数は16を上限とする。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は次のADRの決定の一部を変える。
採用した場合は、完了時に、起点の時点で有効なADR（先行の設計書が置き換えていればその後継）を確認し、新しいADRで置き換える。
部分的に変える場合は、残る決定も新しいADRへ引き継ぐ。

- [LMRの式のADR](../adr/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)（2026年9月7日）は、history補正を閾値による±1の段差とし、連続量の案を比例係数と切り詰めの2つの自由度を絞る診断が要るとして棄却した。対象は置換表の手、捕獲手、killer手を除く非捕獲手とし、根では行わないとした。M4は段差を残し、Stockfishの連続量を並べて加える。比例係数はStockfishの値から始めてSPSAで調整する。M5は対象を先頭の手を除く全手と根へ広げる。このADRは評価関数がPSTだった時期の測定に基づき、上限3と段差を「STCの到達深さが5〜6だった時期の判断」として見直しの余地を留保している。同じADRの上限は[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF4も変える。
- [静かな手の順序のADR](../adr/order-quiets-with-malus-and-pst-tiebreak.md)（2026年10月10日）は、加点と減点を別の尺度で深さの2乗に比例させ、重力式で更新すると決めた。M1は加点と減点をStockfishの上限つき1次式に置き換え、M2は更新する場面を増やし、M3は並べ替えの値に新しい表を加える。重力式と同点破りは変えない。このADRはPSTを既定の評価関数としていた時期の測定に基づき、同点破りの効果をNNUEで測り直していないことと、尺度がSPSAの調整を待つ既定値であることを記している。
- [historyの持ち越しのADR](../adr/carry-history-tables-within-a-game.md)（2026年10月10日）は、butterfly historyと捕獲履歴を対局内で減衰させて持ち越し、killer表と補正表は持ち越さないと決めた。M3はcontinuation historyとpawn historyを加え、新規対局などの時点で初期化して対局内で減衰させずに持ち越し、low ply historyは根の探索ごとに初期化する。butterfly historyと捕獲履歴の減衰の仕組みは変えず、`HistoryDecay`の開始値をStockfishの値に置き直す。このADRはPSTとFMを既定の評価関数としていた時期の測定に基づき、continuation historyの設計を別の設計書に委ねている。
- [butterfly表だけのADR](../adr/keep-only-the-butterfly-history-table.md)（2026年9月7日）は、静かな手のhistoryをbutterfly表だけで構成すると決め、直前の手と組み合わせる表は対象外とした。M3はcontinuation history、pawn history、およびlow ply historyを加える。駒種と到達升で引く表（piece-to history）は、Stockfishにもないので加えない。このADRはPSTの時期の測定に基づき、4次元の表が疎だという判断は表を根の探索ごとに初期化していた時期のものなので、再提案するときは現行の表の寿命で測り直す必要があると留保している。
- [捕獲履歴のADR](../adr/bound-capture-history-and-use-it-in-qsearch.md)（2026年10月10日）は、捕獲履歴の換算値を歩兵価値の定数倍に抑えて並べ替えに使い、重力式で更新すると決めた。M1は捕獲履歴の更新量と上限をStockfishの形にし、M4は捕獲手のLMRにも使う。並べ替えへの換算の形は変えず、尺度`CaptureHistoryScale`の開始値をStockfishの換算値に置き直す。このADRはPSTの時期の測定に基づき、LMRへの使用については留保していない。

[late move pruningの数え方のADR](../adr/count-only-quiet-moves-for-late-move-pruning.md)、[killer手を損な先頭捕獲より先に返すADR](../adr/try-killers-before-a-losing-first-capture.md)、および[根の手の並べ替えのADR](../adr/order-root-moves-by-previous-subtree-nodes.md)の決定は変えない。

## 設計判断

| 項目 | 決定 |
|---|---|
| 合わせる先 | 2026年10月17日の利用者の決定により、形や値がStockfishとYaneuraOuで違う場合はStockfishに合わせ、YaneuraOuにだけある項目は入れず、Stockfishにだけある項目は入れる。本書の範囲では、YaneuraOuもpawn history、low ply history、continuation historyによる枝刈り、および静的評価の変化による更新を持つ（`yaneuraou-search.cpp:1540, 3126-3132, 3646-3661`）。同じ日の利用者の決定により、minaseにあってStockfishにない既存の仕組みのうち、中将棋の診断や測定で決めたもの（killer手、LMRの段差、静かな手の同点破り、捕獲履歴の静止探索での使用、根の手の並べ替え、および捕獲履歴の根ごとの減衰）は残し、Stockfishの仕組みを並べて足し、残したものもSPSAで調整する。 |
| 候補の束ね方 | 全項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回の段階ゲートで採否を決める。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。項目ごとにSPSAと段階ゲートを繰り返す案は、1項目あたり約10〜47時間を要するので採らない。代償として、項目ごとの寄与は測らない。不採用のときに一部の項目だけを選び直して再測定しない。 |
| 開始値と検証 | 2026年10月17日の利用者の決定により、新しい係数の開始値はStockfishの値とする。既存の係数も、Stockfishに同じ形の値があれば換算値に置き直す。本書では`HistoryDecay`（23から71）と`CaptureHistoryScale`（49から734）が当たる。`LmrHistoryThreshold`はStockfishに対応する値がないので、表の上限を7183へ改める尺度の変更に合わせて、`round(起点のLmrHistoryThreshold × 7183 / 起点のHistoryLimit)`を開始値にする（起案時点の146と17408なら60）。各項目のコミットでbenchのノード数の変化を記録し、挙動を変えない移行（M0と単位の変更）だけをノード数の一致で検証する。 |
| 尺度と表の上限 | butterfly historyと捕獲履歴の上限を、係数`HistoryLimit`と`CaptureHistoryLimit`から、Stockfishと同じ定数7183と10692に改め（`src/history.h:128, 135`）、両係数を廃する。新しい表の上限もStockfishと同じにする。表の上限がStockfishと同じなら、Stockfishの更新量と重みを換算せずに使え、表の間の値の比もStockfishと同じになる。上限を係数として残すと、更新量を上限に比例させて換算する限り、上限は値の分解能を変えるだけで探索の挙動に効かなくなる。 |
| 係数にする値と定数にする値 | 2026年10月17日の利用者の決定により、SPSAで調整するのは各仕組みの強さを決める値（傾き、切片、上限の手前で効く量、重み、倍率）だけとし、上下限、平均の重み、更新の内部の定数はStockfishの値で固定する。本書の新しい係数は15個であり、固定する70個の定数は後述の「定数」の表に行番号つきでまとめる。 |
| 加点と減点の形（history bonus/malus） | 現行の深さの2乗の項（`HistoryBonusScale`、`HistoryMalusScale`）を廃し、Stockfishの`min(傾き×d − 切片, 上限)`に置き換える（`src/search.cpp:2004-2006`）。加点には、最善手が置換表の手なら定数を、直前の手のstatScore（後述）の1/28を足し、零窓のノードでは読んだ手の数に比例して増やす（同2007-2011）。静かな最善手には加点の899/1024、先に読んだ静かな手には減点の1159/1024を1手ごとに`HistoryMalusDecay`/1024倍しながら与え（同2013-2023）、捕獲の最善手には加点の1427/1024、先に読んだ捕獲手には減点の1489/1024を与える（同2024-2041）。 |
| 減点の逓減（malus decay） | 先に読んだ静かな手ほど大きく減点する。Stockfishは、読んだ順に、減点を1手ごとに921/1024倍してから与える（`src/search.cpp:2018-2022`）。係数の上限を1024にするのは、1024を超えると後の手の減点が大きくなり、重力式の切り詰めに頼ることになるからである。 |
| 更新の条件の拡大 | Stockfishと同じく、最善手が探索開始時のαを上げたノードで更新する（`src/search.cpp:1602-1608`）。β打ち切りに限っていた現行より更新が増える。killer手の更新はminaseだけの仕組みなので、現行どおりβ打ち切りに限る。加えて、置換表で打ち切ったときの静かな置換表の手への加点と直前の手への減点（同902-910）、fail lowのノードでの直前の静かな手への加点と直前の捕獲手への捕獲履歴の加点（同1611-1642）、前の最善手が反駁されたときの直前の手への追加の減点（同2031-2035）、および直前の手の前後の静的評価の変化による更新（同1002-1012）を加える。 |
| 置換表の手の確認 | 置換表で打ち切ったときの加点は、置換表の手が現局面で合法な静かな手であることを`MoveGenerator::is_legal_move`で確かめてから行う。置換表の手はハッシュの衝突で別の局面のものである場合があり、continuation historyを引くには移動元の駒種が要るからである。Stockfishは合法性を確かめずに更新するが、チェスの表は移動元と到達升だけで引けるので、別の局面の手でも表の外を読まない。 |
| 静的評価を求めるノード | 静的評価の変化による更新は、直前の手のノードと現在のノードの補正後の静的評価を使う（`src/search.cpp:845-875, 1002-1012`）。[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF2がノードごとに1回求めて保持する補正後の評価を再利用し、historyの更新のために補正表を読み直さない。起点でそのノードの補正後の評価がまだ求められていなければ、F2と同じ規則で求めて保持する。静的評価の計算は探索ノード数を変えず、費用は探索速度の低下として測る。 |
| continuation historyの表の形 | 2026年10月17日の利用者の決定により、表はStockfishと同じく1組の共有の表とし、何手前の文脈も同じ表の中の区画を引く（Stockfish `src/history.h:198-199`、`src/search.cpp:690-691, 1140-1142`）。手は180度の回転で引き、表を引く手を指す側が後手なら、文脈の手と現在の手の升をともに回転する。中将棋は初期配置と規則が180度の回転と手番の入れ替えについて対称なので、先手の局面と回転した後手の局面の統計を同じ要素に集められる。添字は「文脈の手が捕獲手か（2）、文脈の手を指したのが表を引く手と同じ側か（2）、文脈の手の着手前の駒種（29）、文脈の手の到達升（144）、現在の手の着手前の駒種（29）、現在の手の到達升（144）」とし、要素はi16とする。29は成駒を含む駒種の数（`PIECE_KIND_COUNT`）、144は盤の升の数である。同じ側かどうかの次元は、Stockfishの添字の駒が手番の色を含むことに当たる。捕獲手かどうかの次元はStockfishに合わせて持つ。王手の次元は対象外の節のとおり持たない。 |
| continuation historyの大きさと共有 | 表は69,755,904要素で約140 MBであり、Stockfishと同じく探索チームの全ワーカーで共有する（`src/history.h:201-212`）。要素は不可分の読み書き（`AtomicI16`の`Relaxed`順序）で扱い、Stockfishも同じく競合を許す（同138行の`AtomicStats`）。共有するのでメモリはワーカー数によらない。捕獲手かどうかの次元を持たなければ約70 MBになる。表は新規対局、受理した規則と置換表の容量`USI_Hash`の変更、およびワーカー数の変更の時点で初期値にし、対局内で減衰させずに持ち越す。Stockfishも減衰させるのはbutterfly historyだけである（`src/search.cpp:341-343`）。効果の根拠とした診断は、1手前と2手前の2表を根の探索ごとに23%へ減衰させる条件で測ったものであり、本書の表の形と寿命での発動率は測っていない。 |
| pawn historyの形 | Stockfishと同じく、歩兵の配置の鍵で引くエントリごとに、動かす駒と到達升の表を持つ（`src/history.h:146, 220-225`）。中将棋の鍵は、規則書が「価値ある駒」から除く2種（RULES.md第3条第11項）である歩兵と仲人の配置とし、[correction historyの設計書](correction-history-tables.md)のC2の鍵と同じ定義にする。鍵は升と駒ごとの64ビットの乱数の排他的論理和であり、`material_keys`と同じくplyごとの配列で差分更新する。鍵は盤の絶対の配置なので、この表は回転せず、駒は手番を含む58種で引く。1エントリは駒58種×升144のi16で約16.7 KBになり、チェスの約8倍なので、エントリの数をStockfishのワーカーあたり8192から1024に減らし、ワーカー1つ分の大きさをStockfishと同程度（約17 MB）にする。エントリの数は1024にワーカー数以上の最小の2のべきを掛けた数とし、表は全ワーカーで共有する。Stockfishも表の大きさをワーカー数に比例させて共有する。 |
| low ply historyの形 | Stockfishと同じく、根からのplyが5未満のノードで、plyと移動元と到達升で引く表を持つ（`src/history.h:42, 132`）。根の探索の開始時に全要素を102にする（`src/search.cpp:339`）。ワーカーごとに持ち、約200 KBである。 |
| 並べ替えの値 | 静かな手の段の値を、Stockfishと同じく`2×butterfly + 2×pawn + (1、2、3、4、6手前の文脈のcontinuation historyの和) + 8×low ply/(1 + ply)`とする（`src/movepick.cpp:233-251`）。同点破り（線形経路の差分、生成順）は変えない。根の並べ替えは前回の反復の部分木ノード数で決めるので変わらない。 |
| LMRのhistory補正（statScore） | 閾値による±1手の段差（`LmrHistoryThreshold`）は、[棋力向上段階5](archived/strength-stage5.md)の診断と測定で決めた仕組みなので、2026年10月17日の利用者の決定により残す。そこへStockfishのstatScoreに比例する連続量を並べて加える（`src/search.cpp:1371-1381`）。statScoreは、静かな手では`(2252×butterfly + 1126×1手前 + 1093×2手前)/1024`、捕獲手では`873×取る駒の価値/128 + 捕獲履歴`である。取る駒の価値はStockfishの尺度へ換算し、獅子の2段階移動で2枚を取る手は合計する。段差はbutterfly historyの値で判定し、捕獲手には使わない。表の値がStockfishの水準に上がると段差はほとんどの手で働くようになるが、閾値を調整の対象に残すので、SPSAが閾値を上げて段差を弱めることもできる。 |
| 並べ替えの王手と脅威の項 | Stockfishと同じく、静かな手の並べ替えの値に、王手になる手でSEE（駒の取り合いの損得の見積り）が`−trunc(75P/208)`（Stockfishの尺度で−75）以上なら`OrderingCheckBonus`を足し（`src/movepick.cpp:241-242`）、動かす駒より価値の低い相手の駒に狙われている升から出る手に加点、そうした升へ入る手に減点する（同244-247）。王手の判定は依存関係の節の前計算を使う。Stockfishは価値の低い駒の集合を駒の格（歩兵、小駒、飛車、女王）で作り、キングには0を使う（同201-209）。中将棋は駒種が29あるので、相手の駒のうち駒価値（`Nnue::piece_value`）が動かす駒より小さいものの利きの和集合を価値の低い駒の脅威とし、王駒には0を使う。この集合はノードで静かな手の段を開くときに、相手の駒を価値の順に並べた累積の利きとして1回だけ求め、費用は探索速度の低下として測る。YaneuraOuは王手の加点だけを持ち、脅威の項を持たない（`movepick.cpp:371-388`）。 |
| 評価値の単位を持つ係数 | 2026年10月17日の利用者の決定により、評価値の単位を持つ係数の範囲は、Stockfishの換算値（歩兵価値208で割った値）、YaneuraOuの換算値（歩兵価値90で割った値）、およびminaseの現行の値をすべて含むように取る。本書で当たるのは2つである。`CaptureHistoryScale`（捕獲履歴が上限のときの並べ替えへの加算、歩兵価値に対する百分率）は、Stockfishが10692/7/208で734、YaneuraOuが10692/7/90で1697、現行が49であり、開始値734は現行の約15倍である（Stockfish `src/movepick.cpp:225-226`、YaneuraOu `movepick.cpp:340-341`）。`OrderingThreatWeight`（動かす駒の価値が歩兵1枚分のときの脅威の項の大きさ）は、Stockfishが20×208で4160、YaneuraOuは項を持たず、現行は0である。 |
| continuation historyによる枝刈り | Stockfishと同じく、静かな手のうち、1手前と2手前のcontinuation historyとpawn historyの和が`−ContinuationPruning × d`を下回る手を読まない（`src/search.cpp:1229-1235`）。適用条件は起点の静かな手のfutility pruningと同じとする。Stockfishは前回の反復の主変化を辿るPVノードを除く（同1227行の`followPV`）。この状態は[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF5が導入するので、本書はそれを再利用する。起点にない場合だけ、F5と同じ定義で導入する。 |
| LMRの対象 | Stockfishと同じく、先頭の手を除くすべての手（捕獲手、killer手、成る手、および先頭以外に来た置換表の手）を減深の対象とし、根も対象にする（`src/search.cpp:1395`）。根の`search_root`は、F4の式で減深後の深さを求めて`search_move`へ渡す。減深した探索の値がαを上回った後の深さの±1手の調整と再探索、負の減深量による延長、およびPVノードの1手の加算は、F4の規則のまま保つ。`search_move`は再探索の後の値を返すので、途中結果の採用（`RootResults`）には再探索の後の値だけが記録される。 |
| 根での更新 | Stockfishは根も同じ探索関数で読むので、根でも最善手がαを上げればhistoryを更新する。本書は`search_root`にも同じ更新を加える。根には直前の手がないので、直前の手を使う更新は行わない。 |
| 重みが0のときの処理 | 係数が0でも、表の参照と更新、および置換表の手の合法性の確認を省かない。省く分岐を置くと、フェーズ1で測る探索速度に、SPSAで係数が動いた後の費用が現れないからである。 |
| SPSAの対象 | 起点の表のすべての係数（`FmScale`を除く）から、廃する4係数（`HistoryBonusScale`、`HistoryMalusScale`、`HistoryLimit`、`CaptureHistoryLimit`）を除き、新しい15係数を加える。新しい係数だけを調整する案は、既存の係数との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の係数を含むので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 既定の設定を流用する。3,750反復、1反復8ペア（計30,000ペア）、持ち時間10秒＋0.1秒、同時対局数16、`c_end`（最後の反復の摂動幅）は範囲の1/6、`r_end`（最後の反復の学習率）は0.002である（[SPSAの手引き](../guides/spsa.md)）。系列の先行の設計書をすべて採用した場合、係数の数は本書の時点で81前後になる（先行の設計書の改訂で変わり得るので、起点の表で数え直す）。30,000ペアでこの数を十分に調整できるかは未検証であり、不採用はこの予算と設定で改善を示さなかったことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。最終値を最適値とは呼ばない。総ペア数は、[利得と規模のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)が定める標準の30,000ペアである。 |
| 新しい係数の範囲 | 2026年10月17日の利用者の決定により、範囲は開始値を含み、両エンジンの値と現行の値の両方を含むように取る。現行の値は、新しい仕組みがない状態（追加の更新の強さ、LMRの補正の強さ、王手と脅威の項は0、減点の逓減は1024、上限は表の上限）に当たる値とする。continuation historyによる枝刈りの閾値だけは、枝刈りがない現行の状態が閾値の無限大に当たるので、範囲に含められない。 |

メモリは、continuation historyの約140 MBと、全ワーカーで共有するpawn historyの約17 MB×（ワーカー数以上の最小の2のべき）が増える。
lishogiのBotは`Threads`を4としているので（`minase-lishogi-bot/config.yml`）、1局あたり約210 MB増え、同時対局数を掛けた分だけコンテナのメモリ上限の見直しが要る。
この確認は完了条件に含める。

## 変更の内容

以下の式で、`d`はノードの残り深さ（置換表で打ち切った場合は要求深さ、それ以外はIIRの後の深さ）、`[条件]`は条件が成り立てば1、成り立たなければ0を表す。
IIR（internal iterative reduction）は、置換表に記録手がない残り深さ3以上のノードを1だけ浅く読む現行の規則であり、適用しないノードではIIRの後の深さは要求深さに等しい。
表の値と更新量はStockfishの尺度で表す。
重力式の除算は0の方向へ切り捨て、それ以外の除算はStockfishと同じく整数の除算（0の方向への切り捨て）とする。
LMRの1/100手単位の減深量を手単位へ直す除算も、[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF4と同じく0の方向へ丸める。

### M0　探索経路の状態の追加（挙動を変えない移行）

`Searcher`に、plyごとの着手の記録、歩兵と仲人の配置の鍵、F2が保持する補正後の静的評価への参照（起点で保持していなければ保持する欄）、statScore、ノードの手数、および置換表に局面があったかどうかを加える。
statScoreはノードの開始時に0とし、手を読む直前にその手の値を入れる。
これらは記録するだけで探索に使わないので、探索ノード数は変わらない。

### M1　加点と減点の形と表の上限（history bonus/malus）

butterfly historyの上限を7183、捕獲履歴の上限を10692の定数とする。
最善手`best`があるノードで、加点`b`と減点`m`を次のように求める。

```math
b = \min(\mathrm{HistoryBonusSlope} \times d - \mathrm{HistoryBonusBase},\ \mathrm{HistoryBonusCap}) + \mathrm{HistoryTtMoveBonus} \times [\mathit{best} = \text{置換表の手}] + \left\lfloor \frac{s_{p-1}}{28} \right\rfloor
```

```math
m = \min(\mathrm{HistoryMalusSlope} \times d - \mathrm{HistoryMalusBase},\ \mathrm{HistoryMalusCap})
```

ここで`s_{p−1}`は、このノードへ至った手のstatScoreである。
零窓のノードでは、`b`に`b × (最善手より先に読んだ静かな手の数 + 同じく捕獲手の数)/256`を足す。
最善手が静かな手なら、M3の「静かな手の表の更新」で最善手に`b × 899/1024`を与える。
続いて`m′ = m × 1159/1024`とし、先に読んだ静かな手を読んだ順に、`m′ ← m′ × HistoryMalusDecay/1024`としてから`−m′`を与える。
最善手が捕獲手なら、捕獲履歴の最善手の要素に`b × 1427/1024`を与える。
最善手の種類によらず、先に読んだ捕獲手の捕獲履歴に`−m × 1489/1024`を与える。
killer表は、β打ち切りのときに現行どおり更新する。
新規対局などで表を初期化するときは、Stockfishと同じく、butterfly historyを−5、捕獲履歴を−742にする（`src/search.cpp:714-715`）。

### M2　historyを更新する条件の拡大

M1の更新を、β打ち切りのノードだけでなく、最善手の値が探索開始時のαを上回ったすべてのノードで行う。
次の場面でも更新する。

置換表で打ち切ったときは、置換表の値がβ以上で、置換表の手が現局面で合法な静かな手なら、その手に`HistoryTtCutoffBonus × d`を静かな手の表の更新で与える。
同じ条件の打ち切りで、このノードへ至った手が静かな手で、その手のノードの手数が5未満なら、その手のcontinuation historyに−2210を与える（`src/search.cpp:902-910`）。

前の最善手が反駁されたときは、M1の更新を行うノードで、このノードへ至った手が静かな手であり、その手のノードの手数が「1 + 置換表に局面があったか」に等しければ、その手のcontinuation historyに`−m × 713/1024`を与える（`src/search.cpp:2031-2035`）。

fail lowのとき、すなわち最善手の値が探索開始時のα以下だったときは、このノードへ至った手に次の加点をする（`src/search.cpp:1611-1642`）。
その手が静かな手か捕獲手かで、次のどちらか一方を行う。
その手が静かな手なら、`e`をこのノードの静的評価、`e_{p−1}`をその手のノードの静的評価、`s_{p−1}`をその手のstatScore、`n_{p−1}`をその手のノードの手数として、評価値をStockfishの尺度に換算し、次の量を求める。

```math
k = \max\left(0,\ -241 - \left\lfloor \frac{s_{p-1}}{98} \right\rfloor + \min(59d, 420) + 186[n_{p-1} > 9] + 142[v \le e - 106] + 159[v \le -e_{p-1} - 68]\right)
```

```math
B = \min(150d - 85, 1337) \times k
```

ここで`v`はこのノードの最善値である。
その手のcontinuation historyに`B × 263/16384`、その手を指した側のbutterfly historyに`B × HistoryPriorBonus/32768`を与える。
その手が歩兵の手でも成る手でもなければ、その手のpawn historyに`B × 324/8192`を与える。
その手が捕獲手なら、上の加点は行わず、捕獲履歴のその手の要素に892を与える。

静的評価の変化による更新は、このノードへ至った手が静かな手なら、ノードの開始時に行う（`src/search.cpp:1002-1012`）。

```math
\delta = \mathrm{clamp}\left(-(e_{p-1} + e),\ -189,\ 194\right) + 60
```

`e`と`e_{p−1}`は、Stockfishと同じく補正後の静的評価（correction historyの補正を足した値）をStockfishの尺度に換算したものである。
その手を指した側のbutterfly historyに`δ × HistoryEvalDiff`を与え、置換表に局面がなく、その手が歩兵の手でも成る手でもなければ、pawn historyに`δ × 13`を与える。

### M3　continuation history、pawn history、low ply historyと並べ替え

continuation historyの表`C`を探索チームで共有して持つ。
ply `p`のノードで手`x`を引くとき、`x`を指す側を基準に、文脈の手`y = ply p − i`の記録から次の要素を引く。

```math
C\big[\,[y\ \text{が捕獲手}]\,\big]\big[\,[i\ \text{が偶数}]\,\big][k_y][\rho(t_y)][k_x][\rho(t_x)]
```

ここで`k`と`t`は着手前の駒種と到達升、`ρ`は`x`を指す側が後手なら升の番号`s`を`143 − s`へ回転し、先手ならそのまま返す写像である。
`i`が偶数の文脈の手は`x`と同じ側が指した手である。
文脈の手を持たない`i`の項は、参照も更新もしない。
fail lowの加点のように直前のplyの手の値を更新する場合は、その手を`x`とし、文脈の手はその手のplyから数える。

静かな手の表の更新は、手`x`に量`u`を与えるときに次を行う（`src/search.cpp:2077-2089, 2049-2073`）。
butterfly historyの`x`の要素に`u`を与える。
根からのplyが5未満なら、low ply historyの要素に`u × 712/1024`を与える。
continuation historyには、`u′ = u × 750/1024`として、`q = 0`から始めて`i`を1、2、3、4、6の順にたどり、文脈の手があれば、その要素が正の値なら`q`を1増やしてから`u′ × w_i × M_q/65536 + 73[i = 1]`を与える。
`w_i`は`(520, 390, 145, 251, 209)`、`M_q`は`(94, 103, 110, 106, 119, 126)`の`q`番目（0から数える）である。
pawn historyの要素には、`u > −4`なら`u × 1104/1024`、`u ≤ −4`なら`u × 459/1024`を与える。
各表の更新は、その表の上限で重力式を使う。

pawn historyは、歩兵と仲人の配置の鍵の下位ビットでエントリを選び、手番を含む駒と到達升で引く。
新規対局などの初期化では、continuation historyを−586、pawn historyを−1338にする（`src/search.cpp:717-727`）。
low ply historyは根の探索の開始時に102にする。

静かな手の段の並べ替えの値は次のとおりとする。

```math
V(x) = 2h_{\mathrm{bf}}(x) + 2h_{\mathrm{pawn}}(x) + \sum_{i \in \{1,2,3,4,6\}} c_i(x) + [p < 5]\left\lfloor \frac{8 h_{\mathrm{low}}(p, x)}{1 + p} \right\rfloor + \mathrm{OrderingCheckBonus} \times g(x) + \left\lfloor \frac{v_x \times \mathrm{OrderingThreatWeight}}{P} \right\rfloor \times (a_{\mathrm{from}}(x) - a_{\mathrm{to}}(x))
```

`c_i`は文脈の手を持つ`i`だけを足す。
`g(x)`は、`x`が王手になる手で、SEEが`−trunc(75P/208)`以上なら1、それ以外は0である。
`v_x`は動かす駒（着手前の駒）の駒価値、`P`は探索用の歩兵価値である。
`a_from(x)`と`a_to(x)`は、移動元と到達升が、動かす駒より駒価値の小さい相手の駒の利きにあれば1、なければ0であり、動かす駒が王駒なら0とする。

### M4　LMRと枝刈りでのhistoryの使用（statScore、continuation history based pruning）

手を読む直前に、その手のstatScore`s`を求める。
静かな手では`s = (2252 h_bf + 1126 c₁ + 1093 c₂)/1024`、捕獲手では`s = 873 V/128 + h_cap`とする。
`V`はStockfishの尺度に換算した取る駒の価値の合計、`h_cap`は捕獲履歴の値である。

LMRの減深量と減深後の深さは、[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)のF4の式を正本とし、本書はF4の`r₀`にstatScoreの項を加えるだけである。

```math
q_{\mathrm{hist}} = \mathrm{trunc}\left(\frac{s \times \mathrm{LmrHistoryScale} \times 100}{4096 \times 1024}\right), \qquad r_0' = r_0^{\mathrm{F4}} - q_{\mathrm{corr}} - q_{\mathrm{hist}}
```

ここで`r₀^F4`はF4の`r₀`の全項、`q_corr`は後続の[correction historyの設計書](correction-history-tables.md)のC5が加える補正項であり、本書の実装の時点ではこの項はなく、`r₀′ = r₀^F4 − q_hist`である。
各項は個別に0方向へ丸めてから足す。
`r₀′`をF4の`r₀`の代わりに使い、allNodeの拡大、S9の詰みを探す局面での調整、0方向への丸めによる手単位の`R`、および減深後の深さ`d′`（負の減深量による延長とPVノードの1手の加算を含む）は、F4とS9の規則のまま求める。
`q_hist`をallNodeの拡大の前に引くのは、Stockfishと同じ順序である（`src/search.cpp:1381, 1386-1388`）。
F4の段差`h`は、M5で対象に加わる手のうち静かな手（killer手と静かな置換表の手を含む）ではbutterfly historyの値で判定し、捕獲手では0とする。

静かな手の枝刈りでは、起点の静かな手のfutility pruningと同じ適用条件の下で、`c₁ + c₂ + h_pawn < −ContinuationPruning × d`の手を読まない。
文脈の手を持たない`c_i`は0として足す。
起点の静かな手のfutility pruningがlmrDepthを使っていれば、Stockfishと同じく、続いて`H = c₁ + c₂ + h_pawn + 69 h_bf/32`を求め、lmrDepthに`H / D(d)`を足す。
`D(d) = 3000 + 7 × (min(d, 16) − 8)²`である（`src/search.cpp:55-58, 1236-1240`）。

### M5　LMRの対象の拡大と根での更新

M4の減深量を、置換表の手、捕獲手、およびkiller手を除く静かな手に限らず、先頭の手を除くすべての手に適用する。
根の`search_root`でも、先頭の手を除く各手の減深量を求めて`search_move`へ渡す。
根はPVノードとして扱う。
根でも、最善手の値が探索開始時のαを上回ればM1の更新を行う。
根には直前の手がないので、M2の直前の手を使う更新は行わない。

### 新しい係数

廃する係数は`HistoryBonusScale`、`HistoryMalusScale`、`HistoryLimit`、および`CaptureHistoryLimit`である。
`HistoryLimit`と`CaptureHistoryLimit`は、Stockfishと同じ定数7183と10692になる。
開始値はStockfishの値であり、表の上限をStockfishと同じにしたので換算はない。
YaneuraOuの値は範囲の根拠として示す。

| 係数 | 開始値 | 範囲 | 範囲の根拠 |
|---|---|---|---|
| `HistoryBonusSlope`（加点の深さ1あたりの増分） | 133 | 0〜400 | Stockfishは133（`src/search.cpp:2005`）、YaneuraOuは128（`yaneuraou-search.cpp:5309`）。現行には1次の項がないので0を含める |
| `HistoryBonusBase`（加点の切片） | 81 | −100〜300 | Stockfishは81、YaneuraOuは77。現行は0 |
| `HistoryBonusCap`（加点の上限） | 1487 | 200〜7183 | Stockfishは1487、YaneuraOuは1529。現行は表の上限で切り詰めるだけなので7183に当たる |
| `HistoryMalusSlope`（減点の深さ1あたりの増分） | 968 | 0〜2500 | Stockfishは968（`src/search.cpp:2006`）、YaneuraOuは882（`yaneuraou-search.cpp:5310`）。現行は0 |
| `HistoryMalusBase`（減点の切片） | 235 | −200〜800 | Stockfishは235、YaneuraOuは204。現行は0 |
| `HistoryMalusCap`（減点の上限） | 2244 | 200〜7183 | Stockfishは2244、YaneuraOuは2122。現行は7183に当たる |
| `HistoryMalusDecay`（先に読んだ静かな手1つごとの減点の倍率、1/1024） | 921 | 700〜1024 | Stockfishは921（`src/search.cpp:2021`）、YaneuraOuは977（`yaneuraou-search.cpp:5327`）。現行は逓減しない1024 |
| `HistoryTtCutoffBonus`（置換表の打ち切りでの加点の深さ1あたりの量） | 131 | 0〜300 | Stockfishは131（`src/search.cpp:906`）、YaneuraOuは`min(130d − 71, 1043)`の傾き130（`yaneuraou-search.cpp:2739-2740`）。現行は0 |
| `HistoryTtMoveBonus`（最善手が置換表の手のときの加点の上乗せ） | 364 | 0〜1000 | Stockfishは364（`src/search.cpp:2005`）、YaneuraOuは353。現行は0 |
| `HistoryPriorBonus`（fail lowの直前の手へのbutterfly historyの加点、1/32768） | 215 | 0〜600 | Stockfishは215（`src/search.cpp:1630`）、YaneuraOuは235（`yaneuraou-search.cpp:4336`）。現行は0 |
| `HistoryEvalDiff`（静的評価の変化によるbutterfly historyの更新の倍率） | 11 | 0〜40 | Stockfishは11（`src/search.cpp:1008`）、YaneuraOuは10（`yaneuraou-search.cpp:3129`）。現行は0 |
| `ContinuationPruning`（continuation historyによる枝刈りの深さ1あたりの閾値） | 4136 | 1000〜12000 | Stockfishは4136（`src/search.cpp:1234`）、YaneuraOuは4097（`yaneuraou-search.cpp:3653`）。枝刈りがない現行は閾値の無限大に当たり、範囲に含められない |
| `LmrHistoryScale`（statScoreによる減深の減少、1/4096） | 439 | 0〜1000 | Stockfishは439（`src/search.cpp:1381`）、YaneuraOuは428（`yaneuraou-search.cpp:3937`）。現行の連続量はないので0を含める |
| `OrderingCheckBonus`（王手になる手への並べ替えの加点） | 16384 | 0〜32768 | Stockfishは16384（`src/movepick.cpp:242`）、YaneuraOuも16384（`movepick.cpp:371`）。現行は0 |
| `OrderingThreatWeight`（動かす駒の価値が歩兵1枚分のときの脅威の項、並べ替えの値） | 4160 | 0〜12000 | Stockfishは20×208で4160（`src/movepick.cpp:246-247`）。YaneuraOuは項を持たず、現行は0 |

既存の係数のうち、次の3つは開始値または範囲を置き直す。

| 係数 | 起点の値（起案時点） | 開始値 | 範囲 | 根拠 |
|---|---|---|---|---|
| `HistoryDecay`（根の探索ごとに残すbutterfly historyの割合、百分率） | 23 | 71 | 0〜100 | Stockfishは729/1024で71（`src/search.cpp:343`）。YaneuraOuは減衰させず100に当たる。範囲は起点のまま |
| `CaptureHistoryScale`（捕獲履歴が上限のときの並べ替えへの加算、歩兵価値に対する百分率） | 49 | 734 | 0〜2000 | Stockfishは734、YaneuraOuは1697、現行は49（設計判断の表の「評価値の単位を持つ係数」）。開始値は現行の約15倍である |
| `LmrHistoryThreshold`（段差の閾値、Stockfishの尺度の履歴点） | 146 | 60 | 0〜2000 | Stockfishに対応する値はない。表の上限を7183へ改めるので、`round(起点の値 × 7183 / 起点のHistoryLimit)`から始める（起案時点の146と17408なら60）。表の値の水準が上がるので、範囲の上限を広げる |

### 定数

次の70個の値は、Stockfish（コミット5c4cef0b）の値を定数として写す。
評価値の閾値は、Stockfishの尺度へ換算した評価値と比べる。

| 用途 | 値 | Stockfishの行 |
|---|---|---|
| 表の上限 | butterfly history 7183、捕獲履歴10692、continuation history 30000、pawn history 8192、low ply history 7183 | `src/history.h:128-146` |
| 初期値 | butterfly history −5、捕獲履歴−742、continuation history −586、pawn history −1338、low ply history 102（根の探索ごと） | `src/search.cpp:339, 714-727` |
| 加点の追加項 | 直前の手のstatScoreの1/28、零窓のノードで読んだ手の数の1/256 | `src/search.cpp:2005-2011` |
| 最善手と先に読んだ手への比 | 静かな最善手899/1024、静かな手の減点1159/1024、捕獲の最善手1427/1024、捕獲手の減点1489/1024 | `src/search.cpp:2015-2041` |
| 直前の手への減点 | 置換表の打ち切りで−2210（直前の手のノードの手数が5未満）、反駁で`−m × 713/1024` | `src/search.cpp:909-910, 2033-2035` |
| fail lowの加点 | M2の式の定数、continuation history 263/16384、pawn history 324/8192、捕獲履歴892 | `src/search.cpp:1614-1642` |
| 静的評価の変化 | 切り詰め−189と194、加算60、pawn historyの倍率13 | `src/search.cpp:1006-1011` |
| 静かな手の表の更新 | low ply history 712/1024、continuation history 750/1024、pawn history 1104/1024（−4以下の更新では459/1024） | `src/search.cpp:2083-2089` |
| continuation historyの更新 | 文脈ごとの重み(520, 390, 145, 251, 209)/1024、正の要素の数による倍率(94, 103, 110, 106, 119, 126)/64、1手前に73 | `src/search.cpp:2050-2070` |
| 並べ替え | butterfly historyとpawn historyの2倍、continuation historyの5文脈の和、low ply historyの`8/(1 + ply)`倍とその深さ5、王手の加点のSEEの閾値−75 | `src/movepick.cpp:233-251`、`src/history.h:42` |
| statScore | 静かな手`(2252, 1126, 1093)/1024`、捕獲手の駒の価値873/128 | `src/search.cpp:1371-1378` |
| lmrDepthへの項 | butterfly history 69/32、除数`3000 + 7(min(d, 16) − 8)²` | `src/search.cpp:55-58, 1236-1240` |

## 所要時間の見積り

時間は、同時対局数16の測定機での見積りである。
SPSAは、[SPSAの手引き](../guides/spsa.md)が記録する同時対局数16で3,000ペアの完走実績（27,138秒、1ペア約9秒）に基づき、30,000ペアを約75時間（約3日）と見込む。
2026年10月17日の別のセッションでは1反復あたり約1分で進んだが、暫定の観測である。
1反復の時間は持ち時間で決まるので、係数の数が増えても、表の参照で探索速度が下がっても、大きくは変わらない。
段階ゲートの時間は、判定までのペア数が強さの差で変わるので幅を持たせた。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 形の変更の実装と検証（codexへ委任） | 2日程度 |
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
続いて、M0、M1、M2、M3、M4、M5の順に、1項目を1コミットにする。
単位の変更、M0、および王手の判定の移行のコミットでは、benchの全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が起点と一致することを確かめる。
M1からM5の各コミットでは、同じbenchのノード数を記録し、直前のコミットとの差を残す。
M3で表を加える前のコミットでは、M1とM2の静かな手の表の更新はbutterfly historyだけに行い、ほかの表への更新はM3のコミットで加える。
起点に王手の判定の前計算またはS5の静かな手のSEEの判定器がなければ、依存関係の節のとおり、その移行をM0の後の別のコミットとして入れ、ノード数の一致で検証する。
`taskset -c 3`で1つのコアに固定した`minase dev bench --depth 9 --repetitions 3`の中央値を、起点と形の変更で交互に3回ずつ測り、探索速度の変化を記録する（[教訓](../lessons/pin-bench-to-performance-core.md)）。
同じ実行で最大常駐メモリも記録し、増分がcontinuation historyの約140 MBとpawn historyの約17 MB×（ワーカー数以上の最小の2のべき）に見合うことを確かめる。
新しい係数を`params.rs`の表へ追加し、廃する4係数を表から削除し、置き直す3係数の開始値と範囲を改める。
調整用ビルド（`--features tuning`つきのビルド）が新しい係数を`Tune_<係数名>`のUSIオプションとして宣言し、廃した係数を宣言しないことを確かめる。
新しい式の単体テストは、式の写しではなく数値の例で書く（[テストの手引き](../guides/testing.md)）。
着手の記録とcontinuation historyのテストは、null moveがそのplyの記録を空にすること、文脈の手を持たない項は参照も更新もしないこと、1手前がnull moveでも2手前以降の記録があればその項を参照すること、根のノードでは文脈の手がないこと、後手の手で升を180度回転して引くこと、fail lowの加点では1つ前のplyの手を指した側を基準に回転すること、成る手でも着手前の駒種で引くこと、記録から着手後の駒を導けること、および更新後の値が表の上限の範囲に収まることを確かめる。
pawn historyのテストは、差分更新した鍵が局面から計算し直した鍵と一致することを確かめる。
並べ替えのテストは、価値の低い駒の脅威の集合が駒価値の順の累積の利きと一致すること、王駒の手では脅威の項が0であること、およびSEEが閾値を下回る王手には加点しないことを確かめる。
M2のテストは、置換表の手が不合法なら加点しないこと、置換表の値がα以下の打ち切りでは更新しないこと、および直前の手が捕獲手かnull moveなら静かな手への加点をしないことを確かめる。
LMRのテストは、`q_hist`がallNodeの拡大の前に引かれること、残り深さ2のノードでも負の減深量による延長とPVノードの1手の加算がF4のとおり保たれること、および根の途中結果に再探索の後の値だけが記録されることを確かめる。
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
新しい15係数と置き直した3係数については、最終値が開始値からどれだけ離れたかを、`c_end`に対する比で示す。
この比は、どの仕組みが調整で動いたかを見る目安であり、採否には使わない。
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

- 単位の変更とM0のコミットで、benchの全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。
- M1からM5の各コミットで、benchのノード数の変化が記録されていること。
- 探索速度と最大常駐メモリの変化が記録されていること。
- 新しい15係数が調整用ビルドの`Tune_`オプションとして宣言され、廃した4係数が宣言されず、パラメーターファイルの開始値と範囲が表と一致すること。
- 候補の`params.rs`の差分が既定値の書き換えだけであり、調整用ビルドの宣言の既定値が`apply`の出力と一致すること。
- `cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`が通過すること。
- 時間切れの事前確認、SPSA、事前対局、および採否の測定で、エンジン異常、時間切れ、および拒否着手が0件であること。最初の測定で1件でも出た場合は、測定を止めて原因を調べる。

## 完了条件

- 形の変更の検証結果、時間切れの事前確認、SPSAの結果、事前対局、および採否が、測定記録に残っている。
- 採用した場合は、候補のブランチがmasterへ統合され、依存関係の節に挙げたADRが新しいADRに置き換えられている。統合は利用者の許可を得てから行う。
- 採用した場合は、lishogiのBotのコンテナのメモリ上限が、増えるメモリを含めて足りることを確かめてからBotを更新する。
- 不採用の場合は、結論と測定記録だけをmasterへ残し、候補のブランチはmasterへ統合しない。
- ROADMAPの状態表が本書の結論と一致している。

## 参考資料

- [探索と時間管理の係数の関数形に関する既存実装との比較](../research/search-coefficient-forms-prior-art.md)。
- [手の順序付けの改善](archived/move-ordering.md)と、counter move historyとcontinuation historyの再診断を含む[診断の記録](../measurements/move-ordering-diag.md)。
- 根の探索ごとに初期化する表での発動率の記録（[counter move history](../measurements/strength-stage5-cmh-bench.md)、[continuation history](../measurements/strength-stage5-continuation-bench.md)）と、その表の設計を記した[棋力向上段階5](archived/strength-stage5.md)。
- Stockfish（コミット5c4cef0b）の`src/search.cpp`。lmrDepthの除数は55〜58行、low ply historyの初期化は339行、butterfly historyの根ごとの減衰は341〜343行、文脈の表の設定は688〜700行、表の初期化は713〜727行、静的評価の変化による更新は1002〜1012行、置換表の打ち切りでの更新は902〜910行、文脈の表の配列は1140〜1142行、continuation historyによる枝刈りとlmrDepthへの項は1227〜1240行、statScoreとLMRの補正は1371〜1381行、LMRの適用条件は1395行、最善手があるときの更新は1602〜1608行、fail lowの加点は1611〜1642行、加点と減点は2004〜2041行、continuation historyの更新は2049〜2073行、静かな手の表の更新は2077〜2089行。`src/movepick.cpp`の静かな手の並べ替えは233〜251行。`src/history.h`の重力式は70〜76行、表の型と上限は128〜146行、共有の表は198〜225行。`src/types.h`の歩兵価値は199行。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`。low ply historyの初期化は1540行、置換表の打ち切りでの更新は2735〜2748行、静的評価の変化による更新は3126〜3132行、continuation historyによる枝刈りは3646〜3661行、LMRのhistory補正は3925〜3937行、fail lowの加点は4311〜4339行、加点と減点は5308〜5350行。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)、[SPRTによる棋力測定の手引き](../guides/sprt.md)、および[SPSAの反復予算の調査](../research/spsa-iteration-budget.md)。
