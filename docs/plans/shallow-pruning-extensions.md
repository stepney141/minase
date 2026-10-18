# 浅い前向き枝刈りの深さと対象の拡張の設計書

## 先に読む要約

本書は、探索の浅いノードで働く前向き枝刈り（読む価値が低いと見込んだ手やノードを読まずに済ませる枝刈り）とnull move pruningを、チェスのStockfishの形へそろえて深いノードと新しい対象へ広げ、着手時点のmasterである起点より強くなるかを確かめる。
起点は、探索の関数形をStockfishとYaneuraOu（やねうら王）の形へ寄せる一連の設計書で1つ前に当たる[探索の枝刈りとLMRの関数形の見直し](archived/search-pruning-forms.md)が完了した後のmasterである。
futility pruningは、静的評価に余裕値を足してもαに届かない手を読まない枝刈りであり、late move pruningは、後方の静かな手を読まない枝刈りである。
SEE（static exchange evaluation）は、着手の到達升での駒の取り合いの損得の見積りである。
minaseでは、これらを残り深さ3以下の零窓のノードに限っている。
Stockfishでは、これらはより深いノードとPVノードでも働き、捕獲手のfutility pruningと静かな手のSEEによる枝刈りも持ち、王手になる手を別に扱う。
両エンジンとの違いは[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)にまとめてある。
2026年10月17日の利用者の決定により、式の形はStockfishに合わせ、係数の開始値はStockfishの値を換算した値とする。
本書は、枝刈りの条件を手法ごとに分ける移行、王手の判定の前計算、および9項目を扱う。
9項目は、静かな手のfutility pruning、late move pruning、SEEによる捕獲手の枝刈り、捕獲手のfutility pruning、静かな手のSEEによる枝刈り、razoring、null move pruningの条件と検証探索、reverse futility pruningの返り値、および詰みを探す局面でのLMRの王手の項である。
razoringは、静的評価がαを大きく下回るノードを静止探索だけで判定する枝刈りである。
null move pruningは、手番を相手に渡しても打ち切れるノードを浅く読んで打ち切る枝刈りであり、reverse futility pruningは、静的評価がβを十分に上回るノードを読まずに返す枝刈りである。
新しい係数を既存の係数とまとめてSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で調整する。
調整した版は、[統括計画](search-forms.md)の2026年10月18日の利用者の決定により、対局の結果によらず採用する。起点のmasterとの短時間条件STCの逐次検定は、採否に使わない記録として行い、長時間条件LTCは行わない。
完了条件は、候補の採用と、形の変更の検証、SPSA、事前対局、およびSTCの測定記録の保存である。

## 状態

進行中。2026年10月17日に起案し、2026年10月18日に利用者の指示を受けて、[統括計画](search-forms.md)のサブタスク2として着手した。
[前の設計書](archived/search-pruning-forms.md)は採用で完了したので、F0からF6はすべて起点に入っている。
起点の調整対象は`FmScale`を除いて68係数であり、本書の後は11を除き14を加えた71係数になる。
基本シードの予約値が保存済みの測定とSPSAで使われていないことは、着手時に確かめた。
次の一手は、フェーズ1（移行と9項目の実装）を始めることである。

## 目的

本書が確かめるのは、「浅い前向き枝刈りとnull move pruningの適用範囲と式の形をStockfishにそろえ、強さを決める係数を調整すれば、現行の形で調整した探索より強くなる」という仮説である。
minaseの浅い前向き枝刈りの深さの上限3は、[棋力向上段階4](archived/strength-stage4.md)で静的評価に基づく枝刈りを入れた時点の値であり、その後の段階で加えたSEEによる捕獲手の枝刈りとlate move pruningも同じ上限を引き継いだ。
いずれも、評価関数がPSTとFMだった時期の判断である。
PSTは駒の種類と升ごとの点数の表、FMは2駒の関係の補正である。
Stockfishのlate move pruningとSEEによる捕獲手の枝刈りには深さの上限がなく、静かな手のfutility pruningはLMRの減深量を引いた深さであるlmrDepthが12未満のときに働き、いずれもPVノードでも働く（`src/search.cpp:1196-1259`）。
[late move pruningのADR](../adr/count-only-quiet-moves-for-late-move-pruning-and-shrink-the-limit-when-not-improving.md)も、全深さへ適用を広げる変更を未測定の候補として残している。
razoringは、2026年10月17日に中止した現行の形でのSPSAの途中で、`RazoringMargin2`が範囲の上限に達し、`RazoringMargin1`も上限に近づいた。
余裕値が大きいほどrazoringは働かないので、これは、SPSAが範囲の中で最もrazoringが働かない値を選ぼうとした動きであり、NNUEの下で現行の形のrazoringが役に立っていない兆候と読める。
2026年10月17日の利用者の決定により、razoringのコードは残し、Stockfishの形に改めて調整し直す。

## 適用範囲

本書の用語を次のように定める。

- **前の設計書**は[探索の枝刈りとLMRの関数形の見直し](archived/search-pruning-forms.md)であり、F0からF6はその項目の記号である。F0はcutNodeとallNodeの区別、F1はreverse futility pruningとttPv、F2は静的評価と判定用の評価値の求め方、F3はlate move pruningのimprovingの倍率、F4はLMRの減深量の項、F6は詰みを探す局面の判定（Stockfishの`seekMate`）である。
- **起点**は、[前の設計書](archived/search-pruning-forms.md)が完了した後、フェーズ1を始める時点のmasterの先頭のコミットである。前の設計書の候補が採用されていれば、その項目と調整値を含む。
- **形の変更**は、本書の移行と9項目を実装したコミット列であり、新しい係数と開始値を置き直した既存の係数を、表の開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で表（`crates/minase/src/search/alphabeta/params.rs`）の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **残り深さ**`d`は、置換表の手がないノードで深さを1減らす内部反復減深（IIR）を適用した後の深さであり、探索の深さ1を1手と数える。
- **零窓のノード**は、探索窓の幅β − αが1のノードである。窓の幅が1より大きいノードをPVノードという。
- **cutNode**は、親がβ打ち切りを見込んで零窓で読ませる子のノードであり、**allNode**は、PVノードでもcutNodeでもないノードである。前の設計書のF0が、Stockfishの受け渡しの規則に従って導入する。
- **ttPv**は、PVノードであるか、置換表のエントリがPVの印を持つことをいう。前の設計書が導入する。
- **lmrDepth**は、手を読む深さ`d − 1`から、Stockfishが枝刈りの前に求める部分のLMR（late move reductions、後方の手を浅く読む減深）の減深量を引いた深さである。定義は設計判断の表に書く。
- **followPV**は、根から前の反復の最善応手列をたどってきたノードであることを表す印であり、Stockfishの`ss->followPV`（`src/search.cpp:791-794`）に当たる。前の設計書のF5が導入する。
- **王手**は、RULES.md第8条の定義による。着手の後に、相手が王駒を1枚だけ持つならその王駒を手番側が次の着手で取れる状態、相手が王将または玉将と太子を併せ持つなら手番側の獅子が1手で両方を取れる状態（同条第7項）をいう。**王手になる手**は、着手の後にこの状態を作る手であり、Stockfishの`givesCheck`に当たる。
- **詰み帯と専用の帯**は、詰みまでの手数を表す評価値の範囲と、千日手などの反復の結果を表す評価値の範囲（`crates/minase/src/search/mod.rs`の`MATE_THRESHOLD`と`REPETITION_BAND_START`）である。
- **improving**は、起点の`negamax.rs`の`improving`が定める判定であり、同じ手番の2手前の局面より静的評価が上がったことをいう。
- **minaseの安全条件**は、αとβが詰み帯と専用の帯の外にあること、手番側のどの王駒にも相手の疑似利きが届かないこと（[ADR](../adr/skip-static-eval-pruning-when-a-royal-is-attacked-and-extend-reverse-futility-to-depth-18.md)）、および手を飛ばす枝刈りではそのノードで探索済みの最善値が負の詰み帯と専用の帯の外にあることをいう。2つ目はStockfishの`!ss->inCheck`、3つ目は`!is_loss(bestValue)`に当たる。本書のどの枝刈りも、この安全条件を要求する。

対象は、2つの移行と9項目である。

- 枝刈りの条件を手法ごとに分ける移行（M1）。
- 王手の判定の前計算（M2）。
- 静かな手のfutility pruning（futility pruning、S1）。
- late move pruning（S2）。
- SEEによる捕獲手の枝刈り（SEE pruning for captures、S3）。
- 捕獲手のfutility pruning（futility pruning for captures、S4）。
- 静かな手のSEEによる枝刈り（S5）。
- razoring（S6）。
- null move pruningの条件、減深量、および検証探索（null move verification search、S7）。
- reverse futility pruningの返り値（S8）。
- 詰みを探す局面でのLMRの王手の項（S9）。

次の事項は対象外とする。

- 王駒に相手の利きが届くノードを浅い枝刈りから除く条件と、獅子の規則に依存してSEEが判定不能な手を枝刈りしない規則。いずれも中将棋の規則に由来するので残す（[ADR](../adr/skip-static-eval-pruning-when-a-royal-is-attacked-and-extend-reverse-futility-to-depth-18.md)、[SEEのADR](../adr/mark-only-lion-rule-dependent-exchanges-unknown-in-see.md)）。
- lmrDepthをhistoryの値で増減する項と、continuation historyの値が低い静かな手を読まない枝刈り（Stockfish `src/search.cpp:1229-1240`）。continuation historyは直前の手を文脈とするhistoryであり、minaseはまだこの表を持たない。次の[historyの形の見直し](history-forms.md)が、表とともにStockfishの重みでこれらを加える。
- reverse futility pruningとrazoringに使う静的評価を置換表の値で置き換える処理（Stockfish `src/search.cpp:862-865`）。前の設計書が扱う。
- Stockfishの浅い枝刈りの前提条件`pos.non_pawn_material(us)`と、捕獲手のSEEによる枝刈りにある最後の駒を捨てる手の除外（`src/search.cpp:1196, 1223-1224`）。前者はポーンだけの終盤、後者は合法手のない側を引き分けとするステイルメイトのための条件であり、中将棋には相当する仕組みがない（RULES.md第23条）。2026年10月17日の利用者の決定は、Stockfishに相当する仕組みがない事項はYaneuraOuに合わせるとしており、YaneuraOuも前者を外している（`yaneuraou-search.cpp:3585-3588`）。
- YaneuraOuにだけある項。静かな手のfutility pruningの`151 × !bestMove`、捕獲手のSEEの`α ≥ 引き分けの値`の条件、およびrazoringの深さの2乗の余裕値である。2026年10月17日の利用者の決定により入れない。
- 詰みを探す局面の判定（Stockfishの`seekMate`）。seekMate本体は前の設計書が導入する。
- 損な先頭の捕獲手よりkiller手を先に返す手の順序。killer手は、同じ手数の別のノードでβ打ち切りを起こした静かな手である。対象は`ordering.rs`の`MovePicker::reset`の`killers_before_losing_capture`である。残り深さ3以下という条件は、手の順序付けの[診断](../measurements/move-ordering/move-ordering-diag.md)で決めたものであり（[ADR](../adr/try-killers-before-a-losing-first-capture.md)）、本書は変えない。
- 手の並べ替えで王手になる手に加点すること（Stockfish `src/movepick.cpp:242`）。[historyの形の見直し](history-forms.md)が、本書のM2の判定を使って扱う。

## 依存関係

起点は、[前の設計書](archived/search-pruning-forms.md)が完了した後のmasterの先頭である。
前の設計書の候補が採用されたかどうかにかかわらず、起点の表の既定値を、開始値を置き直さない既存の係数のSPSAの開始値とする。
本書は、前の設計書が導入する次のものを使う。
F0が導入するcutNodeとallNodeの区別はS6とS7で使い、F1とF4が導入するttPvとLMRの減深量の前半`r_pre`はlmrDepthの計算で使い、F2が定める判定用の評価値はS6で使う。
F0は挙動を変えない移行なので、前の設計書の候補が不採用でもmasterへ統合されることを前提とする。
統合されていなければ、本書のフェーズ1の最初に同じ移行を行い、探索ノード数の一致で検証する。
ttPvとLMRの項が起点にない場合は、lmrDepthは起点にある項だけで求める。
reverse futility pruningの返り値は、前の設計書のF1ではなく本書のS8が扱う。
本書は、前の設計書の採否にかかわらず、各項目の安全条件（S8の「静的評価 ≥ β」など）を自ら要求する。
本書のM2が導入する王手の判定は、[historyの形の見直し](history-forms.md)の手の並べ替えと、[静止探索の補正と枝刈りの形の見直し](qsearch-forms.md)が再利用し、それぞれで重複して導入しない。
capture historyの表の上限は、historyの形の見直しがStockfishの10692へ固定するが、本書の時点では起点の係数`CaptureHistoryLimit`のままである。
連鎖の順序は、前の設計書の後に本書、その後にhistoryの形の見直しと続く提案であり、最終的な順序は利用者が決める。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、対局の手順は[SPRTの手引き](../guides/sprt.md)に従い、採否の決め方は[統括計画](search-forms.md)の設計判断「各サブタスクの採否」による。
測定機はlishogiのBotと共用するので、測定機での対局の同時対局数は16を上限とする。
時間切れの事前確認、SPSAのセッション、事前対局、およびSTCは、[探索の関数形の見直しの統括計画](search-forms.md)に従い、vast.aiで借りる調整機で行う。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は、次のADR（設計判断の記録、`docs/adr/`）の決定の一部を変える。
いずれも、既定の評価関数がNNUEへ置き換わった2026年10月12日より前、評価関数がPSTとFMだった時期の測定に基づく。
採用した場合は、完了時に、起点の時点で有効なADR（先行の設計書が置き換えていればその後継）を確認し、新しいADRで置き換える。
部分的に変える場合は、残る決定も新しいADRへ引き継ぐ。

- [浅いノードの前向き枝刈りの適用条件のADR](../adr/archive/skip-static-eval-pruning-when-a-royal-is-attacked.md)は、2026年9月6日に、futility pruning、late move pruning、SEEによる捕獲手の枝刈り、およびreverse futility pruningを残り深さ3以下の零窓のノードに、razoringを残り深さ2以下に限り、reverse futility pruningは静的評価をそのまま返すと決めた。返り値をβへ丸める案は、上位ノードが得る情報を減らすとして棄却している。S1からS5は深さの上限と零窓の条件をStockfishの形に改め、S6はrazoringの形を、S8は返り値を改める。王駒への利きによる除外は残す。ADRは再測定を留保していないが、返り値の正しさは確かめておらず、窓の外の値もそのまま返すfail-softの近似として扱う、と記している。
- [improvingの使い道のADR](../adr/archive/use-improving-only-for-futility-margins.md)は、2026年10月4日に、improvingをfutility pruningの余裕値にだけ使うと決めた。S1はStockfishと同じくfutility pruningからimprovingを外し、S7はnull move pruningを試す条件にimprovingを使う。前の設計書も適用先を広げる。ADRは再測定を留保していない。
- [SEEによる捕獲手の枝刈りのADR](../adr/prune-losing-captures-by-see-at-shallow-nodes.md)は、2026年9月21日に、対象を捕獲手に限り、余裕値を深さ1〜3の深さ別の値にすると決めた。深さによらず1つの余裕値にする案は、診断で深さごとに適格な値が異なったとして棄却している。S3は深さに比例する余裕値とcapture historyの項に改め、王手になる静かな手も対象にし、S5は静かな手を対象に加える。ADRは再測定を留保していない。
- [late move pruningの数え方のADR](../adr/archive/count-only-quiet-moves-for-late-move-pruning.md)は、2026年10月4日に、静かな手だけを数える上限式を決めた。全深さへ適用を広げる変更は未測定の候補として残すと、ADR自身が記している。S2は深さの上限を外し、PVノードにも適用する。上限式と、静かな手だけを数える決定は残す。
- [null moveをPVノードでも行うADR](../adr/archive/allow-null-move-and-tt-cutoff-at-pv-nodes.md)は、2026年9月28日に、null move pruningをノードの種別と静的評価のβとの大小によらずに試すと決めた。棄却したのは「非PVノードに限り、かつ補正後の静的評価がβ以上」を全深さに課す案である。S7は、cutNodeに限ったうえでStockfishの静的評価の条件を課す。ADRは、ノード種別の予測を持ったうえで条件を加える方式を未測定と記している。
- [null moveの検証探索を実装しないADR](../adr/omit-null-move-verification-search.md)は、2026年9月6日に、検証探索がβを下回る割合（矛盾率）が深さ5と7で0件だったとして、検証探索を入れないと決めた。同じADRは、減深量が大きくなるほど縮めた探索は浅くなるので、再提案するときは現行の構成で矛盾率を測り直す必要があると記している。S7はStockfishと同じく残り深さ16以上で検証探索を行い、減深量もStockfishの値から始めるので、フェーズ1で矛盾率を測って記録する。
- [capture historyのADR](../adr/bound-capture-history-and-use-it-in-qsearch.md)は、2026年10月10日に、capture historyを通常探索と静止探索の手の順序に使うと決めた。決定は他の用途を禁じていないが、S3とS4は枝刈りの判定にも使うので、新しいADRに用途を記録する。
- [一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)は、2026年10月4日に、razoringと、静的評価の余裕によるnull moveの減深の加算を含む8項目を採用した。S6はrazoringを外さずに形を改め、S7は減深の加算の上限3を外してStockfishの形にする。

## 設計判断

2026年10月17日の利用者の決定は次のとおりである。
1つ目に、両エンジンで形や値が違う場合はStockfishに合わせ、YaneuraOuにだけある項目は入れず、Stockfishにだけある項目は入れる。Stockfishに相当する仕組みがない事項はYaneuraOuに合わせる。
2つ目に、新しい係数の開始値は両エンジンの換算値とし、両者が違えばStockfishの値とする。既存の係数も、Stockfishに同じ形の値があれば換算値を開始値に置き直す。両エンジンに対応する値がない係数は、現行と同じ挙動の値から始める。
3つ目に、cutNodeとallNodeの区別を導入し、Stockfishが使う箇所で同じように使う。
4つ目に、minaseにあってStockfishにない既存の仕組みのうち、中将棋の診断や測定で決めたものは残し、Stockfishの仕組みを並べて足す。本書の範囲では、静かな手だけを数えるlate move pruningと、成る手を枝刈りの対象から外す保護がこれに当たる。中将棋の規則から来る仕組みも残す。
5つ目に、SPSAで調整するのは各仕組みの強さを決める値（余裕値の切片と傾き、重み、倍率など）だけにし、上下限、平均の重みなどはStockfishの値で固定する。
6つ目に、評価値の単位を持つ係数の範囲は、Stockfishの換算値、YaneuraOuの換算値、およびminaseの現行の対応する値の3つをすべて含むように取る。中将棋は将棋ともチェスとも異なるゲームなので、値はminaseのSPSAが決める。
7つ目に、本書の範囲では、静かな手のSEEによる枝刈りを含め、razoringのコードを残してStockfishの形にし、捕獲手のfutility pruningに専用の定数と傾きを持たせ、futility pruningの深さにlmrDepthを使い、王手になる手の扱いとその判定の前計算、null moveの残りの条件と検証探索、およびreverse futility pruningの返り値を本書で扱う。
次の表は、これらの決定を本書の範囲に当てはめた設計である。

| 項目 | 決定 |
|---|---|
| 候補の束ね方 | 9項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整する。起案時は1回の段階ゲートで採否を決めるとしていたが、2026年10月18日の利用者の決定により、候補はSTCの結果によらず採用する（[統括計画](search-forms.md)の設計判断「各サブタスクの採否」）。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。項目ごとにSPSAと段階ゲートを繰り返す案は、1項目あたり、調整機でのSPSAに約13〜18時間と約25〜35米ドル、測定機でのSTCとLTCに約6〜38時間を要するので採らない。代償として、項目ごとの寄与は測らない。不採用のときに一部の項目だけを選び直して再測定しない。 |
| 開始値と検証 | 新しい係数と、Stockfishに同じ形の値がある既存の係数の開始値は、Stockfishの値を換算した値とする。評価値の単位を持つ値は歩兵価値208で百分率に換算する。各項目のコミットで、`minase dev bench`（固定の15局面を指定の深さまで探索するコマンド）の深さ3、5、7の全15局面の探索ノード数の変化を記録する。挙動を変えない移行（M1とM2）だけは、探索ノード数、評価値、および最善手が起点と一致することで検証する。 |
| 3つの値の比較 | 評価値の単位を持つ係数ごとに、Stockfishの換算値（÷208）、YaneuraOuの換算値（÷90）、およびminaseの現行の対応する値を、新しい係数の表に並べる。YaneuraOuは同じ形で定数を将棋向けに調整し直しており、歩兵価値で見るとStockfishの2〜4倍の大きさになることが多く、minaseの現行の値もその大きさに近い。開始値が他の2つから大きく離れる係数は`RazoringDepthMargin`であり、開始値164はYaneuraOuの約900、現行の約1000〜1600の数分の1である。`NullMoveEvalScale`の開始値163も現行の56の約3倍で、YaneuraOuはこの項を持たない。 |
| SPSAの対象 | 起点の調整対象の係数（`FmScale`を除く全係数）から、形を置き換える11係数を除き、新しい14係数を加える。除く係数は、`FutilityMargin1`〜`3`、`NonImprovingFutility1`〜`3`、`SeeMargin1`〜`3`、および`RazoringMargin1`〜`2`である。既存の係数のうち`NullMoveBase`、`NullMoveSlope`、および`NullMoveEvalScale`は、Stockfishの換算値へ開始値を置き直す。`LmpBase`と`LmpSlope`の開始値は前の設計書のF3が置き直すので、本書は起点の既定値を継承する。前の設計書の候補の係数の数は同書の表で確かめ、着手時に起点の表で数え直す。前の設計書の候補を採用した場合、本書の時点の係数の数は70前後になる。新しい係数だけを調整する案は、既存の係数との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の6係数を含めるので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 既定の設定を流用する。3,750反復、1反復8ペアで計30,000ペアとする。ペアは同じ開始局面から先後を入れ替えて指す2局の組である。SPSAは調整機で行い、持ち時間は測定機の10秒＋0.1秒を[統括計画](search-forms.md#持ち時間の換算)の方式で換算した値、同時対局数は[統括計画](search-forms.md#同時対局数とメモリ)が調整機ごとに決める数とし、`c_end`を範囲の1/6、`r_end`を0.002とする。`c_end`は最後の反復の摂動幅、`r_end`は最後の反復の学習率であり、値は[SPSAの手引き](../guides/spsa.md)の既定による。30,000ペアは候補を1つ作るための予算であり、最終値を最適値とは呼ばない。総ペア数は、[利得と規模のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)が定める標準の30,000ペアである。30,000ペアで全係数を十分に調整できるかは未検証であり、不採用は、この予算と設定で改善を示さなかったことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。 |
| 固定する定数 | 上下限、平均の重み、検証探索を行う残り深さの下限などは、後述の固定の定数の表のとおりStockfishの値で固定し、SPSAで調整しない。 |
| 枝刈りの条件を手法ごとに分ける（M1） | 現行は、「残り深さ3以下の零窓のノード」という1つの判定（`negamax.rs`の`pruning_node`）で、futility pruning、late move pruning、SEEによる捕獲手の枝刈り、reverse futility pruning、およびrazoringの対象ノードを決めている。本書は最初に、この判定を手法ごとの条件へ分ける挙動を変えない移行を置き、その後の各項目で手法ごとの条件をStockfishの形に改める。静的評価は、前の設計書のF2が通常探索の各ノードで1回求めて保持するので、本書はその値を使う。F2が不採用なら、静的評価を使う枝刈りのいずれかが働き得るノードで求める。correction historyは、探索値と静的評価のずれを学習して静的評価を補正する表である。静的評価を求めるノードが増えても、`update_correction`は静的評価がなければその場で求めるので、この表の更新は変わらない。静的評価と王駒への利きを調べるノードが増える費用は、探索速度（NPS、1秒あたりの探索ノード数）の低下として測る。 |
| lmrDepthの定義 | Stockfishは、各手について枝刈りの前に`r = reduction(improving, depth, moveCount) − 512 × [PVノード] + 929 × [ttPv]`を1/1024手単位で求め、`lmrDepth = newDepth − r/1024`とする（`src/search.cpp:1183-1203, 1917-1920`）。`reduction()`は表の積、improvingでないときの増分、および定数948からなる。前の設計書のF4は、この部分に当たる減深量を、1/100手単位の前半`r_pre`として後半と分けて定める。本書は`lmrDepth = d − 1 − r_pre/100`（0方向へ丸める整数除算）とする。historyによる補正、cutNodeとallNodeの項、上限の切り詰めなど、Stockfishが枝刈りの後に加える項は`r_pre`に含まれない。Stockfishと同じく、捕獲手とkiller手を含む全ての手について`r_pre`を求める。F4が不採用で起点に`r_pre`がない場合は、起点のLMRの表の値を100倍して`r_pre`の代わりにする。 |
| 王手の判定の前計算（M2） | Stockfishは、ノードごとに相手の王の升へ利く升を駒種ごとに`check_squares`として、相手の王への走り駒の利きを遮る駒を`blockers_for_king`として前計算し、`gives_check`で着手が王手になるかを判定する（`src/position.cpp:731-774`）。本書は、同じ役割の前計算を中将棋の王手の定義（RULES.md第8条）に合わせて導入する。相手が王駒を1枚だけ持つ局面では、その王駒の升へ利く升の集合を手番側の駒種ごとに求め、手番側の駒のうち、動けば別の手番側の駒の利きがその王駒へ通る駒の集合を求める。着手後の駒が到達升から王駒へ利くか、遮っていた駒が線の外へ動くなら、王手になる手とする。前計算で判定できない着手は、着手後の局面の利きで判定する。これは、獅子、角鷹、および飛鷲の2段階移動で経由升の駒を取る手と、相手が王将または玉将と太子を併せ持つ局面（第8条第7項の獅子の場合）である。この2つの経路は判定の対象で決まり、どちらの経路でも結果は同じ定義の王手であることを、着手後の局面の利きによる参照実装と照合する単体テストで確かめる。王手になる手の判定を使うのはS1、S3からS5、およびS9であり、M2のコミットは判定を求めるだけで探索を変えない。 |
| 静かな手のfutility pruningの形（S1） | Stockfishの`staticEval + 164 + 119 × lmrDepth + 90 × [staticEval > α] ≤ α`を、lmrDepthが12未満の手に適用する形にする（`src/search.cpp:1242-1252`）。これはノードごとに1回求める現行の境界値を、手ごとの判定に改めることである。深さ別の余裕値`FutilityMargin1`〜`3`と、improvingでないときの倍率`NonImprovingFutility1`〜`3`は除く。Stockfishの静かな手のfutility pruningはimprovingを使わないからである。静的評価は、起点のfutility pruningと同じくcorrection historyで補正した値を使う。Stockfishと同じく、枝刈りした手の見積り`futilityValue`が最善値より大きければ最善値を上げる。最善値と見積りの両方が詰み帯と専用の帯の外にある場合に限る点は、Stockfishの`!is_decisive(bestValue) && !is_win(futilityValue)`に当たる。見積りはα以下なので、上げた最善値もα以下に留まり、他の手がαを超えない限り置換表に保存する種別は上界のままである。対象は捕獲手でも王手になる手でもない手であり、followPVのPVノードでは行わない（同1205、1227行）。 |
| late move pruningの範囲（S2） | Stockfishには深さの上限がないので（`src/search.cpp:1199`）、深さの上限3を外し、PVノードにも適用する。S2が変えるのは適用範囲だけであり、前の設計書のF3の`LmpNonImproving`を含む起点の上限式と数え方は維持する。`LmpBase`と`LmpSlope`の開始値は、F3が置き直した起点の既定値を継承する。静かな手だけを数え、成る手とkiller手を数えない決定は、中将棋では捕獲手が多く静かな手の枠を捕獲手が埋めるという診断に基づくので、利用者の決定により残す（[ADR](../adr/count-only-quiet-moves-for-late-move-pruning-and-shrink-the-limit-when-not-improving.md)）。 |
| SEEによる捕獲手の枝刈り（S3） | Stockfishの`177 × depth + captHist × 34/1024`（`src/search.cpp:1222`）に合わせ、余裕値を残り深さに比例する項とcapture historyの項の和にする。深さの上限は外し、PVノードにも適用する。深さ別の`SeeMargin1`〜`3`は除く。Stockfishは捕獲手と王手になる手を同じ枝で扱うので（同1205行）、王手になる静かな手にも同じ余裕値で判定する。その手のcapture historyの項は0とする。minaseの表は取る駒の種類で引き、取る駒のない手の欄を持たないからである。capture historyの項は、表の値に重み/1024を掛けたStockfishの評価値の単位の量を、歩兵価値208で換算して足す。historyの形の見直しが表の上限をStockfishの10692に固定した後は、重みはStockfishの値のまま同じ意味になる。Stockfishは余裕値を切り詰めないが、minaseの`see_prunes`は非負の余裕値を契約とするので、本書のS3とS5は現行の契約で書き、S3の余裕値を0以上に切り詰める。後続の[静止探索の補正と枝刈りの形の見直し](qsearch-forms.md)は、引数を符号付きの閾値`T`へ移す挙動を変えない移行（S3は`T = −max(0, 余裕値)`、S5は`T = −余裕値`）の後、Q2の挙動を変えるコミットでS3の0での切り詰めを外す。本書は切り詰めを外さない。 |
| 捕獲手のfutility pruningの形（S4） | Stockfishの`staticEval + 234 + 247 × lmrDepth + PieceValue[captured] + 134 × captHist/1024 ≤ α`を、王手にならない捕獲手でlmrDepthが8未満のものに適用する（`src/search.cpp:1208-1217`）。定数と傾きは静かな手と別の係数にする。獅子の2段階移動で2枚を取る手は、取る駒の価値を合計する（手の順序付けと同じ量）。置換表の手と相手の最後の王駒を取る手は対象にしない。成る捕獲手も、利用者の決定により残す成る手の保護に従って対象にしない。 |
| 静かな手のSEEによる枝刈り（S5） | Stockfishの`SEE < −23 × max(lmrDepth, 0)²`（`src/search.cpp:1255-1259`）の手を読まない。S1の後に判定し、followPVのPVノードでは行わない。深さの上限は持たない。対象は捕獲手でも王手になる手でもない手である。静かな手のSEEは、動かした駒が到達升で取られて損をする手を見つけるための判定である。現行の`see_prunes`は到達升に駒があることを前提にするので、取る駒の価値を0として交換列を作る判定を`see.rs`に加える。2段階移動の手、成る手、および置換表の手は対象にしない。2段階移動の手は現行のSEEでも判定不能として扱い、成る手は保護する。YaneuraOuは、将棋ではSEEが負の手でもその後に詰む場合があるので慎重に扱うべきだと注記している（`yaneuraou-search.cpp:3685-3686`）が、利用者の決定によりこの項目を含める。 |
| razoringの形（S6） | Stockfishの`allNode && eval < alpha − 342 × depth && !seekMate`（`src/search.cpp:1017-1018`）と同じく、allNodeで、詰みを探す局面でなく、判定用の評価値が`α − 342 × depth`より小さければ、静止探索の値をそのまま返す。深さの上限は持たない。現行の、残り深さ2以下に限ることと、静止探索の値がα以下のときだけ返す確認はやめる。判定用の評価値は、前の設計書のF2が定める、置換表の値で置き換えた評価値（Stockfishの`eval`）であり、F2が不採用なら起点のrazoringの静的評価を使う。深さ別の`RazoringMargin1`〜`2`は除き、深さに比例する1つの係数にする。allNodeは零窓のノードなので、現行の零窓の条件はこの条件に含まれる。 |
| null move pruningの条件と減深量（S7） | Stockfishと同じく、cutNodeで、`staticEval + 50 × priorNMPFailHigh ≥ β − 13 × depth − 47 × improving + 365`かつ`β ≥ −2000`のときだけ試す（`src/search.cpp:1037-1039`）。`priorNMPFailHigh`（以下の式では`n`）は、同じ親の子のうち先に探索したノードで、null moveがβ以上を返して打ち切った回数であり、親のノードを始めるときに0へ戻す（同830、1057、1074行）。現行の深さ3以上という条件は、Stockfishが持たないので外す。直前がnull moveでないこと、βが詰み帯と専用の帯の外であること、および王駒以外の自駒があることは残す。最後の条件はStockfishの`non_pawn_material`に当たる。減深量は、Stockfishの`R = 7 + depth/3 + max((staticEval − β)/256, 0)`（同1044行）に合わせる。minaseはnull moveの子を`d − 1 − 減深量`で読み、子の深さで引く1手は減深量の外にある。このため、減深量の基本量`(NullMoveBase + d × NullMoveSlope)/1200`に子の1手を足した量がStockfishの`7 + depth/3`に当たり、開始値は`NullMoveBase`＝7200、`NullMoveSlope`＝400である。静的評価の項は、現行の`((e − β) × NullMoveEvalScale/100)/(2P)`がStockfishの`(e − β)/256`に等しくなる`NullMoveEvalScale`＝163から始め、現行の上限3を外す。3つの既存の係数の範囲は、Stockfishの換算値と現行の値の両方を含む。子の深さは、本書の減深量を`r`として`R = 1 + r`とし、詰みを探す局面（前の設計書のF6）では`R`から`R/2`を0方向へ丸めて引いたうえで、`max(0, d − R)`とする。F6の半減は、着手1手分を含む`R`に掛かる。 |
| 検証探索（S7） | Stockfishと同じく、null moveの探索がβ以上を返したとき、残り深さが16以上で検証探索中でなければ、手数が後述の値に達するまでnull moveを禁じた零窓`(β − 1, β)`の探索を、同じノードでnull moveの子と同じ深さで行い、その値もβ以上のときだけ打ち切る（`src/search.cpp:1055-1076`）。検証探索の間は、手数が`ply + 3 × (null moveの子の深さ)/4`に達するまでnull moveを試さない。YaneuraOuも同じ形を持つ（`yaneuraou-search.cpp:3278-3297`）。[検証探索を実装しないADR](../adr/omit-null-move-verification-search.md)は、再提案するときは現行の構成で矛盾率を測り直すよう求めているので、フェーズ1で起点の構成と形の変更の構成の矛盾率を測って記録する。測定は記録であり、採否には使わない。 |
| reverse futility pruningの返り値（S8） | 返り値をStockfishの`(661β + 363e)/1024`（`src/search.cpp:1033`）にする。平均の重みなので、利用者の決定により係数にせず固定する。YaneuraOuは`(2β + e)/3`である（`yaneuraou-search.cpp:3227`）。返り値がβ以上`e`以下に収まるように、前の設計書の採否にかかわらず「`e ≥ β`」を独立した条件として要求する。現行の発動条件は「`e` − 余裕値 ≥ β」で余裕値が0以上なので、この条件を加えても現行の判定は変わらない。置換表へ保存しないことは変えない。 |
| 詰みを探す局面でのLMRの王手の項（S9） | Stockfishは、詰みを探す局面で減深量が正なら、王手になる手の減深量を1/4減らし、それ以外の手を1/4増やす（`r += (givesCheck ? −r : r)/4`、`src/search.cpp:1391-1392`）。詰みを探す局面の判定は前の設計書のF6が導入するが、王手になる手の判定は本書のM2が導入するので、この項は本書で入れる。対象はLMRを適用する手であり、減深量は前の設計書のF4が定める1/100手単位の値`r`である。1/4は倍率だが、詰みを探す局面でしか働かず、SPSAの対局では発動が少なく勾配が得にくいので、前の設計書のF6がnull moveの減深を半分にする定数と同じく、Stockfishの値で固定する。F6が不採用なら、この項は働かないので入れない。 |
| PVノードへの適用とfollowPV | Stockfishの浅い枝刈り（`src/search.cpp:1196-1259`）は根以外の全ノードで働くので、S1からS5はPVノードにも適用する。ただし、静かな手のfutility pruningと静かな手のSEEは、followPVのPVノードでは行わない（同1227行）。followPVは、前の設計書のF5がIIRの条件のために導入する、前の反復の主変化を辿るノードの状態を再利用する。F5が起点にない場合だけ、同じ定義で本書が導入する。その定義は、根で真とし、子では「親がfollowPVであり、かつ指した手が前の反復の最善応手列の同じ手数の手であること」である（同791〜794行）。minaseの安全条件は残すので、αとβが詰み帯と専用の帯の外にあることはPVノードでも要求する。 |

## 変更の内容

以下の式で、`P`は探索用の歩兵価値（評価関数の重みファイルが持つ値）、`d`は残り深さ、`e`は各枝刈りが使う静的評価（S6では前の設計書のF2が定める判定用の評価値）、`[条件]`は条件が成り立てば1、成り立たなければ0を表す。
百分率の計算は、既存の係数と同じく0方向へ丸める整数除算で行う。
capture historyの値を`h`とし、重み`w`のcapture historyの項を`H(w) = h × w/1024 × P/208`とする。

### M1　枝刈りの条件を手法ごとに分ける移行

`pruning_node`の判定を、futility pruning、late move pruning、SEEによる捕獲手の枝刈り、reverse futility pruning、およびrazoringのそれぞれの条件へ分ける。
各条件は現行と同じ内容にし、探索ノード数、評価値、および最善手が起点と一致することで検証する。

### M2　王手の判定の前計算

設計判断の表のとおり、ノードごとの前計算と、着手が王手になるかの判定を加える。
判定はS1、S3からS5、およびS9で使う。
M2のコミットは探索の判定を変えないので、探索ノード数の一致で検証する。

### S1　静かな手のfutility pruning

置換表の手、成る手、捕獲手、および王手になる手を除く手について、見積り`v_f`を次の式で求め、lmrDepthが12未満で`v_f ≤ α`なら読まない。

```math
v_f = e + \frac{P \times (\mathrm{FutilityBase} + \mathrm{FutilityLmrDepth} \times \mathrm{lmrDepth} + \mathrm{FutilityAboveAlpha} \times [e > \alpha])}{100}
```

`e`は起点のfutility pruningと同じく補正後の静的評価である。
読まなかった手について、最善値が`v_f`より小さく、最善値と`v_f`がともに詰み帯と専用の帯の外にあれば、最善値を`v_f`に上げる。
followPVのPVノードでは行わない。

### S2　late move pruning

深さの上限3を外し、PVノードにも適用する。
前の設計書のF3の`LmpNonImproving`を含む起点の上限式と数え方は変えず、`LmpBase`と`LmpSlope`の開始値も起点の既定値を継承する。

### S3　SEEによる捕獲手の枝刈り

置換表の手と相手の最後の王駒を取る手を除く捕獲手と、置換表の手と成る手を除く王手になる静かな手について、SEEの値が次の余裕値の符号を反転した値を下回れば読まない。
王手になる静かな手では`h = 0`とする。

```math
m_{\mathrm{SEE}} = \max\left(0,\ \frac{P \times \mathrm{SeeDepthMargin} \times d}{100} + H(\mathrm{SeeCaptureHistory})\right)
```

深さの上限は持たず、PVノードにも適用する。

### S4　捕獲手のfutility pruning

置換表の手、成る手、相手の最後の王駒を取る手、および王手になる手を除く捕獲手について、lmrDepthが8未満で、取る駒の価値の合計を`V`として次の式が成り立てば読まない。

```math
e + \frac{P \times (\mathrm{CaptureFutilityBase} + \mathrm{CaptureFutilityLmrDepth} \times \mathrm{lmrDepth})}{100} + V + H(\mathrm{CaptureFutilityHistory}) \le \alpha
```

判定はS3より前に行う。
読まなかった手もLMRの表を引く手番号（ノード内で何番目の手か）に数え、capture historyの減点の対象にしない点は、現行のSEEによる捕獲手の枝刈りと同じである。

### S5　静かな手のSEEによる枝刈り

S1で枝刈りされなかった静かな手のうち、置換表の手、成る手、王手になる手、および2段階移動の手を除く手について、SEEの値が次の余裕値の符号を反転した値を下回れば読まない。

```math
m_{\mathrm{quiet}} = \frac{P \times \mathrm{QuietSeeMargin} \times \max(\mathrm{lmrDepth}, 0)^2}{100}
```

followPVのPVノードでは行わない。
`QuietSeeMargin`の範囲は0以上なので、この余裕値は常に0以上であり、`see_prunes`の非負の契約を満たす。
SEEは、到達升の駒の価値を0として交換列を作る点を除き、捕獲手のSEEと同じ駒の価値、取り返す駒の成り益、および獅子の規則による判定不能の扱いを使う。
読まなかった手もLMRの表を引く手番号に数え、historyの減点とkiller手の更新の対象にしない点は、現行のfutility pruningとlate move pruningと同じである。

### S6　razoring

allNodeで、前の設計書のF6が定める詰みを探す局面でなく（`allNode && !seekMate`）、判定用の評価値`e`について次の式が成り立てば、静止探索を窓`(α, β)`で呼び、その値をそのまま返す。

```math
e < \alpha - \frac{P \times \mathrm{RazoringDepthMargin} \times d}{100}
```

### S7　null move pruningの条件、減深量、および検証探索

null moveは、次の条件をすべて満たすノードで試す。

- ノードがcutNodeである。
- 直前の手がnull moveでない。
- βが詰み帯と専用の帯の外にある。
- 手番側に王駒以外の駒がある。
- 検証探索中なら、根からの手数が、検証探索を始めたノードの手数にnull moveの子の深さの3/4を足した値以上である。
- `β ≥ −P × 2000/208`が成り立つ。
- 次の式が成り立つ。`n`は、Stockfishの`priorNMPFailHigh`に当たり、同じ親の子のうち先に探索したノードで、null moveが打ち切りを起こした回数である。

```math
e + \frac{P \times \mathrm{NullMoveFailHighBonus} \times n}{100} \ge \beta + \frac{P \times (\mathrm{NullMoveEvalMargin} - \mathrm{NullMoveEvalDepth} \times d - \mathrm{NullMoveEvalImproving} \times [\text{improving}])}{100}
```

減深量`r`は`(NullMoveBase + d × NullMoveSlope)/1200`に`((e − β) × NullMoveEvalScale/100)/(2P)`を0以上に切り詰めて足した値とする。
`R = 1 + r`とし、前の設計書のF6が定める詰みを探す局面では`R`から`R/2`（0方向へ丸める）を引く。
null moveの子は`max(0, d − R)`の深さで読む。
子の値がβ以上なら、残り深さが16未満か検証探索中ならそのまま打ち切り、そうでなければ設計判断の表の検証探索を行う。
このノードがnull moveで打ち切るたびに、親が子のために保持する回数`n`を1増やす。

### S8　reverse futility pruningの返り値

「`e ≥ β`」を独立した条件として要求し、返り値を`(661β + 363e)/1024`とする。

### S9　詰みを探す局面でのLMRの王手の項

前の設計書のF6が詰みを探す局面と判定し、LMRの1/100手単位の減深量`r`が正なら、王手になる手では`r`から`r/4`を引き、それ以外の手では`r/4`を足す。
除算は0方向へ丸める整数除算とし、加える位置はStockfishと同じく、allNodeの項の後で、減深量を手数へ直す前とする。

### 新しい係数と開始値を置き直す係数

開始値は、Stockfishの値を換算して丸めた値である。
評価値の単位を持つ値は、Stockfishでは歩兵価値208、YaneuraOuでは歩兵価値90で、歩兵価値に対する百分率へ換算する。
表の「SF」「YO」「現行」は、それぞれStockfishの換算値、YaneuraOuの換算値、minaseの現行の対応する値である。

| 係数 | 開始値 | 範囲 | SF、YO、現行と範囲の根拠 |
|---|---|---|---|
| `FutilityBase`（定数、歩兵価値に対する百分率） | 79 | 0〜400 | SFは164/208で79、YOは42/90で47、現行は深さ1の余裕値165 |
| `FutilityLmrDepth`（lmrDepth 1あたり、同） | 57 | 0〜300 | SFは119/208で57、YOは120/90で133、現行は深さ1〜3の平均の増分59 |
| `FutilityAboveAlpha`（静的評価がαより大きいときの加算、同） | 43 | −100〜300 | SFは90/208で43、YOは86/90で96、現行はこの項がなく0 |
| `SeeDepthMargin`（残り深さ1あたり、同） | 85 | 0〜400 | SFは177/208で85、YOは167/90で186、現行は深さ別の値10、192、13を深さで割って約4〜96 |
| `SeeCaptureHistory`（capture historyの重み、1/1024） | 34 | −50〜200 | SFは34、YOは34で、歩兵価値で見ると79に当たる（34 × 208/90）。現行は0 |
| `CaptureFutilityBase`（定数、歩兵価値に対する百分率） | 113 | 0〜600 | SFは234/208で113、YOは218/90で242。現行にはこの枝刈りがない |
| `CaptureFutilityLmrDepth`（lmrDepth 1あたり、同） | 119 | 0〜500 | SFは247/208で119、YOは223/90で248 |
| `CaptureFutilityHistory`（capture historyの重み、1/1024） | 134 | −100〜600 | SFは134、YOは131で、歩兵価値で見ると303に当たる（131 × 208/90）。現行は0 |
| `QuietSeeMargin`（lmrDepthの2乗あたり、歩兵価値に対する百分率） | 11 | 0〜100 | SFは23/208で11、YOは25/90で28。現行の「読まない手がない」挙動はこの係数では表せない |
| `RazoringDepthMargin`（残り深さ1あたり、同） | 164 | 0〜3000 | SFは342/208で164。YOは`502 + 306 × depth²`で形が違い、深さ1で約898、深さ2で深さ1あたり約959。現行は深さ別の値1606、2015を深さで割って約1008〜1606。開始値はYOと現行の値の約1/6〜1/10である |
| `NullMoveEvalMargin`（定数、同） | 175 | −1000〜1000 | SFは365/208で175、YOは378/90で420。現行の「条件なし」はこの係数では表せない |
| `NullMoveEvalDepth`（残り深さ1あたりの減算、同） | 6 | −50〜100 | SFは13/208で6、YOは16/90で18。現行はこの条件がない |
| `NullMoveEvalImproving`（improvingのときの減算、同） | 23 | −200〜400 | SFは47/208で23、YOは53/90で59。現行はこの条件がない |
| `NullMoveFailHighBonus`（打ち切り1回あたりの加算、同） | 24 | 0〜200 | SFは50/208で24。YOと現行にはこの項がなく0 |
| `NullMoveBase`（既存、1/1200手） | 7200 | 1200〜10000 | SFとYOは`7 + depth/3`の7から子の1手を除いて6手で7200。現行は3617。範囲の上限を起点の4800から広げる |
| `NullMoveSlope`（既存、深さ1あたり1/1200手） | 400 | 100〜600 | SFとYOは1/3手で400。現行は257。範囲の上限を起点の400から広げる |
| `NullMoveEvalScale`（既存、百分率） | 163 | 0〜400 | SFの`(e − β)/256`は歩兵価値で`(e − β)/(1.23P)`に当たり、163。YOはこの項がなく0。現行は56。現行の値の約3倍から始める |

`CaptureHistoryLimit`は本書の時点では起点の係数のままであり、`h`の大きさはStockfishの表と違う。

### 除く係数

`FutilityMargin1`〜`3`、`NonImprovingFutility1`〜`3`、`SeeMargin1`〜`3`、および`RazoringMargin1`〜`2`の11係数を、表から削除する。

### 固定の定数

次の定数は、利用者の決定によりStockfishの値で固定し、SPSAで調整しない。

| 定数 | 値 | 用途 | Stockfishの行 |
|---|---|---|---|
| 静かな手のfutility pruningのlmrDepthの上限 | 12未満 | S1 | `src/search.cpp:1247` |
| 捕獲手のfutility pruningのlmrDepthの上限 | 8未満 | S4 | `src/search.cpp:1211` |
| reverse futility pruningの返り値の重み | βに661/1024、静的評価に363/1024 | S8 | `src/search.cpp:1033` |
| null moveを試すβの下限 | Stockfishの−2000を換算した`−P × 2000/208` | S7 | `src/search.cpp:1039` |
| 検証探索を行う残り深さの下限 | 16 | S7 | `src/search.cpp:1055` |
| 検証探索中にnull moveを禁じる手数 | null moveの子の深さの3/4 | S7 | `src/search.cpp:1066` |
| 詰みを探す局面でのLMRの王手の項の割合 | 減深量の1/4 | S9 | `src/search.cpp:1391-1392` |

## 所要時間の見積り

時間切れの事前確認、SPSA、事前対局、およびSTCは調整機、反映とテストは測定機で行う。
SPSAは、[統括計画](search-forms.md#所要時間と費用)の見積りにより、調整機で約13〜18時間と見込み、時間切れの事前確認を含む1セッションの費用は約25〜35米ドルである。
STCの時間は、判定までのペア数が強さの差で変わるので、統括計画と同じ幅を持たせた。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 移行と9項目の実装と検証（codexへ委任） | 2日程度 |
| 2 | 時間切れの事前確認（調整機） | 約35分 |
| 3 | SPSA（調整機、30,000ペア） | 約13〜18時間 |
| 4 | 反映とテスト（測定機）、および事前対局（調整機、32ペア） | 約1時間 |
| 5 | 候補のSTC（調整機、採否に使わない記録） | 約0.2〜1.3時間 |

実装が前の設計書より長いのは、王手の判定の前計算、静かな手のSEEの判定、lmrDepthの計算、futility pruningの手ごとの判定、および検証探索を加えるからである。
測定機の占有は、反映とテストの約1時間だけである。
調整機は、時間切れの事前確認からSTCまでで約15〜21時間使い、事前対局とSTCと待ち時間の費用は約2〜3米ドルである。

## 実装フェーズ

基本シードは、時間切れの事前確認を3500000000、セッションを3550000000、事前対局を3600000000、STCを3650000000、LTCを3700000000と仮に定める。
これらは起案時点で、保存済みの測定とSPSAの基本シード（`data/matches/`と`data/spsa/`の`manifest.json`）、および他の設計書が予約した値から、各測定のペア数以上離れた未使用の値である。
隣接する基本シードは1局ずれた同じ対局列を生むので、この間隔を保つ（[教訓](../lessons/derive-seed-adjacent-collision.md)）。
着手時に同じ方法で未使用であることを確かめ直し、使われていれば設計書を書き換える。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ1　移行と9項目の実装

起点からブランチ`shallow-pruning-extensions`を切り、worktree `data/worktrees/shallow-pruning-extensions`で作業する。
M1、M2、S1、S2、S3、S4、S5、S6、S7、S8、S9の順に、1項目を1コミットにする。
M1とM2は挙動を変えない移行なので、`minase dev bench`の全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が起点と一致することを確かめる。
S1からS9の各コミットでは、同じbenchの探索ノード数を記録し、直前のコミットとの差を残す。
lmrDepthの計算はS1に含める。followPVは前の設計書のF5の状態を再利用し、起点にない場合だけ同じ定義でS1に含める。静かな手のSEEの判定はS5に、同じ親の子でのnull moveの打ち切りの回数（`priorNMPFailHigh`）と検証探索はS7に含め、それぞれを最初に使うコミットで実装する。
王手の判定は、benchの局面と固定シードの対局の局面の全ての合法手について、着手後の局面の利きで判定する参照実装と照合する。
S7のコミットでは、検証探索がβを下回った割合（矛盾率）を、起点の構成と形の変更の構成で測って記録する。
benchの深さでは検証探索が働かないので、矛盾率は、STCの事前対局の局面から選んだ局面を残り深さ16以上で探索して数える。
最後のコミットの後、`taskset -c 3`で1つのコアに固定した`minase dev bench --depth 9 --repetitions 3`の中央値を、起点と形の変更で交互に3回ずつ測り、探索速度の変化を記録する（[教訓](../lessons/pin-bench-to-performance-core.md)）。
新しい係数は`params.rs`の表へ追加し、除く11係数は表から削除し、開始値を置き直す3係数は既定値を書き換え、そのうち範囲を広げる`NullMoveBase`と`NullMoveSlope`は範囲も書き換える。
調整用ビルド（`--features tuning`つきのビルド）が新しい係数を`Tune_<係数名>`のUSIオプションとして宣言し、除いた係数を宣言しないことを確かめる。
固定の定数は`pruning.rs`などの名前付きの定数とし、docコメントにStockfishの行を書く。
静かな手のSEEの判定は、既存の`see_prunes`の参照実装による単体テスト（`see.rs`のテスト）と同じ方法で、benchの局面と固定シードの対局の局面の静かな手について、参照値と照合する。
新しい式の単体テストは、式の写しではなく数値の例で書く（[テストの手引き](../guides/testing.md)）。
完了条件は、`cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`の通過、M1とM2のノード数の一致、S1からS9のノード数の変化の記録、矛盾率の記録、および探索速度の記録である。

### フェーズ2　パラメーターファイルと時間切れの事前確認

形の変更の最後のコミットでビルドした`minase`を`data/spsa/shallow-pruning-extensions-runner/`へ複製してrunner（測定を実行するプログラム）として固定し、以後のフェーズはこのrunnerで実行する。
ただし、時間切れの事前確認とSPSAのセッションは[SPSAの手引き](../guides/spsa.md)の調整機の手順で実行し、調整機では同じコミットからビルドしたrunnerを使う。
`minase spsa params --engine commit:<形の変更> --rules engine-default`で既定のパラメーターファイルを生成し、`FmScale`の行を削除したファイルを`data/spsa/shallow-pruning-extensions.params`とする。
行数は、起点の調整対象の係数から11を除き14を加えた数である。
開始値と範囲が形の変更の`params.rs`の表と一致すること、および`c_end`と`r_end`が既定値であることを、使い捨てのスクリプトで機械的に突き合わせる。

続いて、手引きの第3節に従い、時間管理の6係数のうち手引きが指定する4係数を思考時間が最も長くなる側の範囲の端に置いても時間切れが起きないことを確かめる。
写しの開始値を`HardRemainingShare`＝50、`HardSoftRatio`＝800、`MinMoves`＝40、`IterationRatio`＝150へ書き換えた`data/spsa/shallow-pruning-extensions-safety.params`で、[統括計画](search-forms.md#実行の構成)が定める反復数（同時対局数191なら156反復、1,248ペア。同時対局数は調整機ごとに決まる）を調整機で実行する。

```console
<調整機のminase> spsa \
  --run-dir data/spsa/shallow-pruning-extensions-safety --seed 3500000000 \
  --engine commit:<形の変更> --params data/spsa/shallow-pruning-extensions-safety.params \
  --rules engine-default --each <換算した持ち時間> --concurrency <調整機の同時対局数> \
  --iterations <事前確認の反復数> --pairs-per-iteration 8
```

`time_forfeits`（時間切れの件数）が1件でもあれば、[デバッグの手引き](../guides/debugging.md#時間切れと時間管理)の手順で原因を調べ、該当する係数の範囲を狭める案を利用者に示してから本番へ進む。
結果は測定記録`shallow-pruning-extensions-safety`に残す。
完了条件は、時間切れとエンジン異常が0件であることと、測定記録の保存である。

### フェーズ3　SPSAによる調整

```console
<調整機のminase> spsa \
  --run-dir data/spsa/shallow-pruning-extensions --seed 3550000000 \
  --engine commit:<形の変更> --params data/spsa/shallow-pruning-extensions.params \
  --rules engine-default --each <換算した持ち時間> --concurrency <調整機の同時対局数> \
  --iterations 3750 --pairs-per-iteration 8
```

このコマンドは調整機で実行し、実行ディレクトリは測定機の`data/spsa/shallow-pruning-extensions`へ取得してから反映に使う。
最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`shallow-pruning-extensions`に、手引きの第6節の項目とともに残す。
測定記録には、最終値を丸めた整数が範囲の上限または下限になった係数を明記する。
新しい14係数と開始値を置き直した3係数については、最終値が開始値（Stockfishの換算値）からどれだけ離れたかを`c_end`に対する比で示し、YaneuraOuの換算値と現行の値のどちらに近づいたかを並べて示す。
これらは、中将棋で値がStockfishからどれだけ離れるかを読むための記録であり、採否には使わない。
完了条件は、セッションの完了と測定記録の保存である。

### フェーズ4　反映と事前対局

手引きの第7節の手順1に従い、`minase spsa apply`で最終値を表の既定値へ書き込み、`git diff`で既定値だけが変わったことを確かめてコミットする。
続いて`cargo test`（通常ビルドと調整用ビルド）を測定より先に実行し、表の既定値に依存するテストが失敗した場合は、テストの前提が新しい既定値でも成り立つようにテストを直して別のコミットにする。
テストの失敗を、末尾に`; true`を付けるようなコマンドの組み方で握りつぶさない。
候補の調整用ビルドの宣言を`minase spsa params`で出力し、既定値の欄が`apply`の出力と一致することを確かめる。

STCの前に、候補が時間切れやエンジン異常を起こさないことと、時間の使い方の変化を確かめるため、STCの条件で候補と起点の32ペアの事前対局を行う。
事前対局とSTCは、[統括計画](search-forms.md)の設計判断「対局の実行場所」により、SPSAを終えた調整機で、持ち時間を換算せずに行う。候補のコミットは`git bundle`で調整機のcloneへ渡し、`<固定したminase>`は調整機でrunnerのコミットからビルドしたminaseとする。同時対局数は、SPSAと同じ調整機の同時対局数とする。実行ディレクトリは測定機の`data/matches/`へ取得してから集計し、記録する。

```console
<固定したminase> match run \
  --run-dir data/matches/shallow-pruning-extensions-smoke --seed 3600000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency <調整機の同時対局数> elo --pairs 32
```

候補と起点の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ（反復深化で読み終えた深さ）、および探索を止めた理由の分布を、前の設計書の事前対局と同じ方法で集計して、測定記録`shallow-pruning-extensions-smoke`に残す。
同じ記録に、候補と起点の探索速度を前述のbenchで測った値も残す。
完了条件は、テストの通過、事前対局での時間切れとエンジン異常が0件であること、および測定記録の保存である。

### フェーズ5　STC

2026年10月18日の利用者の決定により、本書の変更はSTCの判定によらず採用する（[統括計画](search-forms.md)の設計判断「各サブタスクの採否」）。
STCは、統括計画のフェーズ10で強くなっていなかった場合に、どのサブタスクで後退したかの手がかりにするための記録として行い、LTCは行わない。
STCは、事前対局と同じ調整機で、同じ同時対局数で行う。

```console
<固定したminase> match run \
  --run-dir data/matches/shallow-pruning-extensions-stc --seed 3650000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency <調整機の同時対局数> gsprt --max-pairs 3000
```

逐次検定の停止規則は[SPRTの手引き](../guides/sprt.md)のSTCのとおりとし、判定は記録するが採否には使わない。
採用はLTCの`H1`だけが決めるという同手引きの原則の例外であり、統括計画の[採用の例外のADR](../adr/adopt-search-form-subtasks-regardless-of-stc-and-check-the-whole-chain-by-ltc.md)に従う。
予約した基本シード3700000000は使わない。
結果は測定記録`shallow-pruning-extensions-stc`に残す。
完了条件は、STCの判定と測定記録の保存である。

## 検証

- M1とM2のコミットで、`minase dev bench`の全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。
- S1からS9の各コミットで、同じbenchの探索ノード数の変化が記録されていること。
- S9の減深量の加減が、詰みを探す局面の王手になる手とそれ以外の手について、数値の例による単体テストで確かめられていること。
- 王手の判定が、benchの局面と固定シードの対局の局面の全ての合法手について、着手後の局面の利きによる参照実装と一致すること。
- 静かな手のSEEの判定が、benchの局面と固定シードの対局の局面の静かな手について、参照実装の値と一致すること。
- 検証探索の矛盾率が、起点の構成と形の変更の構成で記録されていること。
- 新しい14係数が調整用ビルドの`Tune_`オプションとして宣言され、除いた11係数が宣言されず、パラメーターファイルの開始値と範囲が表と一致すること。
- 候補の`params.rs`の差分が既定値の書き換えだけであり、調整用ビルドの宣言の既定値が`apply`の出力と一致すること。
- `cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`が通過すること。
- 時間切れの事前確認、SPSA、事前対局、およびSTCで、エンジン異常、時間切れ、および拒否着手が0件であること。最初の測定で1件でも出た場合は、測定を止めて原因を調べる。

## 完了条件

- 形の変更の検証結果、矛盾率、時間切れの事前確認、SPSAの結果、事前対局、およびSTCが、測定記録に残っている。
- 候補のブランチがmasterへ統合され、依存関係の節に挙げたADRが新しいADRに置き換えられている。統合は利用者の許可を得てから行う。
- ROADMAPの状態表が本書の結論と一致している。

## 参考資料

- [探索と時間管理の係数の関数形に関する既存実装との比較](../research/search-coefficient-forms-prior-art.md)と[前向き枝刈りの第3層に関する既存実装の調査](../research/forward-pruning-prior-art.md)。
- Stockfish（コミット5c4cef0b）の`src/search.cpp`。followPVは791〜794行、`priorNMPFailHigh`の初期化は830行、razoringは1017〜1018行、reverse futility pruningの返り値は1033行、null move pruningを試す条件は1037〜1039行、減深量は1044行、打ち切りと検証探索は1052〜1076行、枝刈りの前の減深量は1183〜1192行、浅い枝刈りの前提条件は1196行、late move pruningは1199行、lmrDepthは1203行、捕獲手と王手になる手の枝は1205行、捕獲手のfutility pruningは1208〜1217行、捕獲手のSEEは1220〜1225行、静かな手の枝の条件は1227行、historyによるlmrDepthの増減は1229〜1240行、静かな手のfutility pruningは1242〜1252行、静かな手のSEEは1255〜1259行、LMRの定数697は1350行、`reduction()`は1917〜1920行。王手の判定は`src/position.cpp`の731〜774行、capture historyの上限10692は`src/history.h:135`、歩兵価値208は`src/types.h:199`。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`。razoringは3191〜3192行、reverse futility pruningは3214〜3227行、null move pruningを試す条件は3236〜3245行、減深量は3252行、検証探索は3278〜3297行、浅い枝刈りの前提条件は3585〜3588行、late move pruningは3593行、lmrDepthは3600行、捕獲手のfutility pruningは3611〜3621行、捕獲手のSEEは3631〜3641行、historyによるlmrDepthの増減は3661行、静かな手のfutility pruningは3663〜3677行、静かな手のSEEは3684〜3697行。歩兵価値90は`source/evaluate.h:78`。
- [RULES.md](../rules/RULES.md)の第8条と第23条。前者は王手を、後者は合法手がない場合を定める。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)と[SPRTによる棋力測定の手引き](../guides/sprt.md)。
