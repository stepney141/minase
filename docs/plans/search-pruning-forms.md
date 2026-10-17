# 探索の枝刈りとLMRの関数形の見直しの設計書

## 先に読む要約

本書は、探索の前向き枝刈り（読む価値が低いと見込んだ手やノードを読まずに済ませる枝刈り）と減深の式を、チェスのStockfishの形へ寄せ、現行のmasterより強くなるかを確かめる。
[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)によれば、minaseの探索の係数の多くはStockfishや将棋のYaneuraOu（やねうら王）と違う形で使われている。
その差は、係数の値をSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で選び直しても埋まらない。
2026年10月17日の利用者の決定により、Stockfishにある項目は入れ、式の形や値はStockfishに合わせ、係数はStockfishの値を換算した値から始める。
中将棋の診断や測定で決めたminaseの既存の仕組みは残し、Stockfishの仕組みを並べて足す。
まずF0として、ノードの種別の予測であるcutNodeとallNodeの区別を、探索の再帰呼び出しの引数として子へ渡す。
cutNodeはβ打ち切りが見込まれるノード、allNodeはどの子もβ打ち切りを起こさないと見込まれるノードである。
F1は、reverse futility pruning（静的評価がβを十分に上回るノードを読まずに返す枝刈り）をStockfishの形にする。
F2は、静的評価とcorrection history（静的評価のずれを学習して補正する手法）の補正の使い方をStockfishの形にする。
F3は、late move pruning（後方の静かな手を読まない枝刈り）の上限に、improving、つまり静的評価が同じ手番の2手前より上がっている状態の項を加える。
F4は、late move reductions（LMR、後方の手を浅く読む減深）の減深量をStockfishの項で組み直す。
F5は置換表による打ち切りと内部反復減深（IIR）にノードの種別を使い、F6は詰みを探す局面での調整としてStockfishの`seekMate`を入れる。
探索を変えない移行のコミットは探索ノード数の一致で確かめ、探索を変えるコミットは探索ノード数の変化を記録する。
新しい係数を既存の係数とまとめてSPSAで調整し、起点のmasterと短時間条件STCおよび長時間条件LTCの2段階の逐次検定（段階ゲート）で対局させ、1つの候補として採否を決める。
完了条件は、候補の採否が測定記録とともに確定することである。

## 状態

起案。2026年10月17日に起案した。
着手は利用者の指示による。
次の一手は、利用者の指示を受けて起点を確定し、フェーズ1（形の変更の実装）を始めることである。

## 目的

本書が確かめるのは、「枝刈りと減深の式をStockfishの形へ寄せ、新しい形の係数を調整すれば、現行の形で調整した探索より強くなる」という仮説である。
発端は、2026年10月17日に現行の形のまま探索と時間管理の38係数をNNUEに合わせてSPSAで調整し直したとき、`ReverseFutilityMargin`（歩兵価値に対する百分率）が開始値428から下限0まで動いたことである。
この調整は、本書と後続の設計書で全係数を調整し直すので、反映せずに中止した。
現行のreverse futility pruningは、深さ1〜3に共通の1つの余裕値を補正前の静的評価に当てる形なので、深さ1と深さ3の余裕値の比を1以外にできない。
Stockfishの余裕値は深さに比例して増え、improvingのときは補正量の項を除いて深さ2以下で0以下になる（`src/search.cpp:1022-1033`）。
現行の形では、全深さの余裕値を一律に0へ近づけることでしかこの挙動に近づけないので、SPSAが係数を下限まで動かしたのはそのためである可能性がある。
形を変えて係数を調整し直せば、浅い深さと深い深さで別の余裕値を選べるようになる。
LMRも同じ事情にあり、Stockfishの減深量はノードの種別、置換表の情報、および子の打ち切りの数に依存するが、minaseの減深量はそれらに依存しない。

## 適用範囲

本書の用語を次のように定める。

- **起点**は、フェーズ1を始める時点のmasterの先頭のコミットである。
- **形の変更**は、本書の項目を実装したコミット列であり、新しい係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で表（`crates/minase/src/search/alphabeta/params.rs`）の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **PVノード**は、探索窓（αからβまでの区間）の幅が1より大きいノードである。窓の幅が1のノードを零窓のノードという。
- **cutNode**は、親がβ打ち切りを見込んで零窓で読ませる子のノードであり、**allNode**は、PVノードでもcutNodeでもないノードである。どちらもF0が定める受け渡しの規則で決まる予測であり、実際にβ打ち切りが起きるかとは別である。
- **静的評価**は、F2の後は、NNUEの出力に補正を足して評価関数の出力の範囲に収めた値を指す。各ノードで1回だけ求め、手数ごとに保持する。
- **判定用の評価値**は、静的評価を、条件を満たす置換表の値で置き換えた値である。Stockfishの`eval`に当たり、定義はF2に書く。
- **improving**は、静的評価が同じ手番の2手前の局面より上がっていることをいう。
- **opponentWorsening**は、現局面の静的評価が、1手前の局面の静的評価の符号を反転した値より大きいことをいう。1手前の局面は相手の手番なので、相手から見た評価が直前の手で悪くなったことを表す。
- **ttPv**は、PVノードであるか、置換表のエントリがPVの印を持つことをいう。PVの印はF1の準備のコミットが置換表に加える。
- **詰み帯と専用の帯**は、詰みまでの手数を表す評価値の範囲と、千日手などの反復の結果を表す評価値の範囲（`crates/minase/src/search/mod.rs`の`MATE_THRESHOLD`と`REPETITION_BAND_START`）である。通常の評価値はこれらの外にある。
- **探索を変えない移行**は、F0、F1の準備、およびF4の準備の3コミットである。F1の準備は置換表へPVの印を記録し、F4の準備はLMRの単位を改めて上限を係数にする。
- 減深量と深さの単位は、探索の深さ1を1手と数える。

対象は次の7項目である。

- cutNodeとallNodeの区別の導入（F0）。
- reverse futility pruningの余裕値の形と条件（F1）。
- 静的評価、補正、improving、および判定用の評価値の求め方（F2）。
- late move pruningのimprovingの項と既存の係数の開始値（F3）。
- late move reductionsの減深量の項、減深後の深さ、および再探索の深さ（F4）。
- 零窓のノードでの置換表による打ち切りの条件と、IIRを行わない条件（F5）。
- 詰みを探す局面での調整（F6）。

次の事項は対象外とする。

- LMRのstatScore（historyの値による増減）の項、continuation history、LMRの捕獲手と根への拡大、および根でのhistoryの更新。[history-forms.md](history-forms.md)で扱う。
- futility pruning、late move pruning、およびSEE（static exchange evaluation、駒の取り合いの損得の見積り）による枝刈りの深さの拡張と余裕値の形、razoringの形、およびnull move pruningを試す条件。[shallow-pruning-extensions.md](shallow-pruning-extensions.md)で扱う。
- reverse futility pruningの返り値をβと判定用の評価値の重み付き平均にすること（Stockfish `src/search.cpp:1033`）。[shallow-pruning-extensions.md](shallow-pruning-extensions.md)が扱い、F1は返り値の混合を加えない。
- 王手になる手の判定と、それを使う項。王手の判定のための前計算は[shallow-pruning-extensions.md](shallow-pruning-extensions.md)が導入する。このため、詰みを探す局面で王手にならない手の減深を増やすStockfishの項（`src/search.cpp:1391-1392`）も、本書では入れずにshallow-pruning-extensions.mdが扱う。
- null move pruningの減深量の既存の係数（`NullMoveBase`、`NullMoveSlope`、`NullMoveEvalScale`）の開始値をStockfishの換算値へ置き直すこと。[shallow-pruning-extensions.md](shallow-pruning-extensions.md)が扱う。
- LMRの補正量の項（Stockfish `src/search.cpp:1353`）。[correction-history-tables.md](correction-history-tables.md)のC6が扱う。
- 置換表に保存した静的評価を通常探索で使うこと（Stockfish `src/search.cpp:856-860`）。[qsearch-forms.md](qsearch-forms.md)が扱う。
- 静止探索、aspiration windows、および時間管理の形。それぞれ[qsearch-forms.md](qsearch-forms.md)、[aspiration-window-forms.md](aspiration-window-forms.md)、および[time-instability-scaling.md](time-instability-scaling.md)で扱う。
- 王駒に相手の利きが届くノードを枝刈りから除く条件。中将棋の規則から来る条件であり、2026年10月17日の利用者の決定により残す（[ADR](../adr/skip-static-eval-pruning-when-a-royal-is-attacked.md)）。

## 依存関係

起点は、フェーズ1を始める時点のmasterの先頭であり、起点の表の既定値を、Stockfishに同じ形の値がない既存の係数のSPSAの開始値とする。
本書は、探索の関数形をStockfishの形へ寄せる一連の設計書の最初であり、後続の設計書は本書の完了後のmasterを起点とする。
連鎖の順序は提案であり、最終的な順序は利用者が決める。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと計算資源を共用するので、同時対局数は16を上限とする。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

後続の設計書とは、次のように分担する。
F0が導入するcutNodeとallNode、およびF1の準備が導入するttPvは、後続の設計書がそれぞれの範囲で使う。
reverse futility pruningの余裕値の補正量の項はF1が持ち、[correction-history-tables.md](correction-history-tables.md)は持たない。
[shallow-pruning-extensions.md](shallow-pruning-extensions.md)が使う減深後の深さ（lmrDepth）の基になる減深量は、F4の`r_pre`とする。

本書は次のADRの決定の一部を変える。
採用した場合は、完了時に、起点の時点で有効なADRを確かめたうえで新しいADRで置き換え、置き換えた旧ADRを`docs/adr/archive/`へ移す。決定の一部だけを変える場合は、残る決定も新しいADRへ引き継ぐ。

- [correction historyの鍵と適用先のADR](../adr/key-correction-history-by-material.md)は、補正をfutility pruningの判定にだけ使うと決めた。根拠となった表の持ち越しの診断は、補正をfutility pruningだけに使う前提で測ったものであり、適用先の拡大は測っていない。F2は、補正をノードごとに1回読み、静的評価を使うすべての判定に使う。
- [improvingのADR](../adr/use-improving-only-for-futility-margins.md)は、improvingの判定に補正前の静的評価を使い、2手前がない場合と比較の区間にnull moveがある場合をimprovingとみなし、LMRの減深量をimprovingで変えないと決めた。F2は補正後の静的評価で比べ、例外をStockfishの扱いに合わせる。F4はLMRにimprovingの項を加える。
- [浅いノードの前向き枝刈りの適用条件のADR](../adr/skip-static-eval-pruning-when-a-royal-is-attacked.md)は、reverse futility pruningを残り深さ3以下に限ると決めた。F1は上限をStockfishと同じ残り深さ18にし、ttPvと置換表の手の条件を加える。王駒への利きによる除外は変えない。
- [late move reductionsの式のADR](../adr/reduce-lmr-by-log-table-with-stepped-history-adjustment.md)は、上限を3とし、それを超える案を棄却した。同じADRは、この判断をSTCの到達深さが5〜6だった時期のものとしている。F4は上限を係数として残し、Stockfishの項を加え、負の減深量による延長を許す。
- [late move pruningの数え方のADR](../adr/count-only-quiet-moves-for-late-move-pruning.md)は、上限式にimprovingの項を含めず、静かな手だけを数えると決めた。F3は数え方を残し、両エンジンの上限式`(3 + d²)/(2 − improving)`（Stockfish `src/search.cpp:1199`、YaneuraOu `yaneuraou-search.cpp:3593`）に合わせてimprovingの項を加え、既存の係数をStockfishの値から始める。
- [null moveと置換表の打ち切りをPVノードでも行うADR](../adr/allow-null-move-and-tt-cutoff-at-pv-nodes.md)は、置換表の値による打ち切りをノードの種別によらずに行うと決めた。F5は、PVノードでの打ち切りを残したうえで、零窓のノードにStockfishの打ち切りの条件を当てる。F6は、詰みを探す局面でnull moveの減深を半分にする。
- [記録手のないノードを1手浅く読むADR](../adr/reduce-depth-by-one-without-tt-move.md)は、要求深さ3以上で記録手がなければ1手浅く読むと決めた。F5は、この規則を残したうえで、allNodeと前の反復の主変化を辿るノードでは行わない条件を加える。

## 設計判断

各行の決定のうち、2026年10月17日の利用者の決定によるものはその旨を書く。それ以外は、利用者の決定と先例のADRに基づく本書の判断である。

| 項目 | 決定 |
|---|---|
| 候補の束ね方 | 全項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回の段階ゲートで採否を決める。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。項目ごとにSPSAと段階ゲートを繰り返す案は、1項目あたりSPSAの約75時間にSTCとLTCを加えた約81〜113時間を要するので採らない。代償として、項目ごとの寄与は測らない。不採用のときに一部の項目だけを選び直して再測定しない。 |
| 合わせる先 | 2026年10月17日の利用者の決定により、両エンジンで形や値が違う場合はStockfishに合わせる。YaneuraOuにだけある項目は入れず、Stockfishにある項目は入れる。YaneuraOuの値は、新しい係数の表に参考として併記する。 |
| 残す既存の仕組み | 2026年10月17日の利用者の決定により、minaseにあってStockfishにない仕組みのうち、中将棋の診断や測定で決めたものは残し、Stockfishの仕組みを並べて足す。本書の範囲では、LMRの減深量の上限（係数`LmrMaxReduction`）、静かな手だけを数えるlate move pruning、成る手を枝刈りから外す保護、PVノードでの置換表の打ち切り、およびIIRの深さ3の条件を残す。王駒への利きの除外は中将棋の規則から来るので残す。 |
| 新しい係数の開始値 | 2026年10月17日の利用者の決定により、Stockfishの値をminaseの単位へ換算した値とする。評価値の単位を持つ値はStockfishの歩兵価値208（`src/types.h:199`）に対する百分率へ、Stockfishの1/1024手の減深量は1/100手へ、1/1024などの比は百分率へ換算し、整数に四捨五入する。両エンジンに対応する値がない`LmrMaxReduction`は、現行と同じ3から始める。両エンジンには減深量の上限がなく、換算する元がないからである。 |
| 既存の係数の開始値 | 2026年10月17日の利用者の決定により、本書の範囲の既存の係数のうちStockfishに同じ形の値があるものは、換算値を開始値に置き直す。`LmrDivisor`は156から203へ、`LmpBase`は382から300へ、`LmpSlope`は133から100へ移る。その他の既存の係数は、起点の表の値から始める。 |
| 新しい係数の範囲 | 2026年10月17日の利用者の決定により、範囲は開始値、両エンジンの値、および現行の値をすべて含むように取る。評価値の単位を持つ係数では、Stockfishの換算値（歩兵価値208で割る）、YaneuraOuの換算値（歩兵価値90で割る）、およびminaseの現行の対応する値の3つを含める。現行に対応する項がない係数では、項が働かない値を現行の値とする。SPSAの最後の反復の摂動幅は範囲の1/6、最初の反復はその約2.30倍（3,750^0.101、`crates/minase/src/bin/minase/spsa_runner/model.rs`）なので、開始値が下限に近い係数では序盤の摂動が下限で切り詰められる。[利得のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)はこの切り詰めを許容しており、係数ごとの摂動幅の設定は加えない。 |
| 開始値が他の値から遠い係数 | 評価値の単位を持つ係数の3つの値は次の表に並べる。`ReverseFutilityMultBase`は、開始値22がYaneuraOuの84の約1/4、現行の428の約1/19であり、開始値が他の2つから最も遠い。固定する`mult`の上限41も、現行の428の約1/10である。YaneuraOuは同じ形の定数を将棋向けに調整し直しており、歩兵価値で見るとStockfishの2〜4倍になることが多く、minaseの現行の値もその大きさに近い。最終値はminaseのSPSAが決める。 |
| SPSAで調整する値と固定する値 | 2026年10月17日の利用者の決定により、SPSAで調整するのは、余裕値の切片と傾き、重み、および倍率のように各仕組みの強さを決める値だけとする。上下限、比べる回数の閾値、深さの条件などはStockfishの値で固定し、固定の定数の表にまとめる。ただし、LMRの減深量の上限は残す既存の仕組みなので係数として調整する。 |
| 検証の方法 | 2026年10月17日の利用者の決定により、探索を変えない移行の3コミットだけは、`minase dev bench`の全15局面を深さ3、5、7で探索して、探索ノード数、評価値、および最善手が直前のコミットと一致することを確かめる。探索を変えるコミットでは、同じbenchの探索ノード数の直前のコミットからの変化を記録する。開始値で現行と同じ探索になることは求めない。 |
| cutNodeとallNodeの区別（F0） | 2026年10月17日の利用者の決定により、探索の再帰でcutNodeを受け渡し、Stockfishの受け渡しの規則に従う。F0は区別を受け渡すだけで、F0のコミットではどの判定も区別を使わないので、探索ノード数の一致で検証できる。Stockfishは`search`のテンプレート引数でPVノードを区別するが、minaseは窓の幅で区別する。このため、窓の幅が1になったPVノードを、StockfishはPVノードのまま扱って先頭の子をPVノードとして読み、minaseは零窓のノードとして扱って子に零窓の規則を当てる。この差は起こる頻度が低いので、規則を変えて合わせることはしない。 |
| SPSAの対象 | 起点の調整対象の38係数（`FmScale`を除く）から廃する`ReverseFutilityMargin`を除き、新しい30係数を加えた67係数とする。新しい係数だけを調整する案は、既存の係数との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の6係数を含めるので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | [SPSAの手引き](../guides/spsa.md)の既定の設定を流用する。3,750反復、1反復8ペアの計30,000ペアを、持ち時間10秒＋0.1秒、同時対局数16で行う。最後の反復の摂動幅`c_end`は範囲の1/6、最後の反復の学習率`r_end`は0.002である。30,000ペアは候補を1つ作るための予算であり、最終値を最適値とは呼ばない。総ペア数は、[利得と規模のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)が定める標準の30,000ペアである。30,000ペアで67係数を十分に調整できるかは未検証であり、不採用はこの予算と設定で改善を示さなかったことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。 |
| reverse futility pruningの余裕値の形（F1） | 2026年10月17日の利用者の決定により、Stockfishの`mult × depth − (2789 × improving + 335 × opponentWorsening) × mult/1024 + std::abs(correctionValue)/198435`と`mult = min(45 + 4 × depth, 85) − 20 × [置換表にない]`の形（`src/search.cpp:1025-1030`）を、歩兵価値で換算した係数で表す。`mult`の定数は評価値の単位なので歩兵価値に対する百分率へ換算する。improvingとopponentWorseningの項は`mult`の倍数なので、`mult`に対する百分率へ換算する。補正量の項は、`correctionValue`を131072で割ったものが評価値の補正量なので（`src/search.cpp:116-117`）、補正量の絶対値に対する百分率66（131072/198435）へ換算する。`mult`の上限85は上限なので、歩兵価値の41%に固定する。深さの傾き4/208は1.92%を2%に丸めるので約4%ずれるが、SPSAで調整するので許容する。 |
| reverse futility pruningの条件（F1） | 2026年10月17日の利用者の決定により、Stockfishの条件（`src/search.cpp:1022-1023`）に合わせる。「判定用の評価値 ≥ β」を独立の条件として持つ。余裕値はimprovingの項で負になり得るので、この条件がないとβ未満の値を返して打ち切ることになる。ttPvでないノードに限り、置換表の手がないか、置換表の手が捕獲手であるノードに限る。深さの上限はStockfishの`depth < (seekMate ? 6 : 19)`と同じく、残り深さ18以下、詰みを探す局面では5以下に固定する。Stockfishはこの値を詰みの発見のために調整しない値としている（`src/search.cpp:1021`）。αとβが詰み帯と専用の帯の外であること、および王駒に利きが届かないことという現行の条件は残す。前者はStockfishの`!is_loss(beta) && !is_win(eval)`に当たる。返り値は判定用の評価値とし、βとの混合はshallow-pruning-extensions.mdが加える。 |
| ttPvの導入（F1の準備） | 2026年10月17日の利用者の決定（Stockfishにある項目は入れる）により、置換表のエントリにPVの印を加え、F1とF4で使う。印は、エントリの2つの64ビット値のうち`critical`の予約領域（58〜63ビット）の1ビットに置くので、エントリの大きさ16バイトは変わらない。置換表へ保存する3か所（`negamax.rs`、`root.rs`、`quiesce.rs`）に印を渡し、手数ごとのttPvを探索器に持つ。印の付け方はStockfishに従う（`src/search.cpp:842, 1651-1652, 1657, 1741, 1908`）。印を読む判定はF1とF4だけなので、印を記録するコミットは探索を変えず、探索ノード数の一致で検証する。 |
| 静的評価の求め方（F2） | 2026年10月17日の利用者の決定により、Stockfishと同じく補正をノードごとに1回だけ読み（`src/search.cpp:832`）、通常探索の各ノードの最初に静的評価を1回求めて手数ごとに保持する（同846-870行）。静的評価を使う判定（reverse futility pruning、razoring、null move pruning、futility pruning、late move pruning、およびLMR）は、すべてこの値を使う。現行はfutility pruningの判定の時点で補正を読み直しているが、これもノードの最初の値に揃える。根も同じく静的評価を求めて保持し、子のopponentWorseningの比較に使う。現行は浅い枝刈りの対象などに限って静的評価を求めているので、費用は探索速度（NPS、1秒あたりの探索ノード数）の低下として測る。置換表の保存値、静止探索、および補正表の更新に使う差（探索値と補正前の評価の差）は変えない。 |
| improvingとopponentWorsening（F2） | 2026年10月17日の利用者の決定により、improvingの例外をStockfishの扱いに合わせる。Stockfishは`ss->staticEval > (ss - 2)->staticEval`で比べ（`src/search.cpp:883`）、2手前がない根の近くでは比較の相手を`VALUE_NONE`（32002、同309行）とするのでimprovingは偽になる。null moveの区間にも例外を置かない。minaseも、手数2未満ではimprovingを偽とし、null moveの例外を廃する。opponentWorseningは`ss->staticEval > -(ss - 1)->staticEval`（同884行）に合わせ、保持した1手前の静的評価と比べる。null moveの前後は同じ盤面なので、null moveの直後のノードでは、1手前の値はnull moveをした局面の静的評価になり、Stockfishと同じ比較になる。null move pruningの後、静的評価がβ以上ならimprovingを真にする（Stockfish `src/search.cpp:1080`）。 |
| 判定用の評価値（F2） | 2026年10月17日の利用者の決定により、reverse futility pruning、razoring、およびLMRのαとの差の項には、Stockfishの`eval`と同じく、置換表の値が静的評価より良い方向の下界か悪い方向の上界なら置換表の値を使う（`src/search.cpp:862-865`）。置換表の値は、詰み帯と専用の帯の外にある場合に限って使う。専用の帯の値は経路に依存するので打ち切りに使わない現行の規則に揃え、詰み帯の値を返り値に持ち込まないためである。これは本書の判断である。 |
| LMRの減深量の単位（F4の準備） | 減深量を1/100手単位の整数で計算し、最後に100で割る。現行の表は`floor(ln d × ln m/(LmrDivisor/100))`の整数を持つので、定数項や倍率を足すと、表の段階で切り捨てた端数が失われる。表に`floor(100x)`（xは切り捨て前の値）を持てば、整数`h`について`floor((floor(100x) + 100h)/100) = floor(x) + h`が成り立つので、単位の変更だけのコミットは探索を変えず、探索ノード数の一致で検証できる。両エンジンは1/1024手単位を使うが（Stockfish `src/search.cpp:1402-1403`）、minaseの表の係数は百分率で揃えているので1/100手とする。 |
| LMRの項（F4） | 2026年10月17日の利用者の決定（Stockfishにある項目は入れる）により、Stockfishの減深量の項のうち、statScoreの項と補正量の項を除くすべてを入れる（`src/search.cpp:1183-1192, 1345-1388`）。ttPvの項、手番号の項、置換表の手が捕獲の項、子のβ打ち切り数の項、αと判定用の評価値の差の項、cutNodeとallNodeの項がこれに当たる。Stockfishが置換表の手の減深を減らす項（同1368-1369行）は、minaseが置換表の手をLMRの対象にしないので働かず、係数を置かない。減深量の前半`r_pre`（同1183-1192行）は、shallow-pruning-extensions.mdのlmrDepthの基にもなるので、後半と分けて定める。 |
| LMRの減深後の深さ（F4） | 2026年10月17日の利用者の決定（Stockfishにある項目は入れる）により、Stockfishの`max(1, newDepth + min(-r/1024, ply < 2 × rootDepth ? 2 : 0)) + PvNode`（`src/search.cpp:1402`）に合わせる。負の減深量による延長を、根からの手数が反復の深さの2倍未満なら2手まで許し、PVノードでは減深後の深さに1を足す。減深の上限`LmrMaxReduction`は残す既存の仕組みなので、正の減深量の上限としてだけ掛け、延長には掛けない。減深した探索がαを超えたら、Stockfishと同じく、その値が最善値より十分に高ければ1手深く、最善値に届かなければ1手浅く、通常深さを調整してから再探索する（同1414-1420行）。 |
| LMRをしない手の減深（F4） | 2026年10月17日の利用者の決定（Stockfishにある項目は入れる）により、LMRの分岐を通らない零窓の探索でも、置換表の手がなければ減深量に`LmrNoTtMove`を足し、減深量が大きければ通常深さから1手または2手減らして読む（`src/search.cpp:1428-1436`）。Stockfishがこの分岐を通るのは、零窓のノードの先頭の手と残り深さ1のノードの2手目以降だけなので、minaseも同じ手に限る。LMRの対象でない2手目以降の手（捕獲手とkiller手）は、Stockfishでは減深の分岐で読まれる手なので、この分岐には含めず、LMRを広げる[history-forms.md](history-forms.md)のM5に委ねる。 |
| LMRのcutNodeの増分と上限の釣り合い（F4） | cutNodeの増分の開始値は約3.9手、置換表の手がなければ約4.8手であり（`src/search.cpp:1356-1357`）、Stockfishでは多くの減算の項と釣り合っている。上限`LmrMaxReduction`の開始値3では、cutNodeで減深の対象となる手の多くが上限で減深される。2026年10月17日の利用者の決定により上限は係数として残すので、上限と増分の釣り合いはSPSAに選ばせる。上限に張り付いた最終値は、測定記録に残して解釈する。 |
| allNodeの項の分母（F4） | Stockfishの`r × 276/(256 × depth + 268)`（`src/search.cpp:1387-1388`）の分子を係数`LmrAllNode`とし、分母の定数268/256は105に換算して固定する。2026年10月17日の利用者の決定（Stockfishにある項目は入れる）により入れる。 |
| 置換表の打ち切り（F5） | 2026年10月17日の利用者の決定（Stockfishにある項目は入れる）により、零窓のノードでは、置換表の値による打ち切りに「cutNodeで値がβ以上か、allNodeで値がβ未満か、残り深さが4より深い」というStockfishの条件（`src/search.cpp:899`）を加える。同じ決定により、打ち切りの深さの条件（同894-895行）と、残り深さ7以上で置換表の手を指した後の局面を置換表で確かめる処理（同917-931行）も入れる。PVノードでの打ち切りは、[ADR](../adr/allow-null-move-and-tt-cutoff-at-pv-nodes.md)が測定で決めた既存の仕組みなので残し、現行の条件のままとする。50手規則で打ち切りを止める条件（同915行）は、中将棋に50手規則がないので持たない。着手後の局面の値が専用の帯にある場合は、経路に依存する値を打ち切りに使わない現行の規則に揃えて、値がないものとして扱う。これは本書の判断である。 |
| IIR（F5） | 2026年10月17日の利用者の決定（Stockfishにある項目は入れる）により、allNodeではIIRを行わない（`src/search.cpp:1085`）。深さ3以上の条件は、[ADR](../adr/reduce-depth-by-one-without-tt-move.md)が測定で決めた既存の仕組みなので残し、Stockfishの深さ6には変えない。同じ決定により、前の反復の主変化を辿るノードでもIIRを行わない条件（Stockfishの`followPV`、同791-794行と1085行）を入れる。根は主変化を辿るノードとし、子は、親がそうであり、親から子への手が前の反復で確定した主変化の同じ手数の手と一致するときに限って、主変化を辿るノードとする。 |
| 詰みを探す局面（F6） | 2026年10月17日の利用者の決定（Stockfishにある項目は入れる）により、Stockfishの`seekMate`（`src/search.cpp:395`）を入れる。前の反復で確定した根の評価値の絶対値が、歩兵価値の`361 + 105769/D²`%以上なら詰みを探す局面とする。ここで`D`は反復の深さであり、2つの定数はStockfishの750と220000を歩兵価値208で換算した値である。Stockfishはこの値を棋力のための値ではないとして調整しない（同394行）ので、固定する。詰みを探す局面では、reverse futility pruningの深さの上限を5にし、razoringを行わず（同1017行）、null moveの減深を半分にする（同1045行）。 |

## 変更の内容

以下の式で、`P`は評価関数の重みファイルが持つ探索用の歩兵価値、`d`はIIRを適用した後の残り深さ、`c`はノードの最初に読んだ補正の値、`e`は判定用の評価値、`[条件]`は条件が成り立てば1、成り立たなければ0を表す。

### F0　cutNodeとallNodeの区別

`negamax`と`search_move`に、そのノードがcutNodeかを表す引数を加える。
根は`cutNode`を偽とするPVノードであり（Stockfish `src/search.cpp:412`）、`search_root`は偽を`search_move`へ渡す。
子に渡す値は、Stockfishの受け渡し（`src/search.cpp:1048, 1395-1455`）に合わせて次のとおりとする。

| minaseの呼び出し | 子に渡す値 | Stockfishの対応箇所 |
|---|---|---|
| null moveの探索（`negamax.rs`の`search_null_move`） | 偽 | 1048行 |
| 先頭の手で親がPVノードの場合（`search_move`の`first`の分岐） | 偽 | 1455行 |
| 先頭の手で親が零窓の場合（同じ分岐） | 親の値の否定 | 1428-1436行 |
| 2手目以降の零窓の探索で、親の残り深さが2以上の場合 | 真 | 1395行、1405行 |
| 2手目以降の零窓の探索で、親の残り深さが1の場合 | 親の値の否定 | 1428-1436行 |
| 減深した探索がαを超えた後の、調整した通常深さでの零窓の再探索 | 親の値の否定 | 1410-1420行 |
| 全窓の再探索 | 偽 | 1442-1455行 |

Stockfishは残り深さ2以上の2手目以降をすべて減深の分岐で読み、減深量が0以下でも子をcutNodeとする。
minaseの2手目以降の零窓の探索は減深しない手も同じ呼び出しで読むので、親の残り深さで分ける。
razoringが呼ぶ静止探索は区別を持たない（Stockfish 1018行）。
allNodeは、零窓でcutNodeでないノードとする（Stockfish 746行）。
`invariants`機能つきのビルドでは、PVノードがcutNodeでないことを検査する（Stockfish 766行の`assert`）。
F0のコミットでは、どの判定もこの引数を読まない。

### F1　reverse futility pruning

F1は2つのコミットで入れる。
1つ目のコミット（F1の準備）は、置換表のエントリにPVの印を記録する。
ノードのttPvは、PVノードであるか、置換表の照合に成功したエントリが印を持つ場合に真とする（Stockfish `src/search.cpp:842`）。
通常探索のノードは、最善値が入力時のα以下なら、1手前のノードのttPvとの論理和をとってから印として保存する（同1651-1657行）。
良い手が見つからなかったノードでは直前の相手の手が良い手である見込みが高いので、その局面を主変化の木に加える、というのがStockfishの注釈の説明である。
根はPVノードなので印を立てて保存する。
静止探索は、照合したエントリの印をそのまま保存する（同1741行、1908行）。
このコミットはどの判定も印を読まないので、探索を変えない。

2つ目のコミットで、余裕値を次の式にする。
`ReverseFutilityMultBase`、`ReverseFutilityMultSlope`、`ReverseFutilityNoTtHit`、および固定の上限41は歩兵価値に対する百分率、`ReverseFutilityImproving`と`ReverseFutilityOpponentWorsening`は`mult`に対する百分率、`ReverseFutilityCorrection`は補正の値の絶対値に対する百分率である。

```math
\mathrm{mult}(d) = \min(\mathrm{ReverseFutilityMultBase} + \mathrm{ReverseFutilityMultSlope} \times d,\ 41) - \mathrm{ReverseFutilityNoTtHit} \times [\text{置換表にない}]
```

```math
\mathrm{margin}(d) = \frac{P \times \mathrm{mult}(d) \times (100d - \mathrm{ReverseFutilityImproving} \times [\text{improving}] - \mathrm{ReverseFutilityOpponentWorsening} \times [\text{opponentWorsening}])}{10000} + \frac{\mathrm{ReverseFutilityCorrection} \times |c|}{100}
```

reverse futility pruningは、次の条件をすべて満たすときに行い、判定用の評価値`e`を返す。
返り値は置換表へ保存しない現行の扱いを変えない。

- 残り深さが18以下であり、詰みを探す局面（F6）では5以下である。
- ttPvでない。PVノードはttPvなので、零窓のノードに限られる。
- 置換表の手がないか、置換表の手が捕獲手である。置換表の手は検証されていない助言手なので（`tt.rs`の`Hit::best_move`の契約）、`MoveGenerator::is_legal_move`で合法手であることを確かめてから捕獲かを判定し、合法手でなければ置換表の手がないものとして扱う。`Hit::best_move`の契約は、この用途を許すように書き改める。
- αとβが詰み帯と専用の帯の外にある。
- 王駒に相手の利きが届かない。
- `e`がβ以上であり、余裕値を引いてもβ以上である。

`[置換表にない]`は、`TranspositionTable::probe`が照合に成功しなかったことを指す。
Stockfishは静的評価だけのエントリも保存するので（`src/search.cpp:873-874`）、minaseの「置換表にある」はStockfishより狭い。
`ReverseFutilityMargin`は廃し、深さの上限は`pruning.rs`の定数とする。

### F2　静的評価、補正、improving、および判定用の評価値

通常探索の各ノードと根で、置換表を照合した後に、累算器から求めたNNUEの出力にノードの補正を足し、評価関数の出力の範囲（`crates/minase/src/eval/nnue/`の`EVALUATION_LIMIT`）に収めて静的評価とする。
補正はこの時点で1回だけ読み、余裕値の補正量の項にも同じ値を使う。
静的評価は手数ごとに保持し、improvingは保持した2手前の値と、opponentWorseningは保持した1手前の値と比べる。
手数が2未満のノードではimprovingを偽とする。
null move pruningの判定の後、静的評価がβ以上ならimprovingを真にする。
この更新はfutility pruning、late move pruning、およびLMRのimprovingに及ぶ。

判定用の評価値`e`は、置換表の照合に成功し、その値が詰み帯と専用の帯の外にあり、値が静的評価より大きくて下界か正確な値、または静的評価以下で上界か正確な値であれば置換表の値、それ以外は静的評価とする。
`e`は、reverse futility pruning、razoring、およびLMRのαとの差の項に使う。
null move pruning、futility pruning、improving、およびopponentWorseningは、Stockfishと同じく静的評価を使う。

### F3　late move pruning

improvingでないとき、静かな手の上限`(LmpBase + LmpSlope × d²)/100`に`LmpNonImproving/100`を掛けて切り捨てる。
improvingのときの上限は変えない。
開始値50はStockfishとYaneuraOuの`/(2 − improving)`に当たり、improvingでないときの上限を半分にする。
既存の`LmpBase`と`LmpSlope`はStockfishの`3 + d²`（`src/search.cpp:1199`）に合わせて300と100から始める。
静かな手だけを数える数え方と深さの上限は変えない。

### F4　late move reductions

F4も2つのコミットで入れる。
1つ目のコミット（F4の準備）は、表を1/100手単位に改め、上限の定数3を係数`LmrMaxReduction`にする。
`LmrMaxReduction`の開始値は現行と同じ3なので、このコミットは探索を変えない。
2つ目のコミットで、1/100手単位の減深量を次のように求める。
前半`r_pre`はStockfishの1183〜1192行に当たり、後半はStockfishの1345〜1388行に当たる。

```math
r_{\mathrm{pre}} = \left\lfloor \frac{t \times (100 + \mathrm{LmrNonImproving} \times [\text{improvingでない}])}{100} \right\rfloor + \mathrm{LmrBase} - \mathrm{LmrPvReduction} \times [\text{PVノード}] + \mathrm{LmrTtPv} \times [\text{ttPv}]
```

```math
\begin{aligned}
r_0 = r_{\mathrm{pre}} &+ \mathrm{LmrOffset} - \mathrm{LmrMoveCount} \times (m + 1) + 100h \\
&- \left(\mathrm{LmrTtPvBase} + \mathrm{LmrTtPvPv} \times [\text{PVノード}] + \mathrm{LmrTtPvValue} \times [v > \alpha] + (\mathrm{LmrTtPvDepth} + \mathrm{LmrTtPvDepthCutNode} \times [\text{cutNode}]) \times [d_{\mathrm{tt}} \ge d]\right) \times [\text{ttPv}] \\
&+ (\mathrm{LmrCutNode} + \mathrm{LmrCutNodeNoTtMove} \times [\text{置換表の手がない}]) \times [\text{cutNode}] + \mathrm{LmrTtCapture} \times [\text{置換表の手が捕獲手}] \\
&+ (\mathrm{LmrCutoffCount} + \mathrm{LmrCutoffCountMany} \times [k > 2] + \mathrm{LmrCutoffCountAllNode} \times [\text{allNode}]) \times [k > 1] \\
&+ \left\lfloor \frac{\mathrm{LmrAlphaEval} \times \mathrm{clamp}(\alpha - e,\ -0.31P,\ 0.46P)}{P} \right\rfloor \times [\alpha \text{が詰み帯と専用の帯の外}]
\end{aligned}
```

```math
r = r_0 + \left\lfloor \frac{r_0 \times \mathrm{LmrAllNode} \times [\text{allNode}]}{100d + 105} \right\rfloor
```

ここで`t`は表の値`floor(100 × ln d × ln m/(LmrDivisor/100))`、`m`は0から数えた手番号であり、`m + 1`がStockfishの`moveCount`に当たる。
`h`は現行のhistory補正であり、butterfly historyの値が`LmrHistoryThreshold`以上なら−1、`−LmrHistoryThreshold`以下なら+1、それ以外なら0である。
`v`と`d_tt`は置換表の値と深さであり、照合に失敗したときは`[v > α]`と`[d_tt ≥ d]`を0とする。
`k`は子の手数のノードで起きたβ打ち切りの数であり、Stockfishの`(ss + 1)->cutoffCnt`（`src/search.cpp:829, 1562`）と同じく、手数ごとの数をノードの最初に2手先の分だけ0に戻し、ノードがβ打ち切りを起こすたびに1を足して数える。
置換表の手が捕獲手かの判定は、F1と同じく合法手であることを確かめてから行う。
`clamp`の上下限は、Stockfishの−64と96（`src/search.cpp:1384`）を歩兵価値208で換算した歩兵価値の−31%と46%である。

減深した探索の深さは、`r/100`を0方向へ丸めた整数`R`から次のように求める。
`E`は、根からの手数が反復の深さの2倍未満なら2、それ以外なら0である。

```math
d' = \max\left(1,\ d - 1 - \min(\max(R, -E),\ \mathrm{LmrMaxReduction})\right) + [\text{PVノード}]
```

LMRは、残り深さ2以上のノードで、対象の手（置換表の手、捕獲手、およびkiller手を除く非捕獲手）に適用する。対象の手の範囲と、根でLMRを行わないことは変えない。
LMRを捕獲手、killer手、および根へ広げるのは、[history-forms.md](history-forms.md)のM5である。
減深した零窓の探索の値がαを超えたら、通常深さ`d − 1`を、`d' < d − 1`かつ値が最善値より歩兵価値の`LmrDeeperMargin`%を超えて高ければ1手深く、値が最善値に歩兵価値の`LmrShallowerMargin`%を足した値より低ければ1手浅く調整する。
調整した深さが`d'`より深ければ、その深さで零窓の再探索を行う。
全窓の再探索も、調整した深さで行う（Stockfish `src/search.cpp:1414-1455`）。
既存の`LmrDivisor`は、Stockfishの表の積`reductions[d] × reductions[mn]`（`src/search.cpp:82, 1918`）が約0.49 × ln d × ln m手であることから、除数203から始める。
Stockfishの手番号は1から数えるので、表の手番号はminaseと1つずれるが、表の引き方は変えない。

LMRの分岐を通らない零窓の探索、つまり零窓のノードの先頭の手と、残り深さ1のノードの2手目以降の手では、同じ式で`r`を求める。
このとき`h`は、置換表の手、捕獲手、およびkiller手のいずれでもない非捕獲手なら現行どおりとし、それ以外の手では0とする。
置換表の手がなければ`r`に`LmrNoTtMove`を足し、`r`が511を超えれば通常深さ`d − 1`から1手、さらに`r`が536を超えて`d − 1`が2より深ければもう1手減らして読む（Stockfish `src/search.cpp:1428-1436`）。

### F5　置換表の打ち切りとIIR

零窓のノードでは、置換表の値`v`と深さ`d_tt`による打ち切りを、Stockfish（`src/search.cpp:894-934`）に合わせて次の条件をすべて満たすときに行う。

- `v`が専用の帯の外にあり、`d_tt > d − [v ≤ β]`である。
- 値の種別が値の向きと合う。つまり、`v ≥ β`なら下界か正確な値、`v < β`なら上界か正確な値である。
- cutNodeで`v ≥ β`か、allNodeで`v < β`か、残り深さが4より深い。
- 残り深さが7以上で、合法手であることを確かめた置換表の手があり、`v`が詰み帯の外にある場合は、その手を指した局面を置換表で引き、値がないか、`v ≥ β`と「その値の符号を反転した値 ≥ β」が一致する。

PVノードの打ち切りは現行の条件のまま変えない。
IIRを行う条件として、現行の「要求深さ3以上で置換表の手がない」に、allNodeでないことと、前の反復の主変化を辿るノードでないことを加える。
主変化を辿るかは手数ごとに保持し、前の反復で確定した主変化の写しと照らして決める（Stockfish `src/search.cpp:791-794`）。

### F6　詰みを探す局面

反復深化の各反復の開始時に、前の反復で確定した根の評価値`s`と反復の深さ`D`から、詰みを探す局面かを決める。

```math
\mathrm{seekMate} = \left[\, |s| \ge \frac{P \times (361 + 105769 / D^2)}{100} \,\right]
```

詰みを探す局面では、reverse futility pruningの深さの上限を5にし、razoringを行わない。
null move pruningでは、着手1手分を含む減深量`1 + null_move_reduction`から、その半分を0方向へ丸めて引く（Stockfish `src/search.cpp:1045`）。
最初の反復は前の反復がないので、詰みを探す局面としない。

### 新しい係数

表のSFはStockfish、YOはYaneuraOu、現行は起点の対応する値を表す。
評価値の単位を持つ値は、SFは歩兵価値208、YOは歩兵価値90（`source/evaluate.h:78`）で換算した。
Stockfishの行番号は`src/search.cpp`、YaneuraOuの行番号は`yaneuraou-search.cpp`のものである。

| 係数 | 開始値 | 範囲 | SF、YO、現行の値と根拠 |
|---|---|---|---|
| `ReverseFutilityMultBase`（`mult`の定数、歩兵価値に対する百分率） | 22 | 0〜450 | SFは45/208で22（1025行）、YOは76/90で84（3218行）、現行の余裕値は428 |
| `ReverseFutilityMultSlope`（`mult`の深さ1あたりの増分、歩兵価値に対する百分率） | 2 | 0〜12 | SFは4/208で2。YOと現行は深さの項を持たないので0 |
| `ReverseFutilityNoTtHit`（置換表にないときの`mult`の減算、歩兵価値に対する百分率） | 10 | 0〜50 | SFは20/208で10（1026行）、YOは21/90で23（3218行）、現行は0 |
| `ReverseFutilityImproving`（`mult`に対する百分率） | 272 | 0〜400 | SFは2789/1024で272（1029行）、YOは2686/1024で262（3221行）、現行は0 |
| `ReverseFutilityOpponentWorsening`（`mult`に対する百分率） | 33 | 0〜100 | SFは335/1024で33、YOは362/1024で35、現行は0 |
| `ReverseFutilityCorrection`（補正の値の絶対値に掛ける百分率） | 66 | 0〜200 | SFは131072/198435で66（1030行）、YOは131072/180600で73（3222行）、現行は0 |
| `LmpNonImproving`（improvingでないときの倍率、百分率） | 50 | 25〜150 | SFとYOは50（1199行、3593行）、現行は100 |
| `LmrMaxReduction`（手） | 3 | 1〜8 | 両エンジンには上限がない。現行は3。8は今回試す暫定の上限であり、最終値が上限に達した場合は測定記録に残す |
| `LmrBase`（1/100手） | 93 | 0〜200 | SFは948/1024で93（1919行）、YOは1133/1024で111（5150行）、現行は0 |
| `LmrOffset`（1/100手） | 68 | 0〜200 | SFは697/1024で68（1350行）、YOは691/1024で67（3886行）、現行は0 |
| `LmrNonImproving`（表の値の増分、百分率） | 38 | 0〜100 | SFは197/512で38（1919行）、YOは206/512で40（5150行）、現行は0 |
| `LmrPvReduction`（1/100手） | 50 | 0〜200 | SFは512/1024で50（1186-1187行）。YOのPVノードの項は窓の幅とttPvの条件に依存し（3879行、5150行）、単一の対応値がない。現行は0 |
| `LmrTtPv`（1/100手） | 91 | 0〜200 | SFは929/1024で91（1191-1192行）、YOは1013/1024で99（3573-3574行）、現行は0 |
| `LmrTtPvBase`（1/100手） | 295 | 0〜500 | SFは3023/1024で295（1346行）、YOは2819/1024で275（3880行）、現行は0 |
| `LmrTtPvPv`（1/100手） | 98 | 0〜200 | SFは1004/1024で98、YOは973/1024で95、現行は0 |
| `LmrTtPvValue`（1/100手） | 86 | 0〜200 | SFは885/1024で86、YOは905/1024で88、現行は0 |
| `LmrTtPvDepth`（1/100手） | 80 | 0〜200 | SFは816/1024で80（1347行）、YOは935/1024で91（3881行）、現行は0 |
| `LmrTtPvDepthCutNode`（1/100手） | 92 | 0〜200 | SFは940/1024で92、YOは959/1024で94、現行は0 |
| `LmrMoveCount`（手番号1つあたり、1/100手） | 6 | 0〜20 | SFとYOは65/1024で6（1352行、3889行）、現行は0 |
| `LmrCutNode`（1/100手） | 393 | 0〜600 | SFは4026/1024で393（1356-1357行）、YOは3611/1024で353（3903-3904行）、現行は0 |
| `LmrCutNodeNoTtMove`（1/100手） | 91 | 0〜200 | SFは933/1024で91、YOは985/1024で96、現行は0 |
| `LmrTtCapture`（1/100手） | 105 | 0〜250 | SFは1079/1024で105（1360-1361行）、YOは1054/1024で103（3909-3910行）、現行は0 |
| `LmrCutoffCount`（1/100手） | 26 | 0〜100 | SFは264/1024で26（1364-1365行）、YOは251/1024で25（3915-3916行）、現行は0 |
| `LmrCutoffCountMany`（1/100手） | 107 | 0〜250 | SFは1095/1024で107、YOは1124/1024で110、現行は0 |
| `LmrCutoffCountAllNode`（1/100手） | 111 | 0〜250 | SFは1138/1024で111、YOは1042/1024で102、現行は0 |
| `LmrAlphaEval`（αと判定用の評価値の差が歩兵価値1枚分のときの増分、1/100手） | 61 | 0〜150 | SFは3 × 208/1024で61（1383-1384行）。YOと現行はこの項を持たないので0 |
| `LmrNoTtMove`（LMRをしない手で置換表の手がないときの増分、1/100手） | 110 | 0〜250 | SFは1127/1024で110（1431-1432行）、YOは1057/1024で103（4022-4023行）、現行は0 |
| `LmrAllNode`（allNodeで減深量に掛けて足す割合の分子、百分率） | 108 | 0〜200 | SFは276/256で108（1387-1388行）、YOは273/256で107（3940-3941行）、現行は0 |
| `LmrDeeperMargin`（歩兵価値に対する百分率） | 25 | 0〜100 | SFは53/208で25（1414行）、YOは48/90で53（3997行）、現行はこの調整を持たない |
| `LmrShallowerMargin`（歩兵価値に対する百分率） | 4 | 0〜40 | SFは8/208で4（1415行）、YOは9/90で10（3998行）、現行はこの調整を持たない |

開始値を置き直す既存の係数は次の3つである。

| 係数 | 起点の値 | 新しい開始値 | 範囲 | 根拠 |
|---|---|---|---|---|
| `LmrDivisor`（百分率） | 156 | 203 | 100〜400 | SFの表の積は(2872/128)²/1024で約0.49 × ln d × ln m手（82行）、YOは(2763/128)²/1024で除数220（2169行） |
| `LmpBase`（1/100手） | 382 | 300 | 0〜8600 | SFとYOの`3 + d²`の3（1199行、3593行） |
| `LmpSlope`（1/100手） | 133 | 100 | 0〜3600 | SFとYOの`3 + d²`の係数1 |

廃する係数は`ReverseFutilityMargin`である。
SPSAの対象は、起点の38係数から1つを除き30を加えた67係数となる。

### 固定の定数

2026年10月17日の利用者の決定により、次の値はStockfishの値で固定し、SPSAで調整しない。
行番号はStockfishの`src/search.cpp`のものである。

| 定数 | 値 | Stockfishの値と行 |
|---|---|---|
| reverse futility pruningの深さの上限 | 残り深さ18以下 | `depth < 19`（1022行） |
| 詰みを探す局面でのreverse futility pruningの深さの上限 | 残り深さ5以下 | `depth < 6`（1022行） |
| reverse futility pruningの`mult`の上限 | 歩兵価値の41% | 85（1025行） |
| LMRを行う残り深さの下限 | 2 | `depth >= 2`（1395行） |
| 負の減深量による延長の上限 | 根からの手数が反復の深さの2倍未満なら2手、それ以外は0手 | `ss->ply < 2 * rootDepth ? 2 : 0`（1402行） |
| PVノードで減深後の深さに足す手数 | 1 | `+ PvNode`（1402行） |
| 子のβ打ち切り数の閾値 | 1より多い、2より多い | `cutoffCnt > 1`、`> 2`（1364-1365行） |
| αと判定用の評価値の差の上下限 | 歩兵価値の−31%と46% | −64と96（1384行） |
| allNodeの項の分母の定数 | 105（1/100手） | 268/256（1388行） |
| 置換表の打ち切りの深さの条件 | `d_tt > d − [v ≤ β]` | `ttData.depth > depth - (ttData.value <= beta)`（895行） |
| 置換表の打ち切りでcutNodeの条件を外す残り深さ | 4より深い | `depth > 4`（899行） |
| 打ち切りの前に着手後の局面を確かめる残り深さ | 7以上 | `depth >= 7`（917行） |
| LMRをしない手を1手または2手浅く読む減深量の閾値 | 511と536（1/100手）、2手目は通常深さが2より深いとき | 5234と5487、`newDepth > 2`（1436行） |
| 詰みを探す局面の閾値 | 歩兵価値の361%と105769%/D² | 750と220000/rootDepth²（395行） |
| 詰みを探す局面でのnull moveの減深量の減らし方 | 半分を引く | `R -= seekMate * R / 2`（1045行） |
| 根の近くのimprovingの比較の相手 | improvingを偽にする | `VALUE_NONE`（309行、883行） |

## 所要時間の見積り

時間は、同時対局数16の測定機での見積りである。
SPSAは、同時対局数16で3,000ペアの調整を完走した記録（[SPSAの手引き](../guides/spsa.md)の27,138秒、1ペア約9秒）から、30,000ペアを約75時間と見込む。
2026年10月17日に中止した38係数の調整は1反復あたり約1分で進んでおり、これより短く終わる可能性がある。
段階ゲートの時間は、判定までのペア数が強さの差で変わるので、幅を持たせた。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 形の変更の実装と検証（codexへ委任） | 2日程度 |
| 2 | 時間切れの事前確認（13反復、104ペア） | 約15分 |
| 3 | SPSA（30,000ペア） | 約75時間 |
| 4 | 反映、テスト、および事前対局（32ペア） | 約1時間 |
| 5 | 候補のSTC | 約1〜8時間 |
| 6 | 通過した場合のLTC | 約5〜30時間 |

測定機の占有は、STCで棄却される場合は段階2〜5の和の約77〜84時間、段階6まで進む場合は約82〜114時間となる。

## 実装フェーズ

基本シードは、時間切れの事前確認を3200000000、セッションを3250000000、事前対局を3310000000、STCを3350000000、LTCを3400000000と仮に定める。
これらは起案時点で、`data/matches/`と`data/spsa/`の`manifest.json`に残る保存済みの基本シード、および他の設計書が予約した値から、各測定のペア数以上離れた未使用の値である（[教訓](../lessons/derive-seed-adjacent-collision.md)）。
着手時に同じ方法で未使用であることを確かめ直し、使われていれば設計書を書き換える。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ1　形の変更の実装

起点からブランチ`search-pruning-forms`を切り、worktree `data/worktrees/search-pruning-forms`で作業する。
F0、F1の準備、F4の準備、F2、F1、F3、F4、F5、F6の順に、1つを1コミットにする。
探索を変えない移行の3コミットを先に置くのは、起点と同じ探索のまま検証を済ませ、探索を変えるコミットの変化だけを分けて記録するためである。
F2をF1より前に置くのは、F1が静的評価、判定用の評価値、および補正の値をF2の定義で使うからである。
探索を変えない移行の3コミットでは、`minase dev bench`の全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が直前のコミットと一致することを確かめる。
F2からF6までの各コミットでは、同じbenchの探索ノード数の直前のコミットからの変化を、局面ごとと合計で記録する。
F2は全ノードで静的評価を求め、F1とF4は置換表の手の合法性を確かめるので、`taskset -c 3`で1つのコアに固定した`minase dev bench --depth 9 --repetitions 3`の中央値を、起点と形の変更の最後のコミットで交互に3回ずつ測り、探索速度の変化を記録する（[教訓](../lessons/pin-bench-to-performance-core.md)）。
ノード数の変化と探索速度は、測定記録`search-pruning-forms-bench`に残す。
新しい係数と開始値を置き直す既存の係数は`params.rs`の表へ反映し、調整用ビルド（`--features tuning`つきのビルド）が`Tune_<係数名>`のUSIオプションとして宣言することを確かめる。
固定の定数は、`pruning.rs`などの使う箇所の定数とし、docコメントにStockfishの値と行を書く。
新しい式の単体テストは、式の写しではなく数値の例で書く（[テストの手引き](../guides/testing.md)）。
F0のテストは、受け渡しの表の各行について、子に渡る値を確かめる。
完了条件は、`cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`の通過、探索を変えない移行のノード数の一致、探索を変えるコミットのノード数の変化の記録、および探索速度の記録である。

### フェーズ2　パラメーターファイルと時間切れの事前確認

F6まで入った形の変更の最後のコミットでビルドした`minase`を`data/spsa/search-pruning-forms-runner/`へ複製してrunner（測定を実行するプログラム）として固定し、以後のフェーズはこのrunnerで実行する。
`minase spsa params --engine commit:<形の変更> --rules engine-default`で既定のパラメーターファイルを生成し、`FmScale`の行を削除した67行のファイルを`data/spsa/search-pruning-forms.params`とする。
開始値と範囲が形の変更の`params.rs`の表と一致すること、および`c_end`と`r_end`が既定値であることを、使い捨てのスクリプトで機械的に突き合わせる。

続いて、手引きの第3節に従い、時間管理の4係数を思考時間が最も長くなる側の範囲の端に置いても時間切れが起きないことを確かめる。
写しの開始値を`HardRemainingShare`＝50、`HardSoftRatio`＝800、`MinMoves`＝40、`IterationRatio`＝150へ書き換えた`data/spsa/search-pruning-forms-safety.params`で、13反復（104ペア）を実行する。

```console
<固定したminase> spsa \
  --run-dir data/spsa/search-pruning-forms-safety --seed 3200000000 \
  --engine commit:<形の変更> --params data/spsa/search-pruning-forms-safety.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 13 --pairs-per-iteration 8
```

`time_forfeits`（時間切れの件数）が1件でもあれば、[デバッグの手引き](../guides/debugging.md#時間切れと時間管理)の手順で原因を調べ、該当する係数の範囲を狭める案を利用者に示してから本番へ進む。
結果は測定記録`search-pruning-forms-safety`に残す。
完了条件は、時間切れとエンジン異常が0件であることと、測定記録の保存である。

### フェーズ3　SPSAによる調整

```console
<固定したminase> spsa \
  --run-dir data/spsa/search-pruning-forms --seed 3250000000 \
  --engine commit:<形の変更> --params data/spsa/search-pruning-forms.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 3750 --pairs-per-iteration 8
```

最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`search-pruning-forms`に、手引きの第6節の項目とともに残す。
最終値を丸めた整数が範囲の上限または下限になった係数を示す。
新しい30係数と開始値を置き直した3係数については、最終値が開始値（Stockfishの換算値）と現行の値からそれぞれどれだけ離れたかを、`c_end`に対する比で示す。
この比は、SPSAがStockfishの値と現行の挙動のどちらへ寄ったかを読むための目安であり、採否には使わない。
完了条件は、セッションの完了と測定記録の保存である。

### フェーズ4　反映と事前対局

手引きの第7節の手順1に従い、`minase spsa apply`で最終値を表の既定値へ書き込み、`git diff`で既定値だけが変わったことを確かめてコミットする。
続いて`cargo test`（通常ビルドと調整用ビルド）を測定より先に実行し、表の既定値に依存するテストが失敗した場合は、テストの前提が新しい既定値でも成り立つようにテストを直して別のコミットにする。
テストの失敗を、末尾に`; true`を付けるようなコマンドの組み方で握りつぶさない。
候補の調整用ビルドの宣言を`minase spsa params`で出力し、既定値の欄が`apply`の出力と一致することを確かめる。

採否測定の前に、候補が時間切れやエンジン異常を起こさないことと、時間の使い方の変化を確かめるため、STCの条件で候補と起点の32ペアの事前対局を行う。

```console
<固定したminase> match run \
  --run-dir data/matches/search-pruning-forms-smoke --seed 3310000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 elo --pairs 32
```

候補と起点の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ（反復深化で読み終えた深さ）、および探索を止めた理由の分布を集計して、測定記録`search-pruning-forms-smoke`に残す。
総思考時間の比と終局時の残り時間は、`pairs/`に保存された各手の思考時間から計算する。終局時の残り時間は、持ち時間に1手の加算を着手数だけ足し、思考時間の合計を引いた値である。
手数帯ごとの完了深さと停止理由の分布は、`scripts/clock_profile.py`を`--role candidate`と`--role baseline`で別々に実行して集計する。このスクリプトが出す時間予算の比は旧係数の予算式に基づくので、解釈に使わない。
完了条件は、テストの通過、事前対局での時間切れとエンジン異常が0件であること、および測定記録の保存である。

### フェーズ5　採否測定

```console
<固定したminase> match run \
  --run-dir data/matches/search-pruning-forms-stc --seed 3350000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

STCの判定は[SPRTの手引き](../guides/sprt.md)の振分け規則に従う。
候補が有意に強い`H1`ならLTCへ進み、改善を示せない`H0`なら不採用とし、3,000ペアで判定保留ならその時点の対数尤度比の符号でLTCへ進むかを決める。
LTCは測定名の接尾辞を`-ltc`、基本シードを3400000000、持ち時間を`time=60000+200`とし、`--max-pairs`を付けない。
LTCで`H1`となり、エンジン異常、時間切れ、および拒否着手が0件なら採用し、`H0`なら不採用とする。
LTCの判定保留は、手引きのとおり`--resume`と`--target-pairs`で延長する。
結果は測定名ごとの測定記録に残す。
完了条件は、採否の確定と測定記録の保存である。

## 検証

- F0、F1の準備、およびF4の準備の各コミットで、`minase dev bench`の全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が直前のコミットと一致すること。
- F2からF6までの各コミットで、同じbenchの探索ノード数の直前のコミットからの変化が、測定記録`search-pruning-forms-bench`に記録されていること。
- `cargo test`では`invariants`機能が常に有効になるので、PVノードがcutNodeにならないという検査が`cargo test`の全探索で失敗しないこと。
- 新しい30係数が調整用ビルドの`Tune_`オプションとして宣言され、`ReverseFutilityMargin`の宣言がなくなり、開始値を置き直した3係数を含めて、パラメーターファイルの開始値と範囲が表と一致すること。
- 固定の定数の表の各値が、コードの定数とdocコメントに書かれていること。
- 候補の`params.rs`の差分が既定値の書き換えだけであり、調整用ビルドの宣言の既定値が`apply`の出力と一致すること。
- `cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`が通過すること。
- 時間切れの事前確認、SPSA、事前対局、および採否の測定で、エンジン異常、時間切れ、および拒否着手が0件であること。最初の測定で1件でも出た場合は、測定を止めて原因を調べる。

## 完了条件

- 形の変更の検証結果（ノード数の一致と変化、および探索速度）、時間切れの事前確認、SPSAの結果、事前対局、および採否が、測定記録に残っている。
- 採用した場合は、候補のブランチがmasterへ統合され、依存関係の節に挙げたADRが新しいADRに置き換えられている。統合は利用者の許可を得てから行う。
- 不採用の場合は、結論と測定記録だけをmasterへ残し、候補のブランチはmasterへ統合しない。
- ROADMAPの状態表が本書の結論と一致している。

## 参考資料

- [探索と時間管理の係数の関数形に関する既存実装との比較](../research/search-coefficient-forms-prior-art.md)。
- Stockfish（コミット5c4cef0b）の`src/search.cpp`。LMRの表は80〜82行、補正の値は95〜118行と832行、反復の初期化は309行、`seekMate`は394〜395行、根の呼び出しは412行、cutNodeとallNodeの定義は742〜766行、ttPvは842行と1651〜1657行、静的評価と判定用の評価値は846〜870行、improvingとopponentWorseningは883〜884行、置換表の打ち切りは894〜934行、razoringは1017〜1018行、reverse futility pruningは1020〜1033行、null moveは1045〜1048行、improvingの更新は1080行、IIRは1085行、late move pruningは1199行、LMRは1183〜1192行、1345〜1420行、および1917〜1920行、LMRをしない手の減深は1428〜1436行、前の反復の主変化を辿るノードは791〜794行、子への受け渡しは1395〜1455行、子のβ打ち切り数は829行と1562行、静止探索のPVの印は1741行と1908行。歩兵価値208は`src/types.h:199`。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`。LMRの表は2169行、reverse futility pruningは3217〜3227行、late move pruningは3593行、LMRは3573〜3574行、3879〜3941行、3978〜4000行、4017〜4029行、および5148〜5151行。歩兵価値90は`source/evaluate.h:78`。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)と[SPRTによる棋力測定の手引き](../guides/sprt.md)。
