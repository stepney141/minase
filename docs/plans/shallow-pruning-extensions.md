# 浅い前向き枝刈りの深さと対象の拡張の設計書

## 先に読む要約

本書は、探索の浅いノードで働く前向き枝刈り（読む価値が低いと見込んだ手やノードを読まずに済ませる枝刈り）を、より深いノードと、いまは対象にしていない手へ広げ、着手時点のmasterである起点より強くなるかを確かめる。
起点は、探索の関数形を両エンジンの形へ寄せる一連の設計書で1つ前に当たる[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)が完了した後のmasterである。
futility pruningは、静的評価に余裕値を足してもαに届かない静かな手を読まない枝刈りであり、late move pruningは、後方の静かな手を読まない枝刈りである。
SEE（static exchange evaluation）は、着手の到達升での駒の取り合いの損得の見積りである。
minaseでは、futility pruning、late move pruning、およびSEEによる捕獲手の枝刈りを、残り深さ3以下のノードに限っている。
チェスのStockfishと将棋のYaneuraOu（やねうら王）では、これらはより深いノードでも働き、捕獲手のfutility pruningと静かな手のSEEによる枝刈りも持つ。
両エンジンとの違いは[係数の関数形の比較](../research/search-coefficient-forms-prior-art.md)にまとめてある。
本書は7項目を扱う。
1つ目に、futility pruning、late move pruning、およびSEEによる捕獲手の枝刈りの深さの上限を係数にし、深さ4以上の余裕値を深さ3の値からの1次の外挿で与える。
2つ目に、SEEによる捕獲手の枝刈りの余裕値に、capture history（捕獲手の成績を学習する表）の項を加える。
3つ目と4つ目に、捕獲手のfutility pruningと静かな手のSEEによる枝刈りを新設する。
5つ目に、razoring（静的評価がαを大きく下回るノードを静止探索だけで判定する枝刈り）の深さの上限を係数にする。
6つ目に、null move pruning（手番を相手に渡しても打ち切れるノードを浅く読んで打ち切る枝刈り）を試す条件に静的評価の余裕を加える。
7つ目に、reverse futility pruning（静的評価がβを十分に上回るノードを読まずに返す枝刈り）の返り値を、βと静的評価の重み付き平均にする。
6つ目を除く項目の新しい係数は現行と同じ探索になる値を開始値とし、6つ目の条件はStockfishの値を換算した開始値から始める。
これらを既存の係数とまとめてSPSA（simultaneous perturbation stochastic approximation、全係数を同時に少しずらした2つの設定を対局させて係数を動かす調整法）で調整する。
調整した版を、起点のmasterと短時間条件STCおよび長時間条件LTCの2段階の逐次検定（段階ゲート）で対局させ、1つの候補として採否を決める。
完了条件は、候補の採否が測定記録とともに確定することである。

## 状態

起案。2026年10月17日に起案した。
[探索の枝刈りとLMRの関数形の見直し](search-pruning-forms.md)の完了を待って着手し、着手は利用者の指示による。
次の一手は、前の設計書の完了後に起点を確定し、フェーズ1（7項目の実装）を始めることである。

## 目的

本書が確かめるのは、「浅い前向き枝刈りを深いノードと新しい対象へ広げ、広げ方の係数を調整すれば、現行の範囲で調整した探索より強くなる」という仮説である。
minaseの浅い前向き枝刈りの深さの上限3は、[棋力向上段階4](archived/strength-stage4.md)で静的評価に基づく枝刈りを入れた時点の値であり、その後の段階で加えたSEEによる捕獲手の枝刈りとlate move pruningも同じ上限を引き継いだ。
いずれも、評価関数がPSTとFMだった時期の判断である。
PSTは駒の種類と升ごとの点数の表、FMは2駒の関係の補正である。
Stockfishのlate move pruningとSEEによる捕獲手の枝刈りには深さの上限がなく、静かな手のfutility pruningは減深後の深さ12未満で働く（`src/search.cpp:1199-1258`）。
[late move pruningのADR](../adr/count-only-quiet-moves-for-late-move-pruning.md)も、全深さへ適用を広げる変更を未測定の候補として残している。
深さの上限を係数にすれば、どの深さまで枝刈りを使うかをSPSAに選ばせられる。
新設する2つの枝刈りとcapture historyの項は、両エンジンが持ち、minaseにない依存変数と対象を補う。
razoringは、2026年10月17日に中止した現行の形でのSPSAの途中で、`RazoringMargin2`が範囲の上限に達し、`RazoringMargin1`も上限に近づいた。
余裕値が大きいほどrazoringは働かないので、これは、SPSAが範囲の中で最もrazoringが働かない値を選ぼうとした動きであり、NNUEの下でrazoringが役に立っていない兆候と読める。
現行の余裕値の範囲には、razoringを完全に止める値がない。
深さの上限を係数にして0を範囲に含めれば、razoringを止める選択もSPSAに委ねられる。

## 適用範囲

本書の用語を次のように定める。

- **起点**は、[前の設計書](search-pruning-forms.md)が完了した後、フェーズ1を始める時点のmasterの先頭のコミットである。前の設計書の候補が採用されていれば、その4項目と調整値を含む。
- **形の変更**は、本書の7項目を実装したコミット列であり、新しい係数を開始値に置く。
- **候補**は、形の変更の上に、SPSAの最終値を`minase spsa apply`で表（`crates/minase/src/search/alphabeta/params.rs`）の既定値へ書き込んだコミットである。
- **STC**と**LTC**は、それぞれ10秒＋1手0.1秒と60秒＋1手0.2秒の持ち時間で行う自己対局の採否測定である（[SPRTの手引き](../guides/sprt.md)）。
- **残り深さ**は、置換表の手がないノードで深さを1減らす内部反復減深（IIR）を適用した後の深さであり、探索の深さ1を1手と数える。
- **零窓のノード**は、探索窓の幅β − αが1のノードである。窓の幅が1より大きいノードをPVノードという。
- **詰み帯と専用の帯**は、詰みまでの手数を表す評価値の範囲と、千日手などの反復の結果を表す評価値の範囲（`crates/minase/src/search/mod.rs`の`MATE_THRESHOLD`と`REPETITION_BAND_START`）である。
- **improving**は、起点の`negamax.rs`の`improving`が定める判定であり、同じ手番の2手前の局面より静的評価が上がったことをいう。
- **浅い枝刈りの共通条件**は、零窓のノードであること、αとβが詰み帯と専用の帯の外にあること、および手番側のどの王駒にも相手の疑似利きが届かないことをいう（[ADR](../adr/skip-static-eval-pruning-when-a-royal-is-attacked.md)）。手を読まずに飛ばす枝刈りは、さらに、そのノードで探索済みの最善値が負の詰み帯と専用の帯の外にあることを要求する。

対象は次の7項目である。

- futility pruning、late move pruning、およびSEEによる捕獲手の枝刈りの深さの上限と、深さ4以上の余裕値（S1）。
- SEEによる捕獲手の枝刈りの余裕値のcapture historyの項（S2）。
- 捕獲手のfutility pruning（futility pruning for captures、S3）。
- 静かな手のSEEによる枝刈り（S4）。
- razoringの深さの上限（S5）。
- null move pruningを試す条件の静的評価の余裕（S6）。
- reverse futility pruningの返り値の重み付き平均（S7）。

次の事項は対象外とする。

- 王駒に相手の利きが届くノードを浅い枝刈りから除く条件。中将棋では王駒を相手の利きへ動かす手も王手を放置する手も合法であり（RULES.md第8条）、王駒が取られ得る局面の静的評価は、最後の王駒の捕獲を詰みの値とする探索値から大きく外れる。この除外は規則に由来する判断なので、係数の調整の対象にしない（[ADR](../adr/skip-static-eval-pruning-when-a-royal-is-attacked.md)）。
- Stockfishが静かな手のfutility pruningとSEEに使う減深後の深さ（lmrDepth）への置き換え。理由は設計判断の表に書く。同じ理由で、静かな手のfutility pruningを手ごとの判定に改めること、Stockfishの`90 × (staticEval > α)`の項、および枝刈りした手の見積りで最善値を更新する処理も扱わない。
- continuation historyの値が低い静かな手を読まない枝刈り（Stockfish `src/search.cpp:1234-1235`）。continuation historyは直前の手を文脈とするhistoryであり、minaseはこの表を持たず、表の導入は[historyの形の見直し](history-forms.md)が扱う。
- late move pruningとSEEによる枝刈りをPVノードへ広げること。両エンジンはPVノードでも使うが、本書は深さの方向の拡張に限り、浅い枝刈りの共通条件は変えない。
- razoringの、allNode（どの子もβ打ち切りを起こさないと見込まれるノード）に限って静止探索の値をそのまま返す形。理由は設計判断の表に書く。
- null move pruningを試す条件のcutNode（β打ち切りが見込まれるノード）と、直前のnull moveの成否の項。minaseの探索は、ノードの種別を予測する状態を持たない。ノード種別の予測は別の土台を要する（[ADR](../adr/allow-null-move-and-tt-cutoff-at-pv-nodes.md)）。
- reverse futility pruningの置換表に関する条件（Stockfishの`ttPv`、置換表の手が静かな手なら行わない条件、置換表のヒットによる余裕値の倍率）。minaseの置換表はPVの印を持たず、本書の項目と独立に測れる。
- 損な先頭の捕獲手よりkiller手を先に返す手の順序。killer手は、同じ手数の別のノードでβ打ち切りを起こした静かな手である。対象は`ordering.rs`の`MovePicker::reset`の`killers_before_losing_capture`である。残り深さ3以下という条件は、SEEによる枝刈りの深さとは別に、手の順序付けの[診断](../measurements/move-ordering-diag.md)で決めたものであり（[ADR](../adr/try-killers-before-a-losing-first-capture.md)）、本書のS1はこの条件を変えない。
- Stockfishの捕獲手のSEEによる枝刈りにある、最後の駒を捨てる手の除外。これは、合法手のない側を引き分けとするチェスのステイルメイトを狙う手を残すための条件であり、中将棋では合法手のない側が負けなので（RULES.md第23条）、対応する事情がない。YaneuraOuもこの部分を外し、αが引き分けの値以上という条件だけを残している（`yaneuraou-search.cpp:3633-3640`）。

## 依存関係

起点は、[前の設計書](search-pruning-forms.md)が完了した後のmasterの先頭である。
前の設計書の候補が採用されたかどうかにかかわらず、起点の表の既定値をSPSAの開始値とする。
前の設計書の候補がmasterへ統合されると`params.rs`の既定値と浅い枝刈りの判定が変わり、本書の検証（起点との探索ノード数の一致）の基準も変わるので、実装を含めて前の設計書の完了後に始める。
前の設計書は、reverse futility pruning、razoring、null move pruning、およびimprovingの判定に補正後の静的評価を使い、reverse futility pruningの深さの上限を係数`ReverseFutilityMaxDepth`にする。
本書の各項目は、どちらの結果でも成り立つように、起点でそれぞれの枝刈りが使っている静的評価、improving、および深さの条件をそのまま使う。
連鎖の順序は、前の設計書の後に本書、その後に[historyの形の見直し](history-forms.md)と続く提案であり、最終的な順序は利用者が決める。

SPSAの手順と既定の設定は[SPSAによる係数調整の手引き](../guides/spsa.md)に、採否の手順は[SPRTの手引き](../guides/sprt.md)に従う。
測定機はlishogiのBotと共用するので、同時対局数は16を上限とする。
実装はcodexへ委任する（[委任の手引き](../guides/codex-delegation.md)）。

本書は、次のADR（設計判断の記録、`docs/adr/`）の決定の一部を変える。
いずれも、既定の評価関数がNNUEへ置き換わった2026年10月12日より前、評価関数がPSTとFMだった時期の測定に基づく。
採用した場合は、完了時に、起点の時点で有効なADR（先行の設計書が置き換えていればその後継）を確認し、新しいADRで置き換える。
部分的に変える場合は、残る決定も新しいADRへ引き継ぐ。

- [浅いノードの前向き枝刈りの適用条件のADR](../adr/skip-static-eval-pruning-when-a-royal-is-attacked.md)は、2026年9月6日に、futility pruning、late move pruning、SEEによる捕獲手の枝刈り、およびreverse futility pruningを残り深さ3以下に、razoringを2以下に限り、reverse futility pruningは静的評価をそのまま返すと決めた。返り値をβへ丸める案は、上位ノードが得る情報を減らすとして棄却している。S1とS5は深さの上限を係数にし、S7は返り値を静的評価とβの間の値にして、その重みをSPSAに選ばせる。王駒への利きによる除外は変えない。ADRは再測定を留保していないが、返り値の正しさは確かめておらず、窓の外の値もそのまま返すfail-softの近似として扱う、と記している。
- [SEEによる捕獲手の枝刈りのADR](../adr/prune-losing-captures-by-see-at-shallow-nodes.md)は、2026年9月21日に、対象を捕獲手に限り、余裕値を深さ1〜3の深さ別の値にすると決めた。値は、良い結果を失う割合の診断で選び、後にSPSAで調整した。S1は深さの上限と深さ4以上の余裕値を、S2はcapture historyの項を加え、S4は静かな手を対象に加える。ADRは再測定を留保していない。
- [late move pruningの数え方のADR](../adr/count-only-quiet-moves-for-late-move-pruning.md)は、2026年10月4日に、静かな手だけを数える上限式を決めた。全深さへ適用を広げる変更は未測定の候補として残すと、ADR自身が記している。S1は深さの上限を係数にする。
- [null moveをPVノードでも行うADR](../adr/allow-null-move-and-tt-cutoff-at-pv-nodes.md)は、2026年9月28日に、null move pruningをノードの種別と静的評価のβとの大小によらずに試すと決めた。棄却したのは「非PVノードに限り、かつ補正後の静的評価がβ以上」を全深さに課す案である。S6は、ノードの種別を条件にせず、深さ、improving、および定数の項を係数に持つ、別の形の条件を加える。ADRは、ノード種別の予測を持ったうえで条件を加える方式を未測定と記している。
- [improvingの使い道のADR](../adr/use-improving-only-for-futility-margins.md)は、2026年10月4日に、improvingをfutility pruningの余裕値にだけ使うと決めた。前の設計書が適用先を広げ、S6はさらにnull move pruningを試す条件に使う。ADRは再測定を留保していない。
- [capture historyのADR](../adr/bound-capture-history-and-use-it-in-qsearch.md)は、2026年10月10日に、capture historyを通常探索と静止探索の手の順序に使うと決めた。決定は他の用途を禁じていないが、S2とS3は枝刈りの判定にも使うので、新しいADRに用途を記録する。

## 設計判断

| 項目 | 決定 |
|---|---|
| 候補の束ね方 | 7項目を1つの候補とし、既存の係数とまとめて1回のSPSAで調整してから、1回の段階ゲートで採否を決める。[不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)が8項目をこの方式で採用した先例に従う（[ADR](../adr/adopt-revived-search-items-as-a-bundle.md)）。項目ごとにSPSAと段階ゲートを繰り返す案は、1項目あたりSPSAの約75時間にSTCとLTCを加えた約81〜113時間を要するので採らない。代償として、項目ごとの寄与は測らない。不採用のときに一部の項目だけを選び直して再測定しない。 |
| 新しい係数の開始値 | S6を除く6項目は、開始値で現行と同じ探索になる。深さの上限は現行の値（futility pruning、late move pruning、およびSEEは3、razoringは2）とし、新設する2つの枝刈りは深さの上限0、重みは0を開始値とする。深さの上限が現行の値か0である間は、深さ4以上の傾きと、新設する枝刈りの余裕値と重みは探索に影響しない。S6は現行と同じ挙動の開始値を持たない項目であり、理由と開始値の根拠はS6の行に書く。これらの係数の開始値は、現行の深さ別の値の平均の傾き、両エンジンの値の換算、または0のいずれかで、係数ごとに新しい係数の表に根拠を書く。 |
| 開始値を範囲の下端に置く係数 | `CaptureFutilityMaxDepth`、`QuietSeeMaxDepth`、および`ReverseFutilityBetaWeight`の3係数は、開始値を範囲の下端に置く。前の2係数は、0より小さい値がすべて0と同じ「働かない」挙動になり、範囲を下へ広げても平らな区間が増えるだけだからである。この2つの深さの上限は、働かない状態から始めるためだけの装置ではない。minaseの浅い枝刈りはすべて深さの上限を持ち、両エンジンも捕獲手のfutility pruningに減深後の深さ8未満または7未満の上限を置くので、新設する枝刈りも同じ構造にそろえる。`ReverseFutilityBetaWeight`は、S7の返り値の式で負の値をとると静的評価より大きな値を返すことになり、評価関数の出力の範囲（`EVALUATION_LIMIT`）を超え得るからである。SPSAの摂動は範囲の下端で切り詰められ、[利得のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)はこの切り詰めを許容している。その他の新しい係数は、開始値を範囲の内側に置く。 |
| 新しい係数の範囲 | 両エンジンの対応する値を範囲に含める。minaseの余裕値は歩兵価値に対する百分率で、中将棋の歩兵は他の駒に比べて価値が小さいので、余裕値は大きな数になる（現行の`RazoringMargin2`は2015）。このため、範囲の上限は両エンジンの換算値ではなく、minaseの現行の値の数倍まで届くように取る。 |
| SPSAの対象 | 起点の調整対象の係数（`FmScale`を除く全係数）に、新しい16係数を加える。起点の数は前の設計書の候補を採用していれば46、不採用なら38なので、合計は62または54係数になる。全先行案を採用した場合の係数の数は、本書の時点で62前後であり、着手時に起点の表で数え直す。新しい係数だけを調整する案は、既存の係数との相互作用を検証の外に置くので採らない（先例のADRと同じ理由）。時間管理の6係数を含めるので、本番の前に時間切れの事前確認を行う。 |
| SPSAの設定 | 既定の設定を流用する。3,750反復、1反復8ペアで計30,000ペアとする。ペアは同じ開始局面から先後を入れ替えて指す2局の組である。持ち時間は10秒＋0.1秒、同時対局数は16とし、`c_end`を範囲の1/6、`r_end`を0.002とする。`c_end`は最後の反復の摂動幅、`r_end`は最後の反復の学習率であり、値は[SPSAの手引き](../guides/spsa.md)の既定による。30,000ペアは候補を1つ作るための予算であり、最終値を最適値とは呼ばない。総ペア数は、[利得と規模のADR](../adr/decay-spsa-gains-over-30000-pair-sessions.md)が定める標準の30,000ペアである。30,000ペアで62前後の係数を十分に調整できるかは未検証であり、不採用は、この予算と設定で改善を示さなかったことを意味する。予算を変える場合は開始前に決め、途中で延長しない（[反復予算の調査](../research/spsa-iteration-budget.md)）。 |
| 深さの条件を枝刈りごとに分ける | 現行は、浅い枝刈りの共通条件に「残り深さ3以下」を含め、1つの判定でfutility pruning、late move pruning、SEEによる捕獲手の枝刈り、reverse futility pruning、およびrazoringの対象ノードを決めている（`negamax.rs`の`pruning_node`）。SEEによる枝刈りも、futility pruningの境界値があるかどうかでこの判定を流用している。本書は、深さ以外の共通条件を1つの判定に残し、深さの条件をそれぞれの枝刈りの上限の係数で判定する形に改める。静的評価は、静的評価を使う枝刈りのいずれかが働き得る深さで求める。correction historyは、探索値と静的評価のずれを学習して静的評価を補正する表である。静的評価を求めるノードが増えても、`update_correction`は静的評価がなければその場で求めるので、この表の更新は変わらない。王駒への利きの判定も、従来どおり枝刈りが発動し得るときに限って行う。深さの上限が3を超えると、この2つの計算が増えるので、費用は探索速度（NPS、1秒あたりの探索ノード数）の低下として現れる。 |
| 深さ4以上の余裕値（1次の外挿） | futility pruningとSEEによる捕獲手の枝刈りは、深さ1〜3の深さ別の係数を残し、深さ4以上を深さ3の値に傾きの係数と`(d − 3)`の積を足した値とする。深さ別の係数を深さの式へ置き換える案は、現行の値を開始値にできず、既存の係数の意味が変わるので採らない。futility pruningのimprovingでないときの倍率は、深さ4以上でも深さ3の`NonImprovingFutility3`を使う。深さへの依存は傾きが受け持つので、倍率の係数を増やさない。傾きの開始値は、起点の深さ1と深さ3の係数の差の半分を丸めた値とし、深さ1〜3の平均の傾きで延ばす。現行のSEEの余裕値は深さ1、2、3で10、192、13であり、深さについて単調でないので、傾きの範囲は負の値を含める。深さの上限が3の間は傾きに勾配が生じないので、傾きは深さの上限の摂動が4以上になった反復でだけ学習される。これは束ねた調整の既知の制約として受け入れる。 |
| 減深後の深さ（lmrDepth）を使わない | LMR（late move reductions）は、後方の手を浅く読む減深である。StockfishとYaneuraOuは、静かな手のfutility pruning、捕獲手のfutility pruning、および静かな手のSEEに、LMRの減深量を引いた深さを使う（Stockfish `src/search.cpp:1203`、YaneuraOu `yaneuraou-search.cpp:3600`）。本書は残り深さを使う。理由は3つある。1つ目に、minaseのLMRは置換表の手、捕獲手、およびkiller手を除く静かな手にしか減深量を定めず、減深量は枝刈りの判定の後に求める（`negamax.rs`の`lmr_reduction`の呼び出し）。2つ目に、minaseのfutility pruningはノードごとに1回求める境界値で判定するので、lmrDepthを使うには手ごとの判定へ改める必要がある。3つ目に、減深量の式は前の設計書が変え、次の[historyの形の見直し](history-forms.md)も対象を広げる予定なので、lmrDepthに依存させると枝刈りの意味がそれらの採否で変わる。StockfishのlmrDepthはさらにcontinuation historyの値で増減し（`src/search.cpp:1237-1240`）、minaseにはその表がない。 |
| 捕獲手のfutility pruningの形 | 静かな手のfutility pruningと同じノードの境界値（静的評価にcorrection historyの補正とfutility pruningの余裕値を足した値）に、取る駒の価値の合計とcapture historyの項を足し、α以下なら読まない。両エンジンは捕獲手に静かな手と別の定数と傾きを持つが（Stockfish `src/search.cpp:1211-1217`、YaneuraOu `yaneuraou-search.cpp:3611-3621`）、本書は余裕値の係数を新設せず、静かな手の余裕値を当てはめる。係数の総数を抑えるためであり、両エンジンでも捕獲手の余裕値は静かな手の余裕値と同じ桁にある。補正を含む境界値を使うのは、[correction historyのADR](../adr/key-correction-history-by-material.md)がfutility pruningに補正を使うと決めた範囲に、捕獲手のfutility pruningも含まれるからである。獅子の2段階移動で2枚を取る手は、取る駒の価値を合計する（手の順序付けと同じ量）。成る捕獲手は、静かな手のfutility pruningが成る手を保護するのと同じ理由で対象にしない。置換表の手と相手の最後の王駒を取る手も対象にせず、浅い枝刈りの共通条件を要求する。両エンジンが除く王手になる手は除かない。minaseの浅い枝刈りは、王手になる手を区別していないからである。 |
| SEEの余裕値のcapture historyの項 | capture historyの値`h`を、手の順序付けの換算（`CaptureHistory::adjustment`）と同じく`CaptureHistoryLimit`で割って歩兵価値の百分率へ換算し、重み`SeeCaptureHistory`を掛けて余裕値に足す。余裕値は0以上に切り詰める。YaneuraOuは`max(…, 0)`で切り詰め、Stockfishは切り詰めない（YaneuraOu `yaneuraou-search.cpp:3631`、Stockfish `src/search.cpp:1222`）。minaseの`see_prunes`は非負の余裕値を契約としているので、YaneuraOuの形に従う。現行の余裕値は0以上なので、重み0では探索が変わらない。 |
| 静かな手のSEEによる枝刈り | 静かな手のうち、SEEの値が`−P × QuietSeeMargin × d²/100`を下回る手を読まない（`P`は探索用の歩兵価値、`d`は残り深さ）。Stockfishは`−23 × lmrDepth²`、YaneuraOuは`−25 × lmrDepth²`を使う（Stockfish `src/search.cpp:1258`、YaneuraOu `yaneuraou-search.cpp:3697`）。本書は前述の理由で残り深さを使う。lmrDepthは減深量を引く一方で、正のhistoryの値でも増えるので（Stockfish `src/search.cpp:1240`、YaneuraOu `yaneuraou-search.cpp:3661`）、残り深さとlmrDepthの大小は手によって入れ替わる。このため、残り深さへの置き換えで枝刈りが両エンジンより控えめになるとは言えず、置き換えの影響は未検証の仮説として扱う。開始値は深さの上限`QuietSeeMaxDepth`を0として働かない状態にする。余裕値の係数を0にしても、SEEが負の静かな手をすべて読まなくなるので、働かない状態にはならない。静かな手のSEEは、動かした駒が到達升で取られて損をする手を見つけるための判定である。現行の`see_prunes`は到達升に駒があることを前提にするので、取る駒の価値を0として交換列を作る判定を`see.rs`に加える。2段階移動の手、成る手、および置換表の手は対象にしない。2段階移動の手は現行のSEEでも判定不能として扱い、成る手は静かな手のfutility pruningと同じく保護する。killer手は、futility pruningと同じく対象にする。YaneuraOuは、将棋ではSEEが負の手でもその後に詰む場合があるので慎重に扱うべきだと注記している（`yaneuraou-search.cpp:3685-3686`）。詰みを見逃す損が自己対局の勝敗に現れるなら、SPSAは深さの上限を0の近くに留め、段階ゲートでも改善を示せない判定として現れるので、項目に含めて対局で判断する。 |
| razoringの深さの上限 | 深さの上限を係数`RazoringMaxDepth`とし、範囲に0を含める。0ならrazoringは働かない。深さ3以上の余裕値は、`RazoringMargin2`に傾き`RazoringDepth`と`(d − 2)`の積を足す。傾きの開始値は、起点の`RazoringMargin2`と`RazoringMargin1`の差とする。上限が0へ動いて採用された場合、それはrazoringを外すことにあたる。[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)は、8項目の1つを外すには外した版が現行に劣らないことを非劣性の測定で示すよう求めている。本書の候補は起点に対して段階ゲートで`H1`を示して初めて採用されるので、非劣性より強い条件を満たす。 |
| razoringの返し方は変えない | Stockfishは、どの子もβ打ち切りを起こさないと見込まれるallNodeに限って静止探索の値をそのまま返し、YaneuraOuは非PVノードで同じことをする（Stockfish `src/search.cpp:1017-1018`、YaneuraOu `yaneuraou-search.cpp:3191-3192`）。minaseは静止探索の値がα以下のときだけ返し、そうでなければ通常の探索を続ける。minaseの探索はcutNodeとallNodeを区別しないので、Stockfishの形はそのままでは移せない。YaneuraOuの形は、minaseのrazoringが零窓のノードに限られることから移せるが、静止探索がβ以上を返したノードも打ち切るように挙動を変える。係数で連続的につなげず、現行と同じ挙動の開始値を持てないので、本書では扱わない。 |
| null move pruningを試す条件 | 静的評価が`β + P × (NullMoveEvalMargin − NullMoveEvalDepth × d − NullMoveEvalImproving × [improving])/100`以上でなければnull moveを試さない。静的評価は、起点のnull move pruningが減深量の計算に使う値とする。両エンジンの条件`staticEval ≥ β − c₁ × depth − c₂ × improving + c₃`（Stockfish `src/search.cpp:1037-1039`、YaneuraOu `yaneuraou-search.cpp:3236`）は、どの値を選んでも一部のノードで試すことを止めるので、現行と同じ挙動になるのは閾値を評価値の範囲より下へ置いた場合だけである。その値は、静的評価の上限`EVALUATION_LIMIT`の28,999と、専用の帯の手前でβが取り得る絶対値29,486の和である約58,000を超える必要があり、範囲をそこまで広げると摂動幅も広がり、閾値が効く値へ届くまで勾配が生じない。条件を働かなくするためだけの深さの上限も置かない。そこでS6は、現行と同じ挙動の開始値を持たない項目として扱う。開始値は、Stockfishの`β − 13 × depth − 47 × improving + 365`を歩兵価値208で百分率に換算した値（定数175、深さ6、improving 23）とする。YaneuraOuの換算値（定数420、深さ18、improving 59）より緩いので、棄却済みの案に近い側から始められる。S6のコミットは他の6項目の後に置き、固定の15局面を探索する`minase dev bench`の探索ノード数の変化を記録する。 |
| 棄却済みのnull moveの条件との違い | 旧ADRで棄却した案は、非PVノードに限り、全深さで補正後の静的評価がβ以上であることを求めた。S6はノードの種別を条件にせず、定数、深さ、およびimprovingの項をSPSAに選ばせる。両エンジンの条件は棄却済みの案より緩いわけではない。Stockfishは残り深さ3のimprovingでないノードで静的評価がβより約1.6歩兵（326/208）高いことを求め、βと等しい閾値まで下がるのは深さ28付近である。両エンジンが厳しい条件で成り立つのは、cutNodeに限って試すからだと推測される。minaseはcutNodeを区別しないので、開始値の条件は両エンジンより多くのノードで試すことを止め、棄却済みの案と同じく弱くなるおそれがある。このため、定数の範囲は負の値を含め、SPSAが条件を緩める方向へも動けるようにする。 |
| reverse futility pruningの返り値 | 返り値を`e − (e − β) × ReverseFutilityBetaWeight/100`とする（`e`は判定に使った静的評価）。重み0で静的評価を、100でβを返す。βの重みは、Stockfishが661/1024で約65%、YaneuraOuが2/3で約67%である（Stockfish `src/search.cpp:1033`、YaneuraOu `yaneuraou-search.cpp:3227`）。reverse futility pruningは`e ≥ β`のノードでしか働かないので、返り値はβ以上`e`以下に収まり、詰み帯と専用の帯へ入らない。本書は、前の設計書の採否にかかわらず、「`e ≥ β`」をreverse futility pruningの独立した条件として要求する。現行では発動条件「静的評価 − 余裕値 ≥ β」の余裕値が0以上なので、この条件を加えても探索は変わらない。置換表へ保存しないことは変えない。 |

## 変更の内容

以下の式で、`P`は探索用の歩兵価値（評価関数の重みファイルが持つ値）、`d`は残り深さ、`[条件]`は条件が成り立てば1、成り立たなければ0を表す。
百分率の計算は、既存の係数と同じく0方向へ丸める整数除算で行う。

### S1　浅い枝刈りの深さの上限と深さ4以上の余裕値

futility pruningの余裕値の百分率`f(d)`と、SEEによる捕獲手の枝刈りの余裕値の百分率`s(d)`を、次のように定める。

```math
f(d) = \begin{cases} \mathrm{FutilityMargin}_d & (d \le 3) \\ \mathrm{FutilityMargin}_3 + \mathrm{FutilityDepth} \times (d - 3) & (d \ge 4) \end{cases}
```

```math
s(d) = \begin{cases} \mathrm{SeeMargin}_d & (d \le 3) \\ \mathrm{SeeMargin}_3 + \mathrm{SeeDepth} \times (d - 3) & (d \ge 4) \end{cases}
```

futility pruningは残り深さが`FutilityMaxDepth`以下、late move pruningは`LmpMaxDepth`以下、SEEによる捕獲手の枝刈りは`SeeMaxDepth`以下のノードで行う。
futility pruningのimprovingでないときの倍率は、`d ≤ 3`では深さ別の`NonImprovingFutility`、`d ≥ 4`では`NonImprovingFutility3`とする。
late move pruningで数える静かな手の数の上限式は起点のまま変えず、深さの上限だけを加える。
3つの上限が3のとき、現行と同じになる。

### S2　SEEの余裕値のcapture historyの項

捕獲手のcapture historyの値を`h`（`CaptureHistory::read`の値）として、SEEによる捕獲手の枝刈りの余裕値を次の式にする。

```math
\mathrm{margin}_{\mathrm{SEE}}(d, h) = \max\left(0,\ \frac{P \times s(d)}{100} + \frac{h \times P \times \mathrm{SeeCaptureHistory}/100}{\mathrm{CaptureHistoryLimit}}\right)
```

SEEの値がこの余裕値の符号を反転した値を下回る捕獲手を読まないという判定と、対象の手の範囲は変えない。
本書は現行の`see_prunes`の契約である非負の余裕値で書く。後続の[静止探索の補正と枝刈りの形の見直し](qsearch-forms.md)は、引数を符号付きの閾値`T`へ改め、この呼び出しを`T = −余裕値`へ移す。
`SeeCaptureHistory`が0のとき、現行と同じになる。

### S3　捕獲手のfutility pruning（futility pruning for captures）

ノードの境界値`B(d)`を、静かな手のfutility pruningと同じく次の式で求める。
`e`は起点のfutility pruningが使う静的評価、`c`はcorrection historyの補正、`k(d)`はimprovingでないときの倍率（`NonImprovingFutility`、百分率）であり、improvingのときは100とする。

```math
B(d) = e + c + \frac{P \times f(d) / 100 \times k(d)}{100}
```

捕獲手のcapture historyの値を`h`、取る駒の価値の合計を`V`として、残り深さが`CaptureFutilityMaxDepth`以下のノードで、次の式が成り立つ捕獲手を読まない。

```math
B(d) + V + \frac{h \times P \times \mathrm{CaptureFutilityHistory}/100}{\mathrm{CaptureHistoryLimit}} \le \alpha
```

対象は、置換表の手、成る手、および相手の最後の王駒を取る手を除く捕獲手であり、浅い枝刈りの共通条件を要求する。
判定はSEEによる捕獲手の枝刈りより前に行い、読まなかった手もLMRの表を引く手番号（ノード内で何番目の手か）に数え、capture historyの減点の対象にしない。
この扱いは、現行のSEEによる捕獲手の枝刈りと同じである。
`CaptureFutilityMaxDepth`が0のとき、現行と同じになる。

### S4　静かな手のSEEによる枝刈り

残り深さが`QuietSeeMaxDepth`以下のノードで、SEEの値が`−P × QuietSeeMargin × d²/100`を下回る静かな手を読まない。
対象は、置換表の手、成る手、および2段階移動の手を除く静かな手であり、futility pruningとlate move pruningで読まなかった手には判定しない。
浅い枝刈りの共通条件を要求し、読まなかった手もLMRの表を引く手番号に数え、historyの減点とkiller手の更新の対象にしない。
この扱いは、現行のfutility pruningとlate move pruningと同じである。
SEEは、到達升の駒の価値を0として交換列を作る点を除き、捕獲手のSEEと同じ駒の価値、成り益、および獅子の規則による判定不能の扱いを使う。
余裕値はS2と同じく非負の余裕値として渡し、後続の[静止探索の補正と枝刈りの形の見直し](qsearch-forms.md)で`T = −余裕値`へ移される。
`QuietSeeMaxDepth`が0のとき、現行と同じになる。

### S5　razoringの深さの上限

razoringの余裕値の百分率`r(d)`を、次のように定める。

```math
r(d) = \begin{cases} \mathrm{RazoringMargin}_d & (d \le 2) \\ \mathrm{RazoringMargin}_2 + \mathrm{RazoringDepth} \times (d - 2) & (d \ge 3) \end{cases}
```

razoringは残り深さが`RazoringMaxDepth`以下のノードで行い、静止探索の値がα以下のときだけ返す判定は変えない。
`RazoringMaxDepth`が2のとき、現行と同じになる。

### S6　null move pruningを試す条件の静的評価の余裕

null move pruningを試す現行の条件に、次の条件を加える。
`e`は起点のnull move pruningが減深量の計算に使う静的評価である。

```math
e \ge \beta + \frac{P \times (\mathrm{NullMoveEvalMargin} - \mathrm{NullMoveEvalDepth} \times d - \mathrm{NullMoveEvalImproving} \times [\text{improving}])}{100}
```

どの開始値でも一部のノードでnull moveを試さなくなるので、S6だけは開始値で現行と同じにならない。

### S7　reverse futility pruningの返り値

reverse futility pruningの返り値を、判定に使った静的評価`e`から次の式で求める。

```math
v = e - \frac{(e - \beta) \times \mathrm{ReverseFutilityBetaWeight}}{100}
```

`ReverseFutilityBetaWeight`が0のとき、現行と同じになる。

### 新しい係数

| 係数 | 開始値 | 範囲 | 範囲の根拠 |
|---|---|---|---|
| `FutilityMaxDepth`（手） | 3 | 1〜12 | 静かな手のfutility pruningは、Stockfishが減深後の深さ12未満、YaneuraOuが13未満で働く（Stockfish `src/search.cpp:1247`、YaneuraOu `yaneuraou-search.cpp:3677`）。12は両エンジンの上限の値を残り深さに当てはめたものである。減深後の深さは正のhistoryで増えることもあるので、同じ12で適用されるノードが両エンジンより多いか少ないかは決まらない |
| `FutilityDepth`（深さ1あたりの増分、歩兵価値に対する百分率） | 起点の`(FutilityMargin3 − FutilityMargin1)/2`を丸めた値（起案時の表では59） | −200〜400 | 深さ1あたりの値は、Stockfishが119/208で歩兵価値の約57%、YaneuraOuが120/90で約133%。開始値の式は深さ別の係数の範囲0〜400から−200〜200の値をとるので、範囲の内側に入る |
| `LmpMaxDepth`（手） | 3 | 1〜15 | 両エンジンには上限がない。15は、前の設計書が`ReverseFutilityMaxDepth`に定めた範囲の上限とそろえた。起案時の表では、残り深さ15の上限式は303手である |
| `SeeMaxDepth`（手） | 3 | 1〜15 | 両エンジンには上限がない。15は`LmpMaxDepth`とそろえた |
| `SeeDepth`（深さ1あたりの増分、歩兵価値に対する百分率） | 起点の`(SeeMargin3 − SeeMargin1)/2`を丸めた値（起案時の表では2） | −200〜400 | 深さ1あたりの値は、Stockfishが177/208で約85%、YaneuraOuが167/90で約186% |
| `SeeCaptureHistory`（capture historyが上限のときの余裕値、歩兵価値に対する百分率） | 0 | −200〜800 | capture historyが上限10692のときの項は、Stockfishが10692 × 34/1024/208で約171%、YaneuraOuが同じ量を90で割って約394%。負の値は開始値を内側に置くために含める |
| `CaptureFutilityMaxDepth`（手） | 0 | 0〜10 | Stockfishは減深後の深さ8未満、YaneuraOuは7未満 |
| `CaptureFutilityHistory`（capture historyが上限のときの項、歩兵価値に対する百分率） | 0 | −400〜1600 | 上限のときの項は、Stockfishが10692 × 134/1024/208で約673%、YaneuraOuが10692 × 131/1024/90で約1520%。開始値0は新しい項の重み0の原則による |
| `QuietSeeMaxDepth`（手） | 0 | 0〜12 | 両エンジンには上限がなく、lmrDepthの2乗で余裕値が増える。12は`FutilityMaxDepth`とそろえた |
| `QuietSeeMargin`（深さの2乗あたりの余裕値、歩兵価値に対する百分率） | 20 | 0〜200 | Stockfishは23/208で約11%、YaneuraOuは25/90で約28%。開始値は両者の間に置いた |
| `RazoringMaxDepth`（手） | 2 | 0〜8 | 両エンジンには上限がない。0はrazoringを止める値として含める |
| `RazoringDepth`（深さ1あたりの増分、歩兵価値に対する百分率） | 起点の`RazoringMargin2 − RazoringMargin1`（起案時の表では409） | −1000〜3000 | Stockfishは深さ1あたり342/208で約164%。YaneuraOuは深さの2乗に比例し、深さ2から3への増分は306 × 5/90で約1700%。起点の値が範囲の外にあれば、着手時に範囲を広げて本書を書き換える |
| `NullMoveEvalMargin`（定数項、歩兵価値に対する百分率） | 175 | −1000〜1000 | 開始値はStockfishの365/208を丸めた値。YaneuraOuは378/90で約420%。負の値は、静的評価がβを下回るノードでも試す形である |
| `NullMoveEvalDepth`（深さ1あたりの減算、歩兵価値に対する百分率） | 6 | −50〜100 | 開始値はStockfishの13/208を丸めた値。YaneuraOuは16/90で約18% |
| `NullMoveEvalImproving`（improvingのときの減算、歩兵価値に対する百分率） | 23 | −200〜400 | 開始値はStockfishの47/208を丸めた値。YaneuraOuは53/90で約59% |
| `ReverseFutilityBetaWeight`（βの重み、百分率） | 0 | 0〜100 | Stockfishは約65%、YaneuraOuは約67%。100はβを返す形である |

傾きの開始値の式が起点の表で範囲の外になる場合は、範囲を広げて本書を書き換えてから実装する。

## 所要時間の見積り

時間は、同時対局数16の測定機での見積りである。
SPSAは、同時対局数16で3,000ペアを完走した実績（[SPSAの手引き](../guides/spsa.md)の27,138秒、1ペア約9秒）に基づき、30,000ペアを約75時間（約3日）と見込む。
2026年10月17日に中止した現行の形でのSPSAでは1反復あたり約1分で進んだが、これは暫定の観測である。
段階ゲートの時間は、判定までのペア数が強さの差で変わるので、前の設計書と同じ幅を持たせた。

| 段階 | 内容 | 見積り |
|---|---|---|
| 1 | 7項目の実装と検証（codexへ委任） | 1日程度 |
| 2 | 時間切れの事前確認（13反復、104ペア） | 約15分 |
| 3 | SPSA（30,000ペア） | 約75時間 |
| 4 | 反映、テスト、および事前対局（32ペア） | 約1時間 |
| 5 | 候補のSTC | 約1〜8時間 |
| 6 | 通過した場合のLTC | 約5〜30時間 |

実装の所要時間が前の設計書より長いのは、静かな手のSEEの判定を`see.rs`に加え、深さの条件を枝刈りごとに分ける変更が要るからである。
実装を除く測定機の占有は、表の段階2〜5の和で、STCで棄却される場合に約77〜84時間（約3.2〜3.5日）、段階6まで進む場合に約82〜114時間（約3.4〜4.8日）となる。

## 実装フェーズ

基本シードは、時間切れの事前確認を3500000000、セッションを3550000000、事前対局を3600000000、STCを3650000000、LTCを3700000000と仮に定める。
これらは起案時点で、保存済みの測定とSPSAの基本シード（`data/matches/`と`data/spsa/`の`manifest.json`）、および他の設計書が予約した値から、各測定のペア数以上離れた未使用の値である。
隣接する基本シードは1局ずれた同じ対局列を生むので、この間隔を保つ（[教訓](../lessons/derive-seed-adjacent-collision.md)）。
着手時に同じ方法で未使用であることを確かめ直し、使われていれば設計書を書き換える。
各測定の開始前に、測定機でほかの対局測定、SPSA、教師生成、および学習が動いていないことを実行ファイル名で確かめる。

### フェーズ1　7項目の実装

起点からブランチ`shallow-pruning-extensions`を切り、worktree `data/worktrees/shallow-pruning-extensions`で作業する。
S1、S2、S3、S4、S5、S7、S6の順に、1項目を1コミットにする。
S1は深さの条件を枝刈りごとに分ける土台を含むので最初に置き、S3とS4はS1の深さの条件の上に新しい枝刈りを加える。
S6を除く6項目は開始値で現行と同じ探索になるので、各コミットで`minase dev bench`の全15局面を深さ3、5、7のそれぞれで探索し、探索ノード数、評価値、および最善手が起点と一致することを確かめる。
S6は探索を変えるので最後に置き、同じbenchの探索ノード数を記録して、直前のコミットとの差を残す。
最後のコミットの後、`taskset -c 3`で1つのコアに固定した`minase dev bench --depth 9 --repetitions 3`の中央値を、起点と形の変更で交互に3回ずつ測り、探索速度の変化を記録する（[教訓](../lessons/pin-bench-to-performance-core.md)）。
開始値ではS2のcapture historyの読み出しだけが新しい費用であり、深さの上限が開いたときの費用は対局の結果に含まれる。
新しい係数は`params.rs`の表へ追加し、調整用ビルド（`--features tuning`つきのビルド）が`Tune_<係数名>`のUSIオプションとして宣言することを確かめる。
`pruning.rs`の深さ1〜3に限った関数（`futility_margin`、`see_margin`、`razoring_margin`）は、深さ4以上を`unreachable!`で拒むので、外挿の式へ改める。
静かな手のSEEの判定は、既存の`see_prunes`の参照実装による単体テスト（`see.rs`のテスト）と同じ方法で、benchの局面と固定シードの対局の局面の静かな手について、参照値と照合する。
新しい式の単体テストは、式の写しではなく数値の例で書く（[テストの手引き](../guides/testing.md)）。
完了条件は、`cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`の通過、S6を除く項目のノード数の一致、S6のノード数の変化の記録、および探索速度の記録である。

### フェーズ2　パラメーターファイルと時間切れの事前確認

形の変更の最後のコミットでビルドした`minase`を`data/spsa/shallow-pruning-extensions-runner/`へ複製してrunner（測定を実行するプログラム）として固定し、以後のフェーズはこのrunnerで実行する。
`minase spsa params --engine commit:<形の変更> --rules engine-default`で既定のパラメーターファイルを生成し、`FmScale`の行を削除したファイルを`data/spsa/shallow-pruning-extensions.params`とする。
行数は、起点の調整対象の係数に新しい16係数を加えた62または54である。
開始値と範囲が形の変更の`params.rs`の表と一致すること、および`c_end`と`r_end`が既定値であることを、使い捨てのスクリプトで機械的に突き合わせる。

続いて、手引きの第3節に従い、時間管理の6係数のうち手引きが指定する4係数を思考時間が最も長くなる側の範囲の端に置いても時間切れが起きないことを確かめる。
写しの開始値を`HardRemainingShare`＝50、`HardSoftRatio`＝800、`MinMoves`＝40、`IterationRatio`＝150へ書き換えた`data/spsa/shallow-pruning-extensions-safety.params`で、13反復（104ペア）を実行する。

```console
<固定したminase> spsa \
  --run-dir data/spsa/shallow-pruning-extensions-safety --seed 3500000000 \
  --engine commit:<形の変更> --params data/spsa/shallow-pruning-extensions-safety.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 13 --pairs-per-iteration 8
```

`time_forfeits`（時間切れの件数）が1件でもあれば、[デバッグの手引き](../guides/debugging.md#時間切れと時間管理)の手順で原因を調べ、該当する係数の範囲を狭める案を利用者に示してから本番へ進む。
結果は測定記録`shallow-pruning-extensions-safety`に残す。
完了条件は、時間切れとエンジン異常が0件であることと、測定記録の保存である。

### フェーズ3　SPSAによる調整

```console
<固定したminase> spsa \
  --run-dir data/spsa/shallow-pruning-extensions --seed 3550000000 \
  --engine commit:<形の変更> --params data/spsa/shallow-pruning-extensions.params \
  --rules engine-default --each time=10000+100 --concurrency 16 \
  --iterations 3750 --pairs-per-iteration 8
```

最終のθ（調整中の係数の中心値の組）を整数に丸めた値を調整の結果とし、途中の値を対局の成績で選び直さない。
結果は測定記録`shallow-pruning-extensions`に、手引きの第6節の項目とともに残す。
測定記録には、最終値を丸めた整数が範囲の上限または下限になった係数を明記する。
新しい16係数のうち深さの上限の6係数については、最終値で各枝刈りが働く深さの範囲を示す。
`RazoringMaxDepth`が0、または新設した2つの枝刈りの深さの上限が0のままであれば、その項目は候補で働かないことを明記する。
これらは、どの拡張が使われたかを読むための目安であり、採否には使わない。
完了条件は、セッションの完了と測定記録の保存である。

### フェーズ4　反映と事前対局

手引きの第7節の手順1に従い、`minase spsa apply`で最終値を表の既定値へ書き込み、`git diff`で既定値だけが変わったことを確かめてコミットする。
続いて`cargo test`（通常ビルドと調整用ビルド）を測定より先に実行し、表の既定値に依存するテストが失敗した場合は、テストの前提が新しい既定値でも成り立つようにテストを直して別のコミットにする。
テストの失敗を、末尾に`; true`を付けるようなコマンドの組み方で握りつぶさない。
候補の調整用ビルドの宣言を`minase spsa params`で出力し、既定値の欄が`apply`の出力と一致することを確かめる。

採否測定の前に、候補が時間切れやエンジン異常を起こさないことと、時間の使い方の変化を確かめるため、STCの条件で候補と起点の32ペアの事前対局を行う。

```console
<固定したminase> match run \
  --run-dir data/matches/shallow-pruning-extensions-smoke --seed 3600000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 elo --pairs 32
```

候補と起点の総思考時間の比、終局時の残り時間、手数帯ごとの完了深さ（反復深化で読み終えた深さ）、および探索を止めた理由の分布を、前の設計書の事前対局と同じ方法で集計して、測定記録`shallow-pruning-extensions-smoke`に残す。
深さの上限が開くと浅い枝刈りの計算が増えるので、同じ記録に、候補と起点の探索速度を前述のbenchで測った値も残す。
完了条件は、テストの通過、事前対局での時間切れとエンジン異常が0件であること、および測定記録の保存である。

### フェーズ5　採否測定

```console
<固定したminase> match run \
  --run-dir data/matches/shallow-pruning-extensions-stc --seed 3650000000 \
  --candidate commit:<候補> --baseline commit:<起点> \
  --each time=10000+100 --concurrency 16 gsprt --max-pairs 3000
```

STCの判定は[SPRTの手引き](../guides/sprt.md)の振分け規則に従う。
候補が有意に強い`H1`ならLTCへ進み、改善を示せない`H0`なら不採用とし、3,000ペアで判定保留ならその時点の対数尤度比の符号でLTCへ進むかを決める。
LTCは測定名の接尾辞を`-ltc`、基本シードを3700000000、持ち時間を`time=60000+200`とし、`--max-pairs`を付けない。
LTCで`H1`となり、エンジン異常、時間切れ、および拒否着手が0件なら採用し、`H0`なら不採用とする。
LTCの判定保留は、手引きのとおり`--resume`と`--target-pairs`で延長する。
結果は測定名ごとの測定記録に残す。
完了条件は、採否の確定と測定記録の保存である。

## 検証

- S6を除く各コミットで、`minase dev bench`の全15局面の深さ3、5、7の探索ノード数、評価値、および最善手が起点と一致すること。S6のコミットでは、同じbenchの探索ノード数の変化が記録されていること。
- 静かな手のSEEの判定が、benchの局面と固定シードの対局の局面の静かな手について、参照実装の値と一致すること。
- 新しい16係数が調整用ビルドの`Tune_`オプションとして宣言され、パラメーターファイルの開始値と範囲が表と一致すること。
- 候補の`params.rs`の差分が既定値の書き換えだけであり、調整用ビルドの宣言の既定値が`apply`の出力と一致すること。
- `cargo test`（通常ビルドと調整用ビルド）、`cargo clippy -- -D warnings`、および`cargo fmt --check`が通過すること。
- 時間切れの事前確認、SPSA、事前対局、および採否の測定で、エンジン異常、時間切れ、および拒否着手が0件であること。最初の測定で1件でも出た場合は、測定を止めて原因を調べる。

## 完了条件

- 形の変更の検証結果、時間切れの事前確認、SPSAの結果、事前対局、および採否が、測定記録に残っている。
- 採用した場合は、候補のブランチがmasterへ統合され、依存関係の節に挙げたADRが新しいADRに置き換えられている。統合は利用者の許可を得てから行う。
- 不採用の場合は、結論と測定記録だけをmasterへ残し、候補のブランチはmasterへ統合しない。
- ROADMAPの状態表が本書の結論と一致している。

## 参考資料

- [探索と時間管理の係数の関数形に関する既存実装との比較](../research/search-coefficient-forms-prior-art.md)と[前向き枝刈りの第3層に関する既存実装の調査](../research/forward-pruning-prior-art.md)。
- Stockfish（コミット5c4cef0b）の`src/search.cpp`。razoringは1017〜1018行、reverse futility pruningは1021〜1033行、null move pruningを試す条件は1037〜1039行、late move pruningは1199行、lmrDepthは1203行、捕獲手のfutility pruningは1208〜1217行、捕獲手のSEEは1222〜1225行、静かな手のfutility pruningは1242〜1252行、静かな手のSEEは1255〜1259行。capture historyの上限10692は`src/history.h:135`、歩兵価値208は`src/types.h:199`。
- YaneuraOu（コミットc1b80eaa）の`source/engine/yaneuraou-engine/yaneuraou-search.cpp`。razoringは3191〜3192行、reverse futility pruningは3214〜3227行、null move pruningを試す条件は3236行、late move pruningは3593行、lmrDepthは3600行、捕獲手のfutility pruningは3611〜3621行、捕獲手のSEEは3631〜3641行、静かな手のfutility pruningは3663〜3677行、静かな手のSEEは3684〜3697行。capture historyの上限は`source/history.h:244`、歩兵価値90は`source/evaluate.h:78`。
- [不採用だった探索部の改良の再調整](archived/search-revival-spsa.md)と[一括採用のADR](../adr/adopt-revived-search-items-as-a-bundle.md)。
- [SPSAによる係数調整の手引き](../guides/spsa.md)と[SPRTによる棋力測定の手引き](../guides/sprt.md)。
