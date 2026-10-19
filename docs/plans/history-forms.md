# historyの更新量の形とcontinuation historyの設計書

## 先に読む要約

本書は、手の成績を学習する表（history）の更新と使い方をチェスのStockfishの形へそろえ、現行のmasterより強くなるかを確かめる。
minaseのbutterfly history（手番、移動元、移動先ごとの静かな手の成績の表）は、β打ち切り（ある手の値が探索窓の上端βに達し、残りの手を読まずに打ち切ること）のたびに深さの2乗に比例して加減点するので、1回の更新が表の上限に占める割合は、深さ10でStockfishの約半分、深さ2で約7分の1と小さい（[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)）。
本書が変えるのは5つである。
1つ目に、加点と減点の量をStockfishの上限つき1次式に置き換え、表の上限と値の尺度をStockfishと同じにする（history bonus/malus）。
2つ目に、historyを更新する場面を、最善手がαを上げたノード、置換表（探索結果を局面ごとに保存する表）による打ち切り、fail low（全手がα以下）のノードへ至った手、および静的評価の変化へ広げる。
3つ目に、1、2、3、4、6手前の手を文脈として静かな手の成績を引くcontinuation history、歩兵と仲人の配置で引くpawn history、および根に近いplyで引くlow ply historyを新設し、手の並べ替えに使う。並べ替えには、王手になる手への加点と、価値の低い駒に狙われる升への出入りの加減点も加える。
4つ目に、late move reductions（LMR、後方の手を浅く読む減深）の補正に、現行の閾値による段差を残したまま、history値に比例する連続量（statScore）を加え、continuation historyの値が低い静かな手を読まない枝刈りを加える。
5つ目に、LMRを、先頭の手を除くすべての手と根へ広げ、根でもhistoryを更新する。
killer手や段差のように中将棋の診断と測定で決めた既存の仕組みは残す。
新しい係数の開始値はStockfishの値とし、既存の係数とまとめてSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で調整する。
調整した版を、起点のmasterと短時間条件STCの逐次検定で対局させ、強さの変化を記録する。
[統括計画](search-forms.md)の決定により、本書の変更はSTCの結果によらず採用し、長時間条件LTCは行わない。
完了条件は、SPSAとSTCの測定記録を残し、候補をmasterへ統合することである。

## 状態

着手。2026年10月19日に、利用者の指示を受けて着手した。
起点は、[浅い枝刈りの深さと対象の拡張](archived/shallow-pruning-extensions.md)を採用で終えた後のmasterである。
起点には、依存関係の節が前提とする仕組み（1/100手単位のLMR、枝刈り用のlmrDepth、王手の判定、静かな手のSEE、followPV、およびplyごとの補正後の静的評価）がすべてあるので、それらを導入する移行のコミットは置かなかった。
着手前に、本書の記述を起点のコードとStockfishの写しに照らして見直し、更新する手の列の定義、置換表の打ち切りでの更新の条件、LMRの後のcontinuation historyの更新、および置き直す係数の値を改めた。
フェーズ1（形の変更の実装）は終えた。結果は[探索ノード数と探索速度の記録](../measurements/history-forms/history-forms-bench.md)にある。
M3bの時点では、並べ替えの変更で深さ9を読み終える時間が約12%延びるにとどまったので先へ進んだ。最後のコミットで測ると、並べ替えで規則どおりの王手の判定をすべての静かな手に使う費用が探索時間の約19%を占めたので、Stockfishの`check_squares`と同じ直接の王手の判定に改めた（設計判断の「並べ替えの王手と脅威の項」）。これはClaudeの判断である。
最後のコミットは起点に比べて、深さ9の探索ノード数が22%少なく、NPSが約34%低く、深さ9を読み終える時間が約17%長い。
係数の既定値を範囲の1/3と2/3に置いた表でもテストが通ることを、SPSAの前に確かめた。
利用者の判断を待つ点はない。
次の一手は、フェーズ2（パラメーターファイルと時間切れの事前確認）である。

## 目的

本書が確かめるのは、「historyの更新と使い方をStockfishの形へそろえ、係数を調整すれば、現行の形で調整した探索より強くなる」という仮説である。
現行の加点は`min(深さ² × HistoryBonusScale/100, HistoryLimit)`であり、起点の既定値（尺度336%、上限4384）では、深さ10の加点は336で上限の約7.7%、深さ2の加点は13で上限の約0.3%である。
Stockfishの静かな最善手への加点は、深さ10で1096、深さ2で162であり、表の上限7183のそれぞれ約15%と約2.3%に当たる（`src/search.cpp:2004-2006, 2015`）。
Stockfishでは、値が上限に近づくほど更新量を小さくする重力式（後述）による忘却が働く。
minaseでは浅いノードでの1回の更新が小さいので、重力式による忘却は働きにくく、根の探索ごとの減衰（`HistoryDecay`）が忘却の多くを担っていると推測される。
また、直前の手を文脈とする表は、[手の順序付けの改善](archived/move-ordering.md)の診断で、表を対局内で持ち越し、根の探索ごとに23%へ減衰させる条件で、静かな手の順序を変えるノードがSTC相当で44.6%（1手前）と61.3%（2手前）あると分かったが、表の大きさの設計が残っていた（[診断の記録](../measurements/move-ordering/move-ordering-diag.md)）。
2026年10月17日の利用者の決定により、形や値は両エンジンで違えばStockfishに合わせ、Stockfishにだけある項目は含める。
同じ日の利用者の決定により、minaseにあってStockfishにない既存の仕組みのうち、中将棋の診断や測定で決めたものは残し、Stockfishの仕組みを並べて足す。
本書は、historyに関わるStockfishの仕組みを1つの候補にまとめ、係数を調整し直して測る。

## 適用範囲

本書の用語を次のように定める。

- **起点**は、前の設計書が完了した後、フェーズ1を始めた時点のmasterの先頭のコミットである。既存の係数の現行値（`LmrDivisor`、`HistoryDecay`、`CaptureHistoryScale`など）は、起点の`crates/minase/src/search/alphabeta/params.rs`の表の既定値を指す。本書が例に挙げる現行の値は起点の値である。
- **形の変更**は、本書の項目を実装したコミット列であり、新しい係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で`params.rs`の表の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **bench**は、固定の15局面を指定の深さで探索し、探索ノード数、評価値、最善手、および探索速度を出力する`minase dev bench`である。
- **ワーカー**は、1つの探索を並列に進める探索スレッドであり、その数はUSIオプション`Threads`で決まる。
- **butterfly history**は、`history.rs`の`HistoryTable`であり、手番側、移動元、移動先で引く静かな手の成績である。**捕獲履歴**（capture history）は、`capture_history.rs`の、動かす駒、到達升、取られる駒種で引く捕獲手の成績である。
- **重力式**は、符号つきの更新量`b`を表の上限`D`で`[−D, D]`に切り詰めてから、値`v`を`v + b − trunc(v×|b|/D)`へ置き換える更新式である。`trunc`は0の方向への切り捨てである。現行の`record_quiet_beta_cutoff`と同じ式であり、Stockfishの`StatsEntry::operator<<`（`src/history.h:70-76`）も同じ形である。値は常に`[−D, D]`に収まる。
- **Stockfishの尺度**は、表の値と更新量をStockfishと同じ数値で表すことをいう。本書は表の上限をStockfishと同じ値にするので、Stockfishの定数を換算せずに使える。評価値だけは尺度が違うので、minaseの評価値`e`を`e × 208/P`（`P`は探索用の歩兵価値、208はStockfishの歩兵価値、`src/types.h:199`）へ換算して使う。
- **静かな手の段**は、手選択器（`ordering.rs`の`MovePicker`）が置換表の手、捕獲手、killer手（同じ深さの別のノードでβ打ち切りを起こした静かな手）を返した後に、残りの静かな手を並べ替えの値の降順に返す区切りである。
- **着手の記録**は、`Searcher`の`path_moves`の要素に持たせる、そのplyのノードで指した手の`Move`、着手前の駒コード（所有者を含む）、および捕獲手なら捕獲履歴の添字に使う取った駒種の組である。2枚取りの取った駒種は、捕獲履歴の現行の添字と同じく価値の高い方（同値なら到達升の駒）とする。通常探索のply `p`のノードでは`path_moves`の長さが`p`に等しく、ply `p`の記録は、ply `p`の局面からply `p + 1`の局面へ進めた手を表す。着手後の駒（成る手なら成った駒）は、`Move`の成りの指定と着手前の駒コードから導く。null moveはそのplyの記録を空にし、根より前のplyは記録を持たない。静止探索は`path_moves`へ積まないので、記録は通常探索だけで参照する。[correction historyの鍵の追加の設計書](correction-history-tables.md)のC3も、同じ記録を使う。
- **文脈の手**は、ply `p`のノードで手を引くときに参照する、ply `p − i`（`i`は1、2、3、4、6）の記録の手である。記録が空のplyは文脈の手を持たない。
- **ノードの手数**は、そのノードで手選択器の候補列から取り出した適格な合法手の数であり、枝刈りで飛ばした手も含み、R2とR3の禁止手は含めない。Stockfishの`moveCount`（`src/search.cpp:1154`）に当たる。起点の`negamax`の0始まりの添字`index`は禁止手で進まず、枝刈りした手で進むので、処理中の手のノードの手数は`index + 1`である。`Searcher`はこの値を手を処理するたびにplyごとの配列へ書き、子のノードは親の値を読む。同じ手の再探索では値を変えない。
- **探索済みの手の列**は、ノードで実際に読んだ手のうち、ノードの手数が32以下で、読んだ時点でαを上回らなかった手を、静かな手と捕獲手に分けて読んだ順に並べたものである（Stockfish `src/search.cpp:75, 1578`）。αを上回った手は、後で最善手でなくなっても列に加えない。M1の減点の対象と、零窓のノードでの加点の上乗せに使う手の数は、この2つの列である。
- **statScore**は、手を読む直前にM4の式で求める、その手の成績の値である。`Searcher`はplyごとの配列に書き、ノードの開始時にそのplyの値を0にする（Stockfish `src/search.cpp:828`）。子のノードは親の値を、このノードへ至った手のstatScoreとして読む。
- **improving**は、`negamax.rs`の`improving`が定める判定であり、同じ手番の2手前の局面より静的評価が上がったことをいう。
- **PVノード**は窓の幅が1より大きいノード、**零窓のノード**は窓の幅が1のノードである。
- **i16**は、−32768から32767までを表す16ビットの符号つき整数である。
- **PST**と**FM**は、既定の評価関数がNNUEに置き換わる前に使っていた評価関数の部品であり、それぞれ駒と升ごとの評価値の表と、2駒の関係による評価の補正である。`FmScale`はFMの補正に掛ける倍率であり、NNUEでは使わないのでSPSAの対象から外す。
- 減深量と深さの単位は、探索の深さ1を1手と数える。

対象は次の6項目である。

- 着手の記録など、後続の項目が使う探索経路の状態の追加（M0、挙動を変えない移行）。
- 加点と減点の形と表の上限（M1）。
- historyを更新する条件の拡大（M2）。
- continuation history、pawn history、およびlow ply historyの新設と更新（M3a）と、王手と脅威の項を含む手の並べ替え（M3b）。
- LMRと枝刈りでのhistoryの使用（M4）。
- LMRの対象の全手と根への拡大と、根でのhistoryの更新（M5）。

次の事項は対象外とする。

- 置換表の手の成績の表（Stockfishの`ttMoveHistory`）。Stockfishはこの表をsingular extensionの余裕値にだけ使い（`src/search.cpp:1290, 1308, 1608`）、minaseにはsingular extensionがない。
- continuation historyの王手の次元。Stockfishは表を、文脈の手を指した局面が王手を受けていたかで分け、王手を受けている局面では2手前までの表だけを更新する（`src/search.cpp:690-691, 2059-2061`）。minaseは王手を受けている局面を区別しない（王駒を利きへ置く手も合法であり、RULES.md第8条第3項から第5項）。
- killer手の表の変更。killer手はStockfishにないminaseだけの仕組みであり、2026年10月17日の利用者の決定（中将棋の診断や測定で決めた仕組みは残す）により、本書はその更新と手選択器の段を変えない。
- LMRの減深量のうちhistory以外の項（cutNode、allNode、ttPv、置換表の手が捕獲手の場合、子ノードの打ち切りの回数、`α − eval`、手数、補正量など）と、減深の結果に応じて読み直しの深さを増減する処理。これらは[探索の枝刈りとLMRの関数形の見直し](archived/search-pruning-forms.md)と[correction historyの設計書](correction-history-tables.md)が扱う。本書のstatScoreの項は、それらの項と同じ式の中で引く（M4）。
- 置換表で打ち切る条件そのもの。本書は打ち切ったときの更新だけを加える。
- correction historyにcontinuationの鍵を加えること。[correction historyの設計書](correction-history-tables.md)が扱う。

## 依存関係

起点は、[浅い枝刈りの深さと対象の拡張](archived/shallow-pruning-extensions.md)が完了した後のmasterの先頭である。
起点の表の既定値をSPSAの開始値とする。

M4とM5は、LMRの減深量を1/100手単位の整数で計算し、最後に手単位へ直す形を前提にする。
この形は[探索の枝刈りとLMRの関数形の見直し](archived/search-pruning-forms.md)のF4が導入し、起点の`pruning.rs`の`LmrCoefficients::lmr_reduction`にある。
LMRの減深量と減深後の深さの式は、F4の式を正本とする。
本書はF4の`r₀`からstatScoreの項`q_hist`を引くことだけを加え、その位置はallNodeの拡大の前とする（M4）。
[correction historyの設計書](correction-history-tables.md)のC5の`q_corr`もallNodeの拡大の前に引き、[浅い枝刈りの深さと対象の拡張](archived/shallow-pruning-extensions.md)のS9の詰みを探す局面での調整はallNodeの拡大の後に行う。

M4の減深後の深さ（lmrDepth）へのhistoryの項は、[浅い枝刈りの深さと対象の拡張](archived/shallow-pruning-extensions.md)が静かな手のfutility pruningとSEEによる枝刈りに導入した枝刈り用のlmrDepth（`pruning.rs`の`pruning_lmr_depth`）に足す。

pawn historyは、歩兵と仲人の配置の鍵（pawn key）で引く。
この鍵は本書が導入し、[correction historyの設計書](correction-history-tables.md)のC2は本書の鍵を再利用する。

M3bの王手の加点は、Stockfishの`check_squares`と同じく、ノードで駒種ごとに求める直接の王手の升の集合で判定し、[浅い枝刈りの深さと対象の拡張](archived/shallow-pruning-extensions.md)のS3が導入した静かな手のSEEの判定器（`see.rs`の`see_prunes`）を再利用する。
同じ設計書が導入した王手の判定`Position::gives_check`（RULES.md第8条の定義に従い、開き王手と王駒が2枚の場合を含む）は、枝刈りでの王手の判定に使い続け、並べ替えには使わない。
SEEの判定器の引数を符号つきの閾値`T`（`SEE < T`なら真）に改める契約の移行（[静止探索の設計書](qsearch-forms.md)）には、王手の加点に使う判定も含め、閾値`−trunc(75P/208)`を保つ。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと共用するので、測定機での対局の同時対局数は16を上限とする。
時間切れの事前確認とSPSAのセッションは、[探索の関数形の見直しの統括計画](search-forms.md)に従い、vast.aiで借りる調整機で行う。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は次のADRの決定の一部を変える。
採用した場合は、完了時に、起点の時点で有効なADR（先行の設計書が置き換えていればその後継）を確認し、新しいADRで置き換える。
部分的に変える場合は、残る決定も新しいADRへ引き継ぐ。

- [LMRの式のADR](../adr/archive/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)（2026年9月7日）は、history補正を閾値による±1の段差とし、連続量の案を比例係数と切り詰めの2つの自由度を絞る診断が要るとして棄却した。対象は置換表の手、捕獲手、killer手を除く非捕獲手とし、根では行わないとした。M4は段差を残し、Stockfishの連続量を並べて加える。比例係数はStockfishの値から始めてSPSAで調整する。M5は対象を先頭の手を除く全手と根へ広げる。このADRは評価関数がPSTだった時期の測定に基づき、上限3と段差を「STCの到達深さが5〜6だった時期の判断」として見直しの余地を留保している。同じADRの上限は[探索の枝刈りとLMRの関数形の見直し](archived/search-pruning-forms.md)のF4も変える。
- [静かな手の順序のADR](../adr/order-quiets-with-malus-and-pst-tiebreak.md)（2026年10月10日）は、加点と減点を別の尺度で深さの2乗に比例させ、重力式で更新すると決めた。M1は加点と減点をStockfishの上限つき1次式に置き換え、M2は更新する場面を増やし、M3bは並べ替えの値に新しい表を加える。重力式と同点破りは変えない。このADRはPSTを既定の評価関数としていた時期の測定に基づき、同点破りの効果をNNUEで測り直していないことと、尺度がSPSAの調整を待つ既定値であることを記している。
- [historyの持ち越しのADR](../adr/carry-history-tables-within-a-game.md)（2026年10月10日）は、butterfly historyと捕獲履歴を対局内で減衰させて持ち越し、killer表と補正表は持ち越さないと決めた。M3aはcontinuation historyとpawn historyを加え、新規対局などの時点で初期化して対局内で減衰させずに持ち越し、low ply historyは探索ごとに初期化する。butterfly historyと捕獲履歴の減衰の仕組みは変えず、`HistoryDecay`の開始値をStockfishの値に置き直す。このADRはPSTとFMを既定の評価関数としていた時期の測定に基づき、continuation historyの設計を別の設計書に委ねている。
- [butterfly表だけのADR](../adr/keep-only-the-butterfly-history-table.md)（2026年9月7日）は、静かな手のhistoryをbutterfly表だけで構成すると決め、直前の手と組み合わせる表は対象外とした。M3aはcontinuation history、pawn history、およびlow ply historyを加える。駒種と到達升で引く表（piece-to history）は、Stockfishにもないので加えない。このADRはPSTの時期の測定に基づき、4次元の表が疎だという判断は表を根の探索ごとに初期化していた時期のものなので、再提案するときは現行の表の寿命で測り直す必要があると留保している。
- [捕獲履歴のADR](../adr/bound-capture-history-and-use-it-in-qsearch.md)（2026年10月10日）は、捕獲履歴の換算値を歩兵価値の定数倍に抑えて並べ替えに使い、重力式で更新すると決めた。M1は捕獲履歴の更新量と上限をStockfishの形にし、M4は捕獲手のLMRにも使う。並べ替えへの換算の形は変えず、尺度`CaptureHistoryScale`の開始値をStockfishの換算値に置き直す。このADRはPSTの時期の測定に基づき、LMRへの使用については留保していない。

[late move pruningの数え方のADR](../adr/archive/count-only-quiet-moves-for-late-move-pruning-and-shrink-the-limit-when-not-improving.md)、[killer手を損な先頭捕獲より先に返すADR](../adr/try-killers-before-a-losing-first-capture.md)、および[根の手の並べ替えのADR](../adr/order-root-moves-by-previous-subtree-nodes.md)の決定は変えない。

## 設計判断

| 項目 | 決定 |
|---|---|
| 合わせる先 | 2026年10月17日の利用者の決定により、形や値がStockfishとYaneuraOuで違う場合はStockfishに合わせ、YaneuraOuにだけある項目は入れず、Stockfishにだけある項目は入れる。本書の範囲では、YaneuraOuもpawn history、low ply history、continuation historyによる枝刈り、および静的評価の変化による更新を持つ（`yaneuraou-search.cpp:1540, 3126-3132, 3646-3661`）。同じ日の利用者の決定により、minaseにあってStockfishにない既存の仕組みのうち、中将棋の診断や測定で決めたもの（killer手、LMRの段差、静かな手の同点破り、捕獲履歴の静止探索での使用、根の手の並べ替え、および捕獲履歴の根ごとの減衰）は残し、Stockfishの仕組みを並べて足し、残したものもSPSAで調整する。 |
| 候補の束ね方 | 全項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回のSTCで強さの変化を記録する。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。項目ごとにSPSAと段階ゲートを繰り返す案は、1項目あたり、調整機でのSPSAに約13〜18時間と約25〜35米ドル、測定機でのSTCとLTCに約6〜38時間を要するので採らない。代償として、項目ごとの寄与は測らない。 |
| 開始値と検証 | 2026年10月17日の利用者の決定により、新しい係数の開始値はStockfishの値とする。既存の係数のうち、Stockfishに同じ形の値がある`HistoryDecay`（32から71）と`CaptureHistoryScale`（285から734）は、その換算値に置き直す。`LmrHistoryThreshold`はStockfishに対応する値がないので、表の上限を7183へ改める尺度の変更に合わせて、`round(起点のLmrHistoryThreshold × 7183 / 起点のHistoryLimit)`を開始値にする（起点の9と4384なら15）。本書で置き直す既存の係数はこの3つに限り、ほかの係数は起点の調整済みの値のまま始める。各項目のコミットでbenchのノード数の変化を記録し、挙動を変えない移行（M0）だけをノード数の一致で検証する。 |
| 尺度と表の上限 | butterfly historyと捕獲履歴の上限を、係数`HistoryLimit`と`CaptureHistoryLimit`から、Stockfishと同じ定数7183と10692に改め（`src/history.h:128, 135`）、両係数を廃する。新しい表の上限もStockfishと同じにする。表の上限がStockfishと同じなら、Stockfishの更新量と重みを換算せずに使え、表の間の値の比もStockfishと同じになる。上限を係数として残すと、更新量を上限に比例させて換算する限り、上限は値の分解能を変えるだけで探索の挙動に効かなくなる。 |
| 係数にする値と定数にする値 | 2026年10月17日の利用者の決定により、SPSAで調整するのは各仕組みの強さを決める値（傾き、切片、上限の手前で効く量、重み、倍率）だけとし、上下限、平均の重み、更新の内部の定数はStockfishの値で固定する。本書の新しい係数は15個であり、固定する72個の定数は後述の「定数」の表に行番号つきでまとめる。 |
| 加点と減点の形（history bonus/malus） | 現行の深さの2乗の項（`HistoryBonusScale`、`HistoryMalusScale`）を廃し、Stockfishの`min(傾き×d − 切片, 上限)`に置き換える（`src/search.cpp:2004-2006`）。加点には、最善手が置換表の手なら定数を、直前の手のstatScore（後述）の1/28を足し、零窓のノードでは探索済みの手の列の長さに比例して増やす（同2007-2011）。静かな最善手には加点の899/1024、探索済みの静かな手には減点の1159/1024を1手ごとに`HistoryMalusDecay`/1024倍しながら与え（同2013-2023）、捕獲の最善手には加点の1427/1024、探索済みの捕獲手には減点の1489/1024を与える（同2024-2042）。減点の対象は、現行の「打ち切りまでに読んだすべての手」から、Stockfishと同じ探索済みの手の列（ノードの手数32以下で、読んだ時点でαを上回らなかった手、同1578）に改める。係数の範囲によって`b`や`m`が負になっても、符号つきのまま式を評価する。 |
| 減点の逓減（malus decay） | 探索済みの静かな手の列の先の手ほど大きく減点する。Stockfishは、読んだ順に、減点を1手ごとに921/1024倍してから与える（`src/search.cpp:2018-2022`）。係数の上限を1024にするのは、1024を超えると後の手の減点が大きくなり、重力式の切り詰めに頼ることになるからである。 |
| 更新の条件の拡大 | Stockfishと同じく、最善手が探索開始時のαを上げたノードで更新する（`src/search.cpp:1602-1608`）。起点の`negamax`はfail lowでも`best_move`を設定するので、判定は`best_score > original_alpha`で行う。β打ち切りに限っていた現行より更新が増える。killer手の更新はminaseだけの仕組みなので、現行どおりβ打ち切りに限る。加えて、置換表の打ち切りの条件が成り立ったときの静かな置換表の手への加点と直前の手への減点（同902-910）、fail lowのノードでの直前の静かな手への加点と直前の捕獲手への捕獲履歴の加点（同1611-1643）、前の最善手が反駁されたときの直前の手への追加の減点（同2031-2035）、直前の手の前後の静的評価の変化による更新（同1002-1012）、およびLMRで減深した探索の値がαを上回った手へのcontinuation historyの加点（同1423）を加える。置換表による更新は、Stockfishと同じく、打ち切りの前に置換表の手の子局面を照合する処理より前に行うので、照合の結果で打ち切りを見送っても更新は残る。 |
| 置換表の手の確認 | 置換表の打ち切りでの置換表の手への加点は、置換表の手が現局面で合法な静かな手であることを`MoveGenerator::is_legal_move`で確かめてから行う。置換表の手はハッシュの衝突で別の局面のものである場合があり、continuation historyを引くには移動元の駒種が要るからである。Stockfishは合法性を確かめずに更新するが、チェスの表は移動元と到達升だけで引けるので、別の局面の手でも表の外を読まない。同じ打ち切りでの直前の手への減点は、直前の手の記録だけを使うので、Stockfishと同じく置換表の手があれば、捕獲手でも合法性を確かめずに行う。 |
| 静的評価を求めるノード | 静的評価の変化による更新は、直前の手のノードと現在のノードの補正後の静的評価を使う（`src/search.cpp:845-875, 1002-1012`）。[探索の枝刈りとLMRの関数形の見直し](archived/search-pruning-forms.md)のF2がノードごとに1回求めて保持する補正後の評価を再利用し、historyの更新のために補正表を読み直さない。起点の`Searcher::static_evals`がこの値を保持している。Stockfishは、現在のノードか直前の手のノードが王手を受けていればこの更新とfail lowの加点の一部の項を行わない（同1001、1618-1619）。王手の局面では`staticEval`が2手前の値の写しであり、実際の評価ではないからである（同848）。minaseはすべてのノードで静的評価を求めるので、この条件を置かない。王駒に相手の疑似利きが届くノードで既存の枝刈りを止めるのは探索の判断であり、historyの更新に混じる雑音は係数の調整が吸収するとみる。 |
| continuation historyの表の形 | 2026年10月17日の利用者の決定により、表はStockfishと同じく1組の共有の表とし、何手前の文脈も同じ表の中の区画を引く（Stockfish `src/history.h:198-199`、`src/search.cpp:690-691, 1140-1142`）。手は180度の回転で引き、表を引く手を指す側が後手なら、文脈の手と現在の手の升をともに回転する。中将棋は初期配置と規則が180度の回転と手番の入れ替えについて対称なので、先手の局面と回転した後手の局面の統計を同じ要素に集められる。添字は「文脈の手が捕獲手か（2）、文脈の手を指したのが表を引く手と同じ側か（2）、文脈の手の着手前の駒種（29）、文脈の手の到達升（144）、現在の手の着手前の駒種（29）、現在の手の到達升（144）」とし、要素はi16とする。29は成駒を含む駒種の数（`PIECE_KIND_COUNT`）、144は盤の升の数である。同じ側かどうかの次元は、Stockfishの添字の駒が手番の色を含むことに当たり、文脈の手が何手前かの偶奇で決まる。捕獲手かどうかの次元はStockfishに合わせて持つ。王手の次元は対象外の節のとおり持たない。文脈の手を持たない項は参照も更新もせず、並べ替えなどでは0として足す。Stockfishは文脈の手がない場合に初期値の区画を引くので（`src/search.cpp:690-691`）、これは意図した違いである。 |
| continuation historyの大きさと共有 | 表は69,755,904要素で約140 MBであり、探索チームの全ワーカーで共有する。Stockfishは同じNUMAノードのワーカーの間で共有する（`src/history.h:201-212`）。要素は不可分の読み書き（`AtomicI16`の`Relaxed`順序の読み出しと書き込み）で扱い、競合した更新が失われることを許す。Stockfishも同じく競合を許す（同138行の`AtomicStats`）。共有するのでメモリはワーカー数によらない。捕獲手かどうかの次元を持たなければ約70 MBになるが、Stockfishとの一致を崩すので採らない。表は`HistoryTables`が持ち、各ワーカーの探索状態へ共有の参照で渡す。表は新規対局、受理した規則と置換表の容量`USI_Hash`の変更、ワーカー数の変更、および引き分けの評価値`DrawValue`の変更の時点で初期値にし、対局内で減衰させずに持ち越す。Stockfishも減衰させるのはbutterfly historyだけである（`src/search.cpp:341-343`）。効果の根拠とした診断は、1手前と2手前の2表を根の探索ごとに23%へ減衰させる条件で測ったものであり、本書の表の形と寿命での発動率は測っていない。 |
| pawn historyの形 | Stockfishと同じく、歩兵の配置の鍵で引くエントリごとに、動かす駒と到達升の表を持つ（`src/history.h:146, 220-225`）。中将棋の鍵は、規則書が「価値ある駒」から除く2種（RULES.md第3条第11項）である歩兵と仲人の配置とし、[correction historyの設計書](correction-history-tables.md)のC2の鍵と同じ定義にする。鍵は所有者、歩兵か仲人か、および升ごとの64ビットの乱数の排他的論理和であり、乱数は`correction.rs`の材料の鍵と同じく固定のシードのsplitmix64で作るが、シードは別にする。`material_keys`と同じくplyごとの配列で差分更新し、着手の取り消しの記録（`Undo`）から、動かした駒が歩兵か仲人なら移動元を、着手後も歩兵か仲人なら到達升を、取った駒が歩兵か仲人ならその升を出し入れする。材料の鍵は加減算なので、演算は写さない。null moveでは親の鍵をそのまま使う。鍵は盤の絶対の配置なので、この表は回転せず、駒は手番を含む58種で引く。1エントリは駒58種×升144のi16で約16.7 KBになり、チェスの約8倍なので、エントリの数をStockfishのワーカーあたり8192から1024に減らし、ワーカー1つ分の大きさをStockfishと同程度（約17 MB）にする。エントリの数は1024にワーカー数以上の最小の2のべきを掛けた数とし、表は全ワーカーで共有する。Stockfishも表の大きさをワーカー数に比例させて共有する。USIの`Threads`の上限256では表だけで約4.4 GBになるが、多数のワーカーを使う機械はメモリも大きいとみて、上限は変えない。 |
| low ply historyの形 | Stockfishと同じく、根からのplyが5未満のノードで、plyと移動元と到達升で引く表を持つ（`src/history.h:42, 132`）。Stockfishは反復深化のループに入る前に全要素を102にする（`src/search.cpp:339`）。minaseでは、探索ごとに各ワーカーの探索状態（`Searcher`）を作る時点で102にし、反復深化と窓の読み直しでは初期化しない。ワーカーごとに持ち、約200 KBである。 |
| 並べ替えの値 | 静かな手の段の値を、Stockfishと同じく`2×butterfly + 2×pawn + (1、2、3、4、6手前の文脈のcontinuation historyの和) + 8×low ply/(1 + ply)`とする（`src/movepick.cpp:233-251`）。同点破り（線形経路の差分、生成順）は変えない。根の並べ替えは前回の反復の部分木ノード数で決めるので変わらない。 |
| LMRのhistory補正（statScore） | 閾値による±1手の段差（`LmrHistoryThreshold`）は、[棋力向上段階5](archived/strength-stage5.md)の診断と測定で決めた仕組みなので、2026年10月17日の利用者の決定により残す。そこへStockfishのstatScoreに比例する連続量を並べて加える（`src/search.cpp:1371-1381`）。statScoreは、静かな手では`(2252×butterfly + 1126×1手前 + 1093×2手前)/1024`、捕獲手では`873×取る駒の価値/128 + 捕獲履歴`である。取る駒の価値は、獅子の2段階移動で2枚を取る手では合計してから、Stockfishの尺度へ1回だけ換算する。statScoreは、LMRを適用するかどうかによらず、実際に読むすべての手（先頭の手と根の手を含む）について求めて記録する。子のノードのM1とM2が、このノードへ至った手のstatScoreを使うからである。段差はbutterfly historyの値で判定し、捕獲手には使わない。statScoreは段差の判定に使わない。表の値がStockfishの水準に上がると段差はほとんどの手で働くようになるが、閾値を調整の対象に残すので、SPSAが閾値を上げて段差を弱めることもできる。 |
| 並べ替えの王手と脅威の項 | Stockfishと同じく、静かな手の並べ替えの値に、王手になる手でSEE（駒の取り合いの損得の見積り）が`−trunc(75P/208)`（Stockfishの尺度で−75）以上なら`OrderingCheckBonus`を足し（`src/movepick.cpp:241-242`）、動かす駒より価値の低い相手の駒に狙われている升から出る手に加点、そうした升へ入る手に減点する（同244-247）。王手の判定は、Stockfishの`check_squares`と同じく直接の王手に限る。直接の王手とは、相手の王駒がちょうど1枚あり、現在の占有のまま到達升に着手後の駒種の駒を置くと、その駒の疑似利きが王駒に届く手であり、開き王手と王駒が2枚の場合は含めない。中将棋の動きは180度の回転と先後の入れ替えについて対称なので、駒種ごとの直接の王手の升の集合は、相手の色の同じ駒種の駒を王駒の升に置いたときの疑似利きに等しく、ノードで静かな手の段を開くときに、必要な駒種についてだけ求める。当初は規則どおりの`Position::gives_check`をすべての静かな手に使ったが、フェーズ1の測定でこの判定が探索時間の約19%を占めたので、Stockfishの形に改めた。SEEが規則に依存して判定できない手と、経由升を持つ手は、閾値以上として扱う。既存の枝刈りが判定できない手を飛ばさないのと同じ側である。Stockfishは価値の低い駒の集合を駒の格（歩兵、小駒、飛車、女王）で作り、キングには0を使う（同201-209）。中将棋は駒種が29あり、評価関数の駒価値は成駒を区別した駒の状態ごとに異なるので、相手の駒のうち盤上の駒コードの駒価値（`Nnue::piece_value`）が動かす駒より厳密に小さいものの疑似利きの和集合を価値の低い駒の脅威とし、王駒には0を使う。疑似利きは現在の占有での利きであり、獅子、角鷹、および飛鷲の2段階の到達範囲を含み、獅子の捕獲制限の合法性は調べない。利きはminase-coreの既存の利きの計算を呼ぶ公開の関数を`Position`に加えて求める。この集合はノードで静かな手の段を開くときに、相手の駒を価値の順に並べ、同じ価値の駒をまとめた累積の利きとして1回だけ求め、費用は探索速度の低下として測る。YaneuraOuは王手の加点だけを持ち、脅威の項を持たない（`movepick.cpp:371-388`）。 |
| 評価値の単位を持つ係数 | 2026年10月17日の利用者の決定により、評価値の単位を持つ係数の範囲は、Stockfishの換算値（歩兵価値208で割った値）、YaneuraOuの換算値（歩兵価値90で割った値）、およびminaseの現行の値をすべて含むように取る。本書で当たるのは2つである。`CaptureHistoryScale`（捕獲履歴が上限のときの並べ替えへの加算、歩兵価値に対する百分率）は、Stockfishが10692 × 100/(7 × 208) = 734.3を丸めて734、YaneuraOuが10692 × 100/(7 × 90)で1697、現行が285であり、開始値734は現行の約2.6倍である（Stockfish `src/movepick.cpp:225-226`、YaneuraOu `movepick.cpp:340-341`）。現行の換算は尺度を先に整数で求めるので、Stockfishの比とは丸めの分だけ違う。`OrderingThreatWeight`（動かす駒の価値が歩兵1枚分のときの脅威の項の大きさ）は、Stockfishが20×208で4160、YaneuraOuは項を持たず、現行は0である。 |
| continuation historyによる枝刈り | Stockfishと同じく、静かな手のうち、1手前と2手前のcontinuation historyとpawn historyの和が`−ContinuationPruning × d`を下回る手を読まない（`src/search.cpp:1229-1235`）。適用条件は起点の静かな手のfutility pruningと同じとし、前回の反復の主変化を辿るPVノードを除く（同1227行の`followPV`）。Stockfishと同じく、この枝刈りはlmrDepthを使わずに先に判定し、続いてlmrDepthにhistoryの項を足してからfutility pruningと静かな手のSEEによる枝刈りに渡す（同1229-1259）。王手になる手はこの枝刈りの対象外であり、起点と同じく王手の判定は安い条件がすべて成り立った後に行う。 |
| LMRの対象 | Stockfishと同じく、先頭の手を除くすべての手（捕獲手、killer手、成る手、および先頭以外に来た置換表の手）を減深の対象とし、根も対象にする（`src/search.cpp:1395`）。根の`search_root`は、F4の式で減深後の深さを求めて`search_move`へ渡す。根の式では、PVノードとttPvを真、cutNodeを偽とし、improvingは2手前の局面がないので偽から始めて起点の`static_eval >= β`の規則で更新し、子のβ打ち切りの数は`cutoff_counts[1]`、判定用の評価値は根の補正後の静的評価と置換表の照合結果から起点の`pruning_eval`で求める。Stockfishは根で並べ替えの先頭の手を置換表の手として扱うので（`src/search.cpp:839`）、根の置換表の手の項は並べ替え後の先頭の手について求め、置換表の値と深さの項は実際の照合の結果を使う。減深した探索の値がαを上回った後の深さの±1手の調整と再探索、負の減深量による延長、およびPVノードの1手の加算は、F4の規則のまま保つ。`search_move`は再探索の後の値を返すので、途中結果の採用（`RootResults`）には再探索の後の値だけが記録される。 |
| 根での更新 | Stockfishは根も同じ探索関数で読むので、根でも最善手がαを上げればhistoryを更新する。本書は`search_root`にも同じ更新を加える。根には直前の手がないので、直前の手を使う更新は行わない。 |
| 重みが0のときの処理 | 係数が0でも、表の参照と更新、および置換表の手の合法性の確認を省かない。省く分岐を置くと、フェーズ1で測る探索速度に、SPSAで係数が動いた後の費用が現れないからである。 |
| SPSAの対象 | 起点の表のすべての係数（`FmScale`を除く71係数）から、廃する4係数（`HistoryBonusScale`、`HistoryMalusScale`、`HistoryLimit`、`CaptureHistoryLimit`）を除き、新しい15係数を加えた82係数とする。新しい係数だけを調整する案は、既存の係数との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の係数を含むので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 既定の設定を流用する。3,750反復、1反復8ペア（計30,000ペア）、調整機での実行、持ち時間は測定機の10秒＋0.1秒を[統括計画](search-forms.md#持ち時間の換算)の方式で換算した値、同時対局数は[統括計画](search-forms.md#同時対局数とメモリ)が調整機ごとに決める数、`c_end`（最後の反復の摂動幅）は範囲の1/6、`r_end`（最後の反復の学習率）は0.002である（[SPSAの手引き](../guides/spsa.md)）。30,000ペアで82係数を十分に調整できるかは未検証である。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。最終値を最適値とは呼ばない。総ペア数は、[利得と規模のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)が定める標準の30,000ペアである。 |
| 新しい係数の範囲 | 2026年10月17日の利用者の決定により、範囲は開始値を含み、両エンジンの値と現行の値の両方を含むように取る。現行の値は、新しい仕組みがない状態（追加の更新の強さ、LMRの補正の強さ、王手と脅威の項は0、減点の逓減は1024、上限は表の上限）に当たる値とする。continuation historyによる枝刈りの閾値だけは、枝刈りがない現行の状態が閾値の無限大に当たるので、範囲に含められない。 |

メモリは、continuation historyの約140 MB、全ワーカーで共有するpawn historyの約17 MB×（ワーカー数以上の最小の2のべき）、およびワーカーごとのlow ply historyの約0.2 MBが増え、`Threads`が1なら1プロセスで約157 MBになる。
SPSAと対局測定は1局に2つのエンジンを動かすので、1局あたり約0.31 GB増える。
lishogiのBotは`Threads`を4としているので（`minase-lishogi-bot/config.yml`）、1局あたり約209 MB増え、同時対局数を掛けた分だけコンテナのメモリ上限の見直しが要る。
この確認は完了条件に含める。

## 変更の内容

以下の式で、`d`はノードの残り深さ（置換表の打ち切りの条件では要求深さ、それ以外はIIRの後の深さ）、`[条件]`は条件が成り立てば1、成り立たなければ0を表す。
IIR（internal iterative reduction）は、置換表に記録手がない残り深さ3以上のノードを1だけ浅く読む現行の規則であり、allNodeと前の反復の主変化を辿るノードには適用しない。適用しないノードではIIRの後の深さは要求深さに等しい。
表の値と更新量はStockfishの尺度で表す。
`trunc`は0の方向への丸めであり、重力式の除算も、それ以外の除算もStockfishと同じく整数の除算（0の方向への切り捨て）とする。
LMRの1/100手単位の減深量を手単位へ直す除算も、[探索の枝刈りとLMRの関数形の見直し](archived/search-pruning-forms.md)のF4と同じく0の方向へ丸める。
各節の見出しの順は、フェーズ1でコミットする順である。

### M0　探索経路の状態の追加（挙動を変えない移行）

`path_moves`の要素に、着手の記録として着手前の駒コードと、捕獲手なら取った駒種を加える。
`Searcher`に、plyごとのstatScore、ノードの手数、および置換表に局面があったかどうかの配列と、plyごとの歩兵と仲人の配置の鍵（pawn key）を加える。
statScoreはノードの開始時にそのplyを0にし、この段階ではほかの値を書かない。
ノードの手数は、禁止手の判定の後に、処理中の手について`index + 1`を書く。
置換表に局面があったかどうかは、ノードで置換表を引いた直後に書く。
pawn keyは根の局面から全計算し、`search_move`で`material_keys`と同じ位置で差分更新し、null moveでは親の値を写す。
補正後の静的評価は、起点の`static_evals`が保持している。
これらは記録するだけで探索に使わないので、探索ノード数は変わらない。

### M1　加点と減点の形と表の上限（history bonus/malus）

butterfly historyの上限を7183、捕獲履歴の上限を10692の定数とし、`HistoryLimit`と`CaptureHistoryLimit`を廃する。
`HistoryBonusScale`と`HistoryMalusScale`も廃し、`HistoryDecay`、`CaptureHistoryScale`、および`LmrHistoryThreshold`の開始値と範囲を「新しい係数」の節の2つ目の表のとおりに改める。
探索済みの手の列は、適用範囲の定義のとおり、ノードの手数が32以下で、読んだ時点でαを上回らなかった手だけを積む。
β打ち切りのノードで、加点`b`と減点`m`を次のように求める。

```math
b = \min(\mathrm{HistoryBonusSlope} \times d - \mathrm{HistoryBonusBase},\ \mathrm{HistoryBonusCap}) + \mathrm{HistoryTtMoveBonus} \times [\mathit{best} = \text{置換表の手}] + \mathrm{trunc}\left(\frac{s_{p-1}}{28}\right)
```

```math
m = \min(\mathrm{HistoryMalusSlope} \times d - \mathrm{HistoryMalusBase},\ \mathrm{HistoryMalusCap})
```

ここで`s_{p−1}`は、このノードへ至った手のstatScoreであり、根では0とする。
M3aでstatScoreを求めるまでは、`s_{p−1}`は0である。
零窓のノードでは、`b`に`trunc(b × (探索済みの静かな手の列の長さ + 探索済みの捕獲手の列の長さ)/256)`を足す。
最善手が静かな手なら、静かな手の表の更新で最善手に`trunc(b × 899/1024)`を与える。
続いて`m′ = trunc(m × 1159/1024)`とし、探索済みの静かな手の列を順にたどって、`m′ ← trunc(m′ × HistoryMalusDecay/1024)`としてから`−m′`を与える。
最善手が捕獲手なら、捕獲履歴の最善手の要素に`trunc(b × 1427/1024)`を与える。
最善手の種類によらず、探索済みの捕獲手の列の各手の捕獲履歴に`−trunc(m × 1489/1024)`を与える。
各表の更新は、更新量をその表の上限で切り詰めてから重力式を使う。
係数の範囲によって`b`や`m`が負になっても、符号つきのまま式を評価する。
この段階の静かな手の表の更新は、butterfly historyだけに行う。
killer表は、β打ち切りのときに現行どおり更新する。
新規対局などで表を初期化するときは、Stockfishと同じく、butterfly historyを−5、捕獲履歴を−742にする（`src/search.cpp:714-715`）。
根の探索ごとの減衰は、起点と同じくbutterfly historyと捕獲履歴の値に`HistoryDecay`と`CaptureHistoryDecay`の百分率を掛ける。

### M3a　continuation history、pawn history、low ply historyの新設と更新

continuation historyの表`C`とpawn historyの表を`HistoryTables`に置いて探索チームで共有し、low ply historyの表をワーカーの`Searcher`に置く。
ply `p`のノードで手`x`を引くとき、`x`を指す側を基準に、文脈の手`y = ply p − i`の記録から次の要素を引く。

```math
C\big[\,[y\ \text{が捕獲手}]\,\big]\big[\,[i\ \text{が偶数}]\,\big][k_y][\rho(t_y)][k_x][\rho(t_x)]
```

ここで`k`と`t`は着手前の駒種と到達升、`ρ`は`x`を指す側が後手なら升の番号`s`を`143 − s`へ回転し、先手ならそのまま返す写像である。
`i`が偶数の文脈の手は`x`と同じ側が指した手である。
文脈の手を持たない`i`の項は、参照も更新もしない。
fail lowの加点のように直前のplyの手の値を更新する場合は、その手を`x`とし、文脈の手はその手のplyから数え、回転もその手を指した側を基準にする。

静かな手の表の更新は、手`x`に量`u`を与えるときに次を行う（`src/search.cpp:2077-2089, 2049-2073`）。
butterfly historyの`x`の要素に`u`を与える。
根からのplyが5未満なら、low ply historyの要素に`trunc(u × 712/1024)`を与える。
continuation historyの更新は、手`x`に量`v`を与えるとき、`q = 0`から始めて`i`を1、2、3、4、6の順にたどり、文脈の手があれば、その要素の更新前の値が正なら`q`を1増やしてから`trunc(v × w_i × M_q/65536) + 73[i = 1]`を与える。
`w_i`は`(520, 390, 145, 251, 209)`、`M_q`は`(94, 103, 110, 106, 119, 126)`の`q`番目（0から数える）である。
`v`が0でも1手前の要素には73を与えるので、係数が0でも更新を省かない。
静かな手の表の更新では、`v = trunc(u × 750/1024)`としてこのcontinuation historyの更新を行う。
pawn historyの要素には、`u > −4`なら`trunc(u × 1104/1024)`、`u ≤ −4`なら`trunc(u × 459/1024)`を与える。
各表の更新は、更新量をその表の上限で切り詰めてから重力式を使う。
M1の静かな最善手と探索済みの静かな手への更新は、ここで静かな手の表の更新全体に広げる。

pawn historyは、歩兵と仲人の配置の鍵の下位ビットでエントリを選び、手番を含む駒と到達升で引く。
新規対局などの初期化では、continuation historyを−586、pawn historyを−1338にする（`src/search.cpp:717-727`）。
low ply historyは、探索ごとに`Searcher`を作る時点で102にする。

手を読む直前に、その手のstatScore`s`を求めて記録する。
静かな手では`s = trunc((2252 h_bf + 1126 c₁ + 1093 c₂)/1024)`、捕獲手では`s = trunc(873 V/128) + h_cap`とする。
`V`は取る駒の価値の合計`v`をStockfishの尺度へ換算した`trunc(v × 208/P)`、`h_cap`は捕獲履歴の値である。
文脈の手を持たない`c_i`は0とする。
statScoreは、LMRを適用するかどうかによらず、実際に読むすべての手について求める。

### M2　historyを更新する条件の拡大

M1の更新を、β打ち切りのノードだけでなく、最善値が探索開始時のαを上回ったすべてのノード（`best_score > original_alpha`）で行う。
この更新、反駁の減点、およびfail lowの加点は、手のループを終えて合法手がない場合の処理を通った後、correction historyを更新する前に行う。
次の場面でも更新する。

置換表の打ち切りの条件が成り立ち、置換表の値がβ以上で、置換表の手がある場合は、打ち切りの前に置換表の手の子局面を照合する処理より前に、次の2つを行う（`src/search.cpp:902-910`）。
置換表の手が現局面で合法な静かな手なら、その手に`HistoryTtCutoffBonus × d`を静かな手の表の更新で与える。
このノードへ至った手が静かな手で、その手のノードの手数が5未満なら、置換表の手が捕獲手でも、このノードへ至った手のcontinuation historyに−2210を与える。

前の最善手が反駁されたときは、M1の更新を行うノードで、このノードへ至った手が静かな手であり、その手のノードの手数が「1 + 置換表に局面があったか」に等しければ、その手のcontinuation historyに`−trunc(m × 713/1024)`を与える（`src/search.cpp:2031-2035`）。

fail lowのとき、すなわち最善値が探索開始時のα以下だったときは、このノードへ至った手に次の加点をする（`src/search.cpp:1611-1643`）。
その手が静かな手か捕獲手かで、次のどちらか一方を行う。
その手が静かな手なら、`e`をこのノードの静的評価、`e_{p−1}`をその手のノードの静的評価、`s_{p−1}`をその手のstatScore、`n_{p−1}`をその手のノードの手数として、評価値をStockfishの尺度に換算し、次の量を求める。

```math
k = \max\left(0,\ -241 - \mathrm{trunc}\left(\frac{s_{p-1}}{98}\right) + \min(59d, 420) + 186[n_{p-1} > 9] + 142[v \le e - 106] + 159[v \le -e_{p-1} - 68]\right)
```

```math
B = \min(150d - 85, 1337) \times k
```

ここで`v`はこのノードの最善値である。
その手のcontinuation historyに`trunc(B × 263/16384)`、その手を指した側のbutterfly historyに`trunc(B × HistoryPriorBonus/32768)`を与える。
その手が歩兵と仲人の手でも成る手でもなければ、その手のpawn historyに`trunc(B × 324/8192)`を与える。
その手が捕獲手なら、上の加点は行わず、捕獲履歴のその手の要素に892を与える。

静的評価の変化による更新は、このノードへ至った手が静かな手なら、補正後の静的評価を求めた直後、reverse futility pruningなどの前向き枝刈りより前に行う（`src/search.cpp:1002-1012`）。

```math
\delta = \mathrm{clamp}\left(-(e_{p-1} + e),\ -189,\ 194\right) + 60
```

`e`と`e_{p−1}`は、Stockfishと同じく補正後の静的評価（correction historyの補正を足した値）をStockfishの尺度に換算したものである。
その手を指した側のbutterfly historyに`δ × HistoryEvalDiff`を与え、置換表に局面がなく、その手が歩兵と仲人の手でも成る手でもなければ、pawn historyに`δ × 13`を与える。

LMRで減深した探索の値がαを上回ったときは、必要な零窓の再探索を終えた後、全窓の再探索の前に、その手のcontinuation historyへ1334を与える（`src/search.cpp:1423`）。
零窓の再探索を行わなかった場合も与え、再探索が中断した場合は与えない。
この加点は静かな手の表の更新を通らないので、750/1024を掛けない。

直前の手を使う更新は、根とnull moveの直後のノードでは行わない。
根の子のノードでは、根の着手の記録を使って通常どおり行う。
`search_null_move_verification`は同じplyで`negamax`を呼び直すので、本書が加えるplyごとの状態（statScore、ノードの手数、置換表に局面があったか）も、起点の静的評価とttPvと同じく退避して戻す。
検証探索の中で更新した表の値は戻さない。

### M3b　並べ替えの値と王手と脅威の項

静かな手の段の並べ替えの値は次のとおりとする。

```math
V(x) = 2h_{\mathrm{bf}}(x) + 2h_{\mathrm{pawn}}(x) + \sum_{i \in \{1,2,3,4,6\}} c_i(x) + [p < 5] \cdot \mathrm{trunc}\left(\frac{8 h_{\mathrm{low}}(p, x)}{1 + p}\right) + \mathrm{OrderingCheckBonus} \times g(x) + \mathrm{trunc}\left(\frac{v_x \times \mathrm{OrderingThreatWeight}}{P}\right) \times (a_{\mathrm{from}}(x) - a_{\mathrm{to}}(x))
```

`c_i`は文脈の手を持つ`i`だけを足す。
`g(x)`は、`x`が設計判断の表の「並べ替えの王手と脅威の項」で定義する直接の王手で、`see_prunes(x, trunc(75P/208))`が偽なら1、それ以外は0である。
`v_x`は動かす駒（着手前の駒）の駒価値、`P`は探索用の歩兵価値である。
`a_from(x)`と`a_to(x)`は、移動元と到達升が、動かす駒より駒価値の小さい相手の駒の疑似利きにあれば1、なければ0であり、動かす駒が王駒なら0とする。
同点破り（線形経路の差分、生成順）は変えない。

### M4　LMRと枝刈りでのhistoryの使用（statScore、continuation history based pruning）

LMRの減深量と減深後の深さは、[探索の枝刈りとLMRの関数形の見直し](archived/search-pruning-forms.md)のF4の式を正本とし、本書はF4の`r₀`にM3aのstatScore`s`の項を加えるだけである。

```math
q_{\mathrm{hist}} = \mathrm{trunc}\left(\frac{s \times \mathrm{LmrHistoryScale} \times 100}{4096 \times 1024}\right), \qquad r_0' = r_0^{\mathrm{F4}} - q_{\mathrm{corr}} - q_{\mathrm{hist}}
```

ここで`r₀^F4`はF4の`r₀`の全項、`q_corr`は後続の[correction historyの設計書](correction-history-tables.md)のC5が加える補正項であり、本書の実装の時点ではこの項はなく、`r₀′ = r₀^F4 − q_hist`である。
`q_hist`の積は64ビットの整数で計算し、`4096 × 1024`で1回だけ割る。
各項は個別に0方向へ丸めてから足す。
`r₀′`をF4の`r₀`の代わりに使い、allNodeの拡大、S9の詰みを探す局面での調整、0方向への丸めによる手単位の`R`、および減深後の深さ`d′`（負の減深量による延長とPVノードの1手の加算を含む）は、F4とS9の規則のまま求める。
`q_hist`をallNodeの拡大の前に引くのは、Stockfishと同じ順序である（`src/search.cpp:1381, 1386-1388`）。
F4の段差`h`は、M5で対象に加わる手のうち静かな手（killer手と静かな置換表の手を含む）ではbutterfly historyの値で判定し、捕獲手では0とする。

静かな手の枝刈りでは、起点の静かな手のfutility pruningと同じ適用条件（前回の反復の主変化を辿るPVノードを除く）の下で、まず`c₁ + c₂ + h_pawn < −ContinuationPruning × d`の手を読まない。
文脈の手を持たない`c_i`は0として足す。
この判定はlmrDepthを使わない。
王手の判定と王駒への利きは、起点の静かな手のfutility pruningと同じく、この条件が成り立った後に調べ、王手になる手はこの枝刈りの対象外とする。
続いて、枝刈りの対象として残った手について`H = c₁ + c₂ + h_pawn + trunc(69 h_bf/32)`を求め、枝刈り用のlmrDepthに`trunc(H / D(d))`を足す。
`D(d) = 3000 + 7 × (min(d, 16) − 8)²`である（`src/search.cpp:55-58, 1236-1240`）。
足した後のlmrDepthを、起点の静かな手のfutility pruningとSEEによる枝刈りへ渡す。
futility pruningには負の値のまま渡し、SEEによる枝刈りでは0以上に切り詰めてから2乗する。

### M5　LMRの対象の拡大と根での更新

M4の減深量を、置換表の手、捕獲手、およびkiller手を除く静かな手に限らず、先頭の手を除くすべての手に適用する。
根の`search_root`でも、残り深さ2以上で、先頭の手を除く各手の減深量を求め、減深後の深さとその手を読む直前の最善値を`search_move`へ渡す。
根の式の各項は、設計判断の表の「LMRの対象」のとおりに求める。
根でも、最善値が探索開始時のαを上回ればM1の更新を、探索済みの手の列を同じ定義で集めて1回行う。
根には直前の手がないので、M2の直前の手を使う更新は行わない。
根には新しい前向き枝刈りもkiller手の更新も加えない。

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
| `HistoryMalusDecay`（探索済みの静かな手1つごとの減点の倍率、1/1024） | 921 | 700〜1024 | Stockfishは921（`src/search.cpp:2021`）、YaneuraOuは977（`yaneuraou-search.cpp:5327`）。現行は逓減しない1024 |
| `HistoryTtCutoffBonus`（置換表の打ち切りでの加点の深さ1あたりの量） | 131 | 0〜300 | Stockfishは131（`src/search.cpp:906`）、YaneuraOuは`min(130d − 71, 1043)`の傾き130（`yaneuraou-search.cpp:2739-2740`）。現行は0 |
| `HistoryTtMoveBonus`（最善手が置換表の手のときの加点の上乗せ） | 364 | 0〜1000 | Stockfishは364（`src/search.cpp:2005`）、YaneuraOuは353。現行は0 |
| `HistoryPriorBonus`（fail lowの直前の手へのbutterfly historyの加点、1/32768） | 215 | 0〜600 | Stockfishは215（`src/search.cpp:1630`）、YaneuraOuは235（`yaneuraou-search.cpp:4336`）。現行は0 |
| `HistoryEvalDiff`（静的評価の変化によるbutterfly historyの更新の倍率） | 11 | 0〜40 | Stockfishは11（`src/search.cpp:1008`）、YaneuraOuは10（`yaneuraou-search.cpp:3129`）。現行は0 |
| `ContinuationPruning`（continuation historyによる枝刈りの深さ1あたりの閾値） | 4136 | 1000〜12000 | Stockfishは4136（`src/search.cpp:1234`）、YaneuraOuは4097（`yaneuraou-search.cpp:3653`）。枝刈りがない現行は閾値の無限大に当たり、範囲に含められない |
| `LmrHistoryScale`（statScoreによる減深の減少、1/4096） | 439 | 0〜1000 | Stockfishは439（`src/search.cpp:1381`）、YaneuraOuは428（`yaneuraou-search.cpp:3937`）。現行の連続量はないので0を含める |
| `OrderingCheckBonus`（王手になる手への並べ替えの加点） | 16384 | 0〜32768 | Stockfishは16384（`src/movepick.cpp:242`）、YaneuraOuも16384（`movepick.cpp:371`）。現行は0 |
| `OrderingThreatWeight`（動かす駒の価値が歩兵1枚分のときの脅威の項、並べ替えの値） | 4160 | 0〜12000 | Stockfishは20×208で4160（`src/movepick.cpp:246-247`）。YaneuraOuは項を持たず、現行は0 |

既存の係数のうち、次の3つは開始値または範囲を置き直す。

| 係数 | 起点の値 | 開始値 | 範囲 | 根拠 |
|---|---|---|---|---|
| `HistoryDecay`（根の探索ごとに残すbutterfly historyの割合、百分率） | 32 | 71 | 0〜100 | Stockfishは729/1024で71（`src/search.cpp:343`）。YaneuraOuは減衰させず100に当たる。範囲は起点のまま |
| `CaptureHistoryScale`（捕獲履歴が上限のときの並べ替えへの加算、歩兵価値に対する百分率） | 285 | 734 | 0〜2000 | Stockfishは734、YaneuraOuは1697、現行は285（設計判断の表の「評価値の単位を持つ係数」）。開始値は現行の約2.6倍である。起点の範囲0〜400はStockfishの値を含まないので広げる |
| `LmrHistoryThreshold`（段差の閾値、Stockfishの尺度の履歴点） | 9 | 15 | 0〜2000 | Stockfishに対応する値はない。表の上限を7183へ改めるので、`round(起点の値 × 7183 / 起点のHistoryLimit)`から始める（起点の9と4384なら15）。表の値の水準が上がるので、起点の範囲0〜512の上限を広げる |

### 定数

次の72個の値は、Stockfish（コミット5c4cef0b）の値を定数として写す。
評価値の閾値は、Stockfishの尺度へ換算した評価値と比べる。

| 用途 | 値 | Stockfishの行 |
|---|---|---|
| 表の上限 | butterfly history 7183、捕獲履歴10692、continuation history 30000、pawn history 8192、low ply history 7183 | `src/history.h:128-146` |
| 初期値 | butterfly history −5、捕獲履歴−742、continuation history −586、pawn history −1338、low ply history 102（根の探索ごと） | `src/search.cpp:339, 714-727` |
| 加点の追加項 | 直前の手のstatScoreの1/28、零窓のノードで読んだ手の数の1/256 | `src/search.cpp:2005-2011` |
| 探索済みの手の列 | ノードの手数の上限32 | `src/search.cpp:75, 1578` |
| 最善手と探索済みの手への比 | 静かな最善手899/1024、静かな手の減点1159/1024、捕獲の最善手1427/1024、捕獲手の減点1489/1024 | `src/search.cpp:2015-2042` |
| 直前の手への減点 | 置換表の打ち切りで−2210（直前の手のノードの手数が5未満）、反駁で`−m × 713/1024` | `src/search.cpp:909-910, 2033-2035` |
| fail lowの加点 | M2の式の定数、continuation history 263/16384、pawn history 324/8192、捕獲履歴892 | `src/search.cpp:1614-1643` |
| LMRの後の加点 | continuation history 1334 | `src/search.cpp:1423` |
| 静的評価の変化 | 切り詰め−189と194、加算60、pawn historyの倍率13 | `src/search.cpp:1006-1011` |
| 静かな手の表の更新 | low ply history 712/1024、continuation history 750/1024、pawn history 1104/1024（−4以下の更新では459/1024） | `src/search.cpp:2083-2089` |
| continuation historyの更新 | 文脈ごとの重み(520, 390, 145, 251, 209)/1024、正の要素の数による倍率(94, 103, 110, 106, 119, 126)/64、1手前に73 | `src/search.cpp:2050-2070` |
| 並べ替え | butterfly historyとpawn historyの2倍、continuation historyの5文脈の和、low ply historyの`8/(1 + ply)`倍とその深さ5、王手の加点のSEEの閾値−75 | `src/movepick.cpp:233-251`、`src/history.h:42` |
| statScore | 静かな手`(2252, 1126, 1093)/1024`、捕獲手の駒の価値873/128 | `src/search.cpp:1371-1378` |
| lmrDepthへの項 | butterfly history 69/32、除数`3000 + 7(min(d, 16) − 8)²` | `src/search.cpp:55-58, 1236-1240` |

## 所要時間の見積り

時間切れの事前確認、SPSA、事前対局、およびSTCは調整機で、反映とテストは測定機で行う。
SPSAの所要時間は、[統括計画](search-forms.md#所要時間と費用)の見積りにより、物理192コアの調整機で同時対局数191なら約13〜18時間である。
1反復の時間は持ち時間で決まるので、係数の数が増えても、表の参照で探索速度が下がっても、大きくは変わらない。
ただし本書は1局のメモリを約0.31 GB増やすので、メモリの少ない調整機では同時対局数が下がり、その分だけ所要時間が延びる。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 形の変更の実装と検証（codexへ委任） | 2日程度 |
| 2 | 時間切れの事前確認（調整機） | 約35分 |
| 3 | SPSA（調整機、30,000ペア） | 約13〜18時間 |
| 4 | 反映とテスト（測定機）、事前対局（調整機、32ペア） | 約1時間 |
| 5 | STC（調整機） | 約0.2〜1.3時間 |

測定機の占有は、反映とテストの約1時間である。

## 実装フェーズ

基本シードは、時間切れの事前確認を3800000000、セッションを3850000000、事前対局を3900000000、STCを3950000000とする。
これらは着手時に、保存済みの測定とSPSAの基本シード（`data/matches/`と`data/spsa/`の`manifest.json`）、および他の設計書が予約した値から、各測定のペア数以上離れた未使用の値であることを確かめた（[教訓](../lessons/derive-seed-adjacent-collision.md)）。
予約した4000000000はLTCを行わないので使わない。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ1　形の変更の実装

起点からブランチ`history-forms`を切り、worktree `data/worktrees/history-forms`で作業する。
M0、M1、M3a、M2、M3b、M4、M5の順に、1項目を1コミットにする。
表の新設と更新（M3a）を更新の場面を広げるM2より前に入れ、M2の各条件を完成した表で検証できるようにする。並べ替えの変更（M3b）はM2の後に分ける。
M0のコミットでは、benchの全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が起点と一致することを確かめる。
M1からM5の各コミットでは、同じbenchのノード数を記録し、直前のコミットとの差を残す。
新しい係数は、その係数を使う項目のコミットで`params.rs`の表へ追加する。廃する4係数の削除と、置き直す3係数の開始値と範囲の変更はM1で行う。
`taskset -c 3`で1つのコアに固定した`minase dev bench --depth 9 --repetitions 3`の中央値を、起点とM3bのコミット、および起点と最後のコミットで交互に3回ずつ測り、探索速度の変化を記録する（[教訓](../lessons/pin-bench-to-performance-core.md)）。
王手の判定と脅威の集合の費用はM3bで初めて現れるので、M3bで探索速度が大きく下がった場合は、その時点で対策の案を利用者に示す。
同じ実行で最大常駐メモリも記録し、増分がcontinuation historyの約140 MBとpawn historyの約17 MBに見合うことを確かめる。
調整用ビルド（`--features tuning`つきのビルド）が新しい係数を`Tune_<係数名>`のUSIオプションとして宣言し、廃した係数を宣言しないことを確かめる。
新しい式の単体テストは、式の写しではなく数値の例で書き、係数を引数で受け取る純粋な関数に固定の係数を渡して確かめる（[テストの手引き](../guides/testing.md)）。
着手の記録とcontinuation historyのテストは、null moveがそのplyの記録を空にすること、文脈の手を持たない項は参照も更新もしないこと、1手前がnull moveでも2手前以降の記録があればその項を参照すること、根のノードでは文脈の手がないこと、後手の手で升を180度回転して引くこと、fail lowの加点では1つ前のplyの手を指した側を基準に回転すること、成る手でも着手前の駒種で引くこと、記録から着手後の駒を導けること、および更新後の値が表の上限の範囲に収まることを確かめる。
pawn historyのテストは、通常の移動、成り、2枚取り、および居喰いの後に、差分更新した鍵が局面から計算し直した鍵と一致することを確かめる。
探索済みの手の列のテストは、ノードの手数が32を超えた手と、読んだ時点でαを上回った手が列に入らないことを確かめる。
並べ替えのテストは、価値の低い駒の脅威の集合が駒価値の順の累積の疑似利きと一致すること、王駒の手では脅威の項が0であること、駒種ごとの集合による直接の王手の判定が到達升に駒を置いた参照の判定と一致すること、相手の王駒が2枚なら加点しないこと、SEEが閾値を下回る王手には加点しないこと、およびSEEが判定できない王手には加点することを確かめる。
M2のテストは、置換表の手が不合法なら置換表の手への加点をしないこと、置換表の手が捕獲手でも直前の手への減点を行うこと、置換表の値がβ未満の打ち切りでは更新しないこと、直前の手が捕獲手かnull moveなら静かな手への加点をしないこと、およびnull moveの検証探索の後にplyごとの状態が戻ることを確かめる。
LMRのテストは、`q_hist`がallNodeの拡大の前に引かれること、残り深さ2のノードでも負の減深量による延長とPVノードの1手の加算がF4のとおり保たれること、および根の途中結果に再探索の後の値だけが記録されることを確かめる。
最後のコミットの後に、調整係数の既定値を範囲の1/3と2/3に置いた一時的な`params.rs`で`cargo test`（通常ビルドと調整用ビルド）を走らせ、係数の既定値に依存して落ちるテストをSPSAの前に直す。直し方は、係数を固定の値に設定してから確かめるか、係数によらず結果が決まる局面で確かめる形に限る。
`Threads`の上限256を受理するUSIのテストは表を約4.6 GB確保するので、`cargo test --workspace`の所要時間と最大常駐メモリを1回記録する。
完了条件は、`cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、および`cargo test -p minase --lib --features search-stats,tuning`の通過、ノード数の一致と変化の記録、および探索速度とメモリの記録である。

### フェーズ2　パラメーターファイルと時間切れの事前確認

形の変更の最後のコミットでビルドした`minase`を`data/spsa/history-forms-runner/`へ複製し、以後の再ビルドで変わらないrunner（測定を実行するプログラム）として固定して、以後のフェーズはこのrunnerで実行する。
ただし、時間切れの事前確認とSPSAのセッションは[SPSAの手引き](../guides/spsa.md)の調整機の手順で実行し、調整機では同じコミットからビルドしたrunnerを使う。
`minase spsa params --engine commit:<形の変更> --rules engine-default`で既定のパラメーターファイルを生成し、`FmScale`の行を削除したファイルを`data/spsa/history-forms.params`とする。
開始値と範囲が形の変更の`params.rs`の表と一致すること、および`c_end`と`r_end`が既定値であることを、使い捨てのスクリプトで機械的に突き合わせる。

続いて、手引きの第3節に従い、時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても時間切れが起きないことを確かめる。
写しの開始値を`HardRemainingShare`＝50、`HardSoftRatio`＝800、`MinMoves`＝40、`IterationRatio`＝150へ書き換えた`data/spsa/history-forms-safety.params`で、[統括計画](search-forms.md#実行の構成)が定める反復数（同時対局数191なら156反復、1,248ペア。同時対局数は調整機ごとに決まる）を調整機で実行する。

```console
<調整機のminase> spsa \
  --run-dir data/spsa/history-forms-safety --seed 3800000000 \
  --engine commit:<形の変更> --params data/spsa/history-forms-safety.params \
  --rules engine-default --each <換算した持ち時間> --concurrency <調整機の同時対局数> \
  --iterations <事前確認の反復数> --pairs-per-iteration 8
```

`time_forfeits`（時間切れの件数）が1件でもあれば、[デバッグの手引き](../guides/debugging.md#時間切れと時間管理)の手順で原因を調べ、該当する係数の範囲を狭める案を利用者に示してから本番へ進む。
結果は測定記録`history-forms-safety`に残す。
完了条件は、時間切れとエンジン異常が0件であることと、測定記録の保存である。

### フェーズ3　SPSAによる調整

```console
<調整機のminase> spsa \
  --run-dir data/spsa/history-forms --seed 3850000000 \
  --engine commit:<形の変更> --params data/spsa/history-forms.params \
  --rules engine-default --each <換算した持ち時間> --concurrency <調整機の同時対局数> \
  --iterations 3750 --pairs-per-iteration 8
```

このコマンドは調整機で実行し、実行ディレクトリは測定機の`data/spsa/history-forms`へ取得してから反映に使う。
最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`history-forms`に、手引きの第6節の項目とともに残す。
最終値を丸めた整数が範囲の上限または下限になった係数を示す。
新しい15係数と置き直した3係数については、最終値が開始値からどれだけ離れたかを、`c_end`に対する比で示す。
この比は、どの仕組みが調整で動いたかを見る目安であり、採否には使わない。
完了条件は、セッションの完了と測定記録の保存である。

### フェーズ4　反映と事前対局

手引きの第7節の手順1に従い、`minase spsa apply`で最終値を表の既定値へ書き込み、`git diff`で既定値だけが変わったことを確かめてコミットする。
続いて`cargo test`（通常ビルドと調整用ビルド）を測定より先に実行し、表の既定値に依存するテストが失敗した場合は、フェーズ1の末尾と同じ形でテストを直して別のコミットにする。
テストの失敗を、末尾に`; true`を付けるようなコマンドの組み方で隠さない。
候補の調整用ビルドの宣言を`minase spsa params`で出力し、既定値の欄が`apply`の出力と一致することを確かめる。

STCの前に、候補が時間切れやエンジン異常を起こさないことと、時間の使い方の変化を確かめるため、STCの条件で候補と起点の32ペアの事前対局を行う。
事前対局とSTCは、[統括計画](search-forms.md)の設計判断「対局の実行場所」により、SPSAを終えた調整機で、持ち時間を換算せずに行う。候補のコミットは`git bundle`で調整機のcloneへ渡し、`<固定したminase>`は調整機でrunnerのコミットからビルドしたminaseとする。同時対局数は、SPSAと同じ調整機の同時対局数とする。実行ディレクトリは測定機の`data/matches/`へ取得してから集計し、記録する。

```console
<固定したminase> match run \
  --run-dir data/matches/history-forms-smoke --seed 3900000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency <調整機の同時対局数> elo --pairs 32
```

候補と起点の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ（反復深化で読み終えた深さ）、および探索を止めた理由の分布を、前の設計書の事前対局と同じ方法で集計して、測定記録`history-forms-smoke`に残す。
同じ記録に、候補と起点の探索速度を前述のbenchで測った値も残す。
完了条件は、テストの通過、事前対局での時間切れとエンジン異常が0件であること、および測定記録の保存である。

### フェーズ5　STC

2026年10月18日の利用者の決定により、本書の変更はSTCの判定によらず採用する（[統括計画](search-forms.md)の設計判断「各サブタスクの採否」）。
STCは、統括計画のフェーズ10で強くなっていなかった場合に、どのサブタスクで後退したかの手がかりにするための記録として行い、LTCは行わない。
STCは、事前対局と同じ調整機で、同じ同時対局数で行う。

```console
<固定したminase> match run \
  --run-dir data/matches/history-forms-stc --seed 3950000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency <調整機の同時対局数> gsprt --max-pairs 3000
```

逐次検定の停止規則は[SPRTの手引き](../guides/sprt.md)のSTCのとおりとし、判定は記録するが採否には使わない。
採用はLTCの`H1`だけが決めるという同手引きの原則の例外であり、統括計画の[採用の例外のADR](../adr/adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)に従う。
結果は測定記録`history-forms-stc`に残す。
完了条件は、STCの判定と測定記録の保存である。

## 検証

- M0のコミットで、benchの全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。
- M1からM5の各コミットで、benchのノード数の変化が記録されていること。
- 探索速度と最大常駐メモリの変化が記録されていること。
- 新しい15係数が調整用ビルドの`Tune_`オプションとして宣言され、廃した4係数が宣言されず、パラメーターファイルの開始値と範囲が表と一致すること。
- 候補の`params.rs`の差分が既定値の書き換えだけであり、調整用ビルドの宣言の既定値が`apply`の出力と一致すること。
- `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、および`cargo test -p minase --lib --features search-stats,tuning`が通過すること。
- 既定値を範囲の1/3と2/3に置いた表でも、テストが通過すること。
- 時間切れの事前確認、SPSA、事前対局、およびSTCで、エンジン異常、時間切れ、および拒否着手が0件であること。最初の測定で1件でも出た場合は、測定を止めて原因を調べる。

## 完了条件

- 形の変更の検証結果、時間切れの事前確認、SPSAの結果、事前対局、およびSTCが、測定記録に残っている。
- 候補のブランチがmasterへ統合され、依存関係の節に挙げたADRが新しいADRに置き換えられている。統合は利用者の許可を得てから行う。
- lishogiのBotを更新する前に、Botのコンテナのメモリ上限が、増えるメモリを含めて足りることを確かめる。
- ROADMAPの状態表と統括計画の「状態」が本書の結論と一致している。

## 参考資料

- [探索と時間管理の係数の関数形に関する既存実装との比較](../research/search-coefficient-forms-prior-art.md)。
- [手の順序付けの改善](archived/move-ordering.md)と、counter move historyとcontinuation historyの再診断を含む[診断の記録](../measurements/move-ordering/move-ordering-diag.md)。
- 根の探索ごとに初期化する表での発動率の記録（[counter move history](../measurements/strength-stage5/strength-stage5-cmh-bench.md)、[continuation history](../measurements/strength-stage5/strength-stage5-continuation-bench.md)）と、その表の設計を記した[棋力向上段階5](archived/strength-stage5.md)。
- Stockfish（コミット5c4cef0b）の`src/search.cpp`。lmrDepthの除数は55〜58行、low ply historyの初期化は339行、butterfly historyの根ごとの減衰は341〜343行、文脈の表の設定は688〜700行、表の初期化は713〜727行、探索済みの手の列の上限は75行と1578行、静的評価の変化による更新は1001〜1012行、置換表の打ち切りでの更新は902〜910行、文脈の表の配列は1140〜1142行、continuation historyによる枝刈りとlmrDepthへの項は1227〜1240行、statScoreとLMRの補正は1371〜1381行、LMRの適用条件は1395行、LMRの後のcontinuation historyの更新は1423行、最善手があるときの更新は1602〜1608行、fail lowの加点は1611〜1643行、加点と減点は2004〜2042行、continuation historyの更新は2049〜2073行、静かな手の表の更新は2077〜2089行。`src/movepick.cpp`の静かな手の並べ替えは233〜251行。`src/history.h`の重力式は70〜76行、表の型と上限は128〜146行、共有の表は198〜225行。`src/types.h`の歩兵価値は199行。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`。low ply historyの初期化は1540行、置換表の打ち切りでの更新は2735〜2748行、静的評価の変化による更新は3126〜3132行、continuation historyによる枝刈りは3646〜3661行、LMRのhistory補正は3925〜3937行、fail lowの加点は4311〜4339行、加点と減点は5308〜5350行。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)、[SPRTによる棋力測定の手引き](../guides/sprt.md)、および[SPSAの反復予算の調査](../research/spsa-iteration-budget.md)。
